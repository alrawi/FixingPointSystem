// SPDX-License-Identifier: GPL-3.0-only
//! The frames that the scenarios write, run through the `h2` crate, which is an
//! independent implementation of RFC 9113 and RFC 7541 and is already in
//! `Cargo.lock` through the HTTP stack of the engine.
//!
//! The test runs a scenario with both sides over the in memory duplex and takes
//! the plaintext of each direction. It replays what the client wrote into an
//! `h2` server, which must accept every request with the right method, path,
//! and body size, and it replays what the server wrote into an `h2` client that
//! has opened the same streams, which must receive every response with the
//! right status and body size. The `h2` crate checks the frame layout, the
//! HPACK blocks, the stream states, and the flow control windows on the way.

mod common;

use std::time::Duration;

use bytes::Bytes;
use common::frames::{self, field, header_lists, split, split_client};
use common::{run, scenarios, scenarios_with, tls_small};
use tokio::io::{duplex, split as split_io, AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(30);

/// What the frames of the scenario say about each stream, from the reader that
/// does not use the crate under test.
struct Expect {
    /// Stream id, method, path, request body size.
    requests: Vec<(u32, String, String, usize)>,
    /// Stream id, status, response body size.
    responses: Vec<(u32, u16, usize)>,
}

fn expectations(c2s: &[u8], s2c: &[u8]) -> Expect {
    let client = split_client(c2s);
    let server = split(s2c);
    let data = |frames: &[frames::Frame], stream: u32| -> usize {
        frames
            .iter()
            .filter(|f| f.ty == frames::DATA && f.stream == stream)
            .map(|f| f.payload.len())
            .sum()
    };
    let requests = header_lists(&client)
        .into_iter()
        .map(|(id, list)| {
            (
                id,
                field(&list, ":method").expect("method").to_owned(),
                field(&list, ":path").expect("path").to_owned(),
                data(&client, id),
            )
        })
        .collect();
    let responses = header_lists(&server)
        .into_iter()
        .map(|(id, list)| {
            let status = field(&list, ":status")
                .expect("status")
                .parse()
                .expect("number");
            (id, status, data(&server, id))
        })
        .collect();
    Expect {
        requests,
        responses,
    }
}

/// Replays the bytes of a client into an `h2` server and returns what it saw,
/// as method, path, and body size, in the order of acceptance.
///
/// The replay has two parts. The first ends with the SETTINGS ACK of the
/// client. The server then gets time to send its own SETTINGS and the
/// WINDOW_UPDATE for its connection window, as it does before a real client
/// sends a body, and the second part follows. A replay of both parts at once
/// would send data into the default window of 65535 octets.
async fn replay_into_server(c2s: &[frames::Frame], count: usize) -> Vec<(String, String, usize)> {
    let at = c2s
        .iter()
        .position(|f| f.ty == frames::SETTINGS && f.has(frames::ACK))
        .expect("the client acknowledges the SETTINGS of the server");
    let mut first = frames::PREFACE.to_vec();
    first.extend(frames::join(&c2s[..=at]));
    let second = frames::join(&c2s[at + 1..]);

    let (mine, theirs) = duplex(1 << 22);
    let (mut from_server, mut to_server) = split_io(mine);
    // What the server writes goes nowhere, but the pipe must not fill up.
    let drain = tokio::spawn(async move {
        let mut sink = vec![0u8; 16 * 1024];
        while matches!(from_server.read(&mut sink).await, Ok(n) if n > 0) {}
    });
    // The server handshake reads the client preface, so the bytes go in first.
    to_server.write_all(&first).await.expect("replay");
    let mut server = h2::server::Builder::new()
        .initial_window_size(8 << 20)
        .initial_connection_window_size(16 << 20)
        .handshake::<_, Bytes>(theirs)
        .await
        .expect("handshake");
    // One task owns the connection and keeps it running, and hands each
    // request to the test.
    let (requests, mut inbox) = tokio::sync::mpsc::unbounded_channel();
    let driver = tokio::spawn(async move {
        while let Some(accepted) = server.accept().await {
            if requests.send(accepted).is_err() {
                break;
            }
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    to_server.write_all(&second).await.expect("replay");
    let mut tasks = Vec::new();
    for _ in 0..count {
        let (request, mut respond) = timeout(WAIT, inbox.recv())
            .await
            .expect("a request arrives")
            .expect("the connection stays open")
            .expect("a valid request");
        tasks.push(tokio::spawn(async move {
            let (parts, mut body) = request.into_parts();
            let mut size = 0;
            while let Some(chunk) = body.data().await {
                let chunk = chunk.expect("body data");
                size += chunk.len();
                body.flow_control()
                    .release_capacity(chunk.len())
                    .expect("release");
            }
            let response = http::Response::builder().status(200).body(()).expect("ok");
            // The connection may be gone by now, which does not matter here.
            let _ = respond.send_response(response, true);
            (parts.method.to_string(), parts.uri.path().to_owned(), size)
        }));
    }
    let mut seen = Vec::new();
    for t in tasks {
        seen.push(timeout(WAIT, t).await.expect("body arrives").expect("task"));
    }
    driver.abort();
    drain.abort();
    seen
}

/// Counts the complete HEADERS frames in what a client wrote, which starts
/// with the connection preface.
fn headers_in(mut bytes: &[u8]) -> usize {
    if bytes.len() < frames::PREFACE.len() {
        return 0;
    }
    bytes = &bytes[frames::PREFACE.len()..];
    let mut count = 0;
    while bytes.len() >= 9 {
        let length = u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]) as usize;
        if bytes.len() < 9 + length {
            break;
        }
        count += usize::from(bytes[3] == frames::HEADERS);
        bytes = &bytes[9 + length..];
    }
    count
}

/// Replays the bytes of a server into an `h2` client that sent one request per
/// stream, and returns the status and body size of each response in stream order.
async fn replay_into_client(
    s2c: Vec<u8>,
    streams: &[(u32, String, String, usize)],
) -> Vec<(u16, usize)> {
    let (mine, theirs) = duplex(1 << 22);
    let (mut from_client, mut to_client) = split_io(mine);
    let (mut sender, connection) = h2::client::Builder::new()
        .initial_window_size(8 << 20)
        .initial_connection_window_size(16 << 20)
        .enable_push(false)
        .handshake::<_, Bytes>(theirs)
        .await
        .expect("handshake");
    let driver = tokio::spawn(connection);
    let mut responses = Vec::new();
    for (_, method, path, body) in streams {
        let request = http::Request::builder()
            .method(method.as_str())
            .uri(format!("http://www.example.test{path}"))
            .body(())
            .expect("request");
        sender = sender.ready().await.expect("ready");
        let (response, mut stream) = sender
            .send_request(request, *body == 0 && method == "GET")
            .expect("send");
        if !(*body == 0 && method == "GET") {
            stream.send_data(Bytes::new(), true).expect("finish");
        }
        responses.push(response);
    }
    // The client opens a stream when its HEADERS frame leaves, and a response
    // for a stream that is not open yet is a protocol error, so the replay
    // waits until every request is on the wire.
    let mut written = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    while headers_in(&written) < streams.len() {
        let n = timeout(WAIT, from_client.read(&mut chunk))
            .await
            .expect("the client writes its requests")
            .expect("read");
        assert!(n > 0, "the client closed before it sent its requests");
        written.extend_from_slice(&chunk[..n]);
    }
    let drain = tokio::spawn(async move {
        let mut sink = vec![0u8; 16 * 1024];
        while matches!(from_client.read(&mut sink).await, Ok(n) if n > 0) {}
    });
    to_client.write_all(&s2c).await.expect("replay");
    let mut seen = Vec::new();
    for response in responses {
        let response = timeout(WAIT, response)
            .await
            .expect("a response arrives")
            .expect("a valid response");
        let status = response.status().as_u16();
        let mut body = response.into_body();
        let mut size = 0;
        while let Some(chunk) = timeout(WAIT, body.data()).await.expect("body arrives") {
            let chunk = chunk.expect("body data");
            size += chunk.len();
            body.flow_control()
                .release_capacity(chunk.len())
                .expect("release");
        }
        seen.push((status, size));
    }
    driver.abort();
    drain.abort();
    seen
}

#[tokio::test(flavor = "current_thread")]
async fn the_h2_crate_accepts_both_directions_of_every_scenario() {
    let cleartext = scenarios();
    let tls = scenarios_with(&tls_small());
    for ((name, plain), (_, small)) in cleartext.iter().zip(&tls) {
        let s = if name.starts_with("https_") {
            small
        } else {
            plain
        };
        let r = run(s, 42, 1).await;
        assert!(r.client.is_ok(), "{name}: {:?}", r.client);
        assert!(r.server.is_ok(), "{name}: {:?}", r.server);
        let expected = expectations(&r.c2s, &r.s2c);
        assert!(expected.requests.len() >= 2, "{name}");
        assert_eq!(expected.requests.len(), expected.responses.len(), "{name}");

        // The peers of the replay never sent a PING, so the acknowledgments go.
        let c2s = frames::without_ping_acks(&split_client(&r.c2s));
        let s2c = frames::join(&frames::without_ping_acks(&split(&r.s2c)));
        let seen = replay_into_server(&c2s, expected.requests.len()).await;
        let want: Vec<(String, String, usize)> = expected
            .requests
            .iter()
            .map(|(_, m, p, n)| (m.clone(), p.clone(), *n))
            .collect();
        // The h2 server hands the requests over in stream order, and a task
        // finishes when its body ends, so compare as sets.
        let (mut seen, mut want) = (seen, want);
        seen.sort();
        want.sort();
        assert_eq!(seen, want, "{name}: the h2 server saw other requests");

        let got = replay_into_client(s2c, &expected.requests).await;
        let want: Vec<(u16, usize)> = expected
            .responses
            .iter()
            .map(|(_, s, n)| (*s, *n))
            .collect();
        assert_eq!(got, want, "{name}: the h2 client saw other responses");
    }
}
