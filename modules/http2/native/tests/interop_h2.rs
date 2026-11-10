// SPDX-License-Identifier: GPL-3.0-only
//! Interoperability with the `h2` crate, which the HTTP stack of the engine
//! already depends on, as an independent implementation of RFC 9113 and
//! RFC 7541. The frames of this crate must be accepted by an `h2` server and
//! decode to the request that they describe, and the frames of an `h2` client
//! must parse and decode with this crate.

use std::time::Duration;

use bytes::Bytes;
use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs, FnCall, TemplateFn};
use fps_module_http2::frame::{self, flag, ty};
use fps_module_http2::hpack::Decoder;
use fps_module_http2::Http2;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

fn call(
    h: &mut Http2,
    function: &'static str,
    positional: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Vec<u8> {
    let mut rng = rand_pcg::Pcg64Mcg::new(7);
    let mut out = Vec::new();
    let c = FnCall {
        module: "http2",
        function,
        args: CallArgs { positional, named },
    };
    h.render(&c, &mut rng, &mut out).unwrap();
    out
}

fn named<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
    NamedArg { name, value }
}

const WAIT: Duration = Duration::from_secs(20);

/// A request as the `h2` server saw it.
#[derive(Debug, PartialEq)]
struct Seen {
    method: String,
    uri: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[tokio::test(flavor = "current_thread")]
async fn an_h2_server_accepts_the_frames_of_this_crate() {
    let (client, server_io) = duplex(1 << 20);

    let server = tokio::spawn(async move {
        let mut conn = h2::server::handshake(server_io).await.unwrap();
        let mut tasks = Vec::new();
        for _ in 0..3 {
            let (request, mut respond) = conn.accept().await.unwrap().unwrap();
            tasks.push(tokio::spawn(async move {
                let (parts, mut body) = request.into_parts();
                let mut data = Vec::new();
                while let Some(chunk) = body.data().await {
                    let chunk = chunk.unwrap();
                    data.extend_from_slice(&chunk);
                    body.flow_control().release_capacity(chunk.len()).unwrap();
                }
                let response = http::Response::builder().status(200).body(()).unwrap();
                respond.send_response(response, true).unwrap();
                Seen {
                    method: parts.method.to_string(),
                    uri: parts.uri.to_string(),
                    headers: parts
                        .headers
                        .iter()
                        .map(|(n, v)| (n.to_string(), v.to_str().unwrap().to_string()))
                        .collect(),
                    body: data,
                }
            }));
        }
        // The connection makes progress only while somebody polls it.
        let drive = async { while conn.accept().await.is_some() {} };
        let collect = async {
            let mut seen = Vec::new();
            for t in tasks {
                seen.push(t.await.unwrap());
            }
            seen
        };
        tokio::select! {
            seen = collect => seen,
            () = drive => panic!("the connection ended before the requests finished"),
        }
    });

    let mut h = Http2::default();
    let body: Vec<u8> = (0..=255).cycle().take(40_000).collect();
    let end = [named("end_stream", ArgValue::Boolean(true))];
    let mut wire = Vec::new();
    wire.extend(call(&mut h, "preface", &[], &[]));
    wire.extend(call(
        &mut h,
        "settings",
        &[],
        &[
            named("max_concurrent_streams", ArgValue::Int(10)),
            named("initial_window_size", ArgValue::Int(1 << 20)),
        ],
    ));
    wire.extend(call(&mut h, "settings_ack", &[], &[]));
    // A GET without a body, and a second one that uses the dynamic table.
    let get = |path: &str| {
        format!(
            ":method: GET\n:scheme: https\n:path: {path}\n:authority: www.example.com\nuser-agent: fps-test/1.0\naccept: text/html\nx-trace: abc\n"
        )
    };
    for (stream, path) in [(1, "/"), (3, "/index.html")] {
        let list = get(path);
        wire.extend(call(
            &mut h,
            "headers",
            &[ArgValue::Int(stream), ArgValue::Bytes(list.as_bytes())],
            &end,
        ));
    }
    // A POST with a body that spans several DATA frames, a padded one, and
    // a header block that continues in a CONTINUATION frame.
    let mut post = String::from(
        ":method: POST\n:scheme: https\n:path: /upload\n:authority: www.example.com\ncontent-type: application/octet-stream\ncookie: session=abc123\n",
    );
    for i in 0..8 {
        post.push_str(&format!("x-long-{i}: {}\n", "z".repeat(300)));
    }
    wire.extend(call(
        &mut h,
        "headers",
        &[ArgValue::Int(5), ArgValue::Bytes(post.as_bytes())],
        &[
            named("max_frame_size", ArgValue::Int(1000)),
            named("padding", ArgValue::Int(5)),
            named("priority_dependency", ArgValue::Int(0)),
        ],
    ));
    wire.extend(call(
        &mut h,
        "data",
        &[ArgValue::Int(5), ArgValue::Bytes(&body)],
        &[
            named("end_stream", ArgValue::Boolean(true)),
            named("padding", ArgValue::Int(9)),
            named("max_frame_size", ArgValue::Int(16384)),
        ],
    ));
    wire.extend(call(&mut h, "ping", &[], &[]));
    wire.extend(call(
        &mut h,
        "window_update",
        &[ArgValue::Int(0), ArgValue::Int(1000)],
        &[],
    ));

    let (mut read, mut write) = tokio::io::split(client);
    // Drain what the server writes, so it never blocks on a full pipe.
    let drain = tokio::spawn(async move {
        let mut sink = Vec::new();
        let _ = read.read_to_end(&mut sink).await;
        sink
    });
    write.write_all(&wire).await.unwrap();
    let seen = timeout(WAIT, server).await.unwrap().unwrap();
    drain.abort();

    assert_eq!(seen.len(), 3, "{seen:?}");
    let header = |s: &Seen, name: &str| {
        s.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    for (s, path) in seen[..2].iter().zip(["/", "/index.html"]) {
        assert_eq!(s.method, "GET");
        assert_eq!(s.uri, format!("https://www.example.com{path}"));
        assert_eq!(header(s, "user-agent").as_deref(), Some("fps-test/1.0"));
        assert_eq!(header(s, "accept").as_deref(), Some("text/html"));
        assert_eq!(header(s, "x-trace").as_deref(), Some("abc"));
        assert!(s.body.is_empty());
    }
    let p = &seen[2];
    assert_eq!(
        (p.method.as_str(), p.uri.as_str()),
        ("POST", "https://www.example.com/upload")
    );
    assert_eq!(header(p, "cookie").as_deref(), Some("session=abc123"));
    assert_eq!(
        header(p, "x-long-7").as_deref(),
        Some("z".repeat(300).as_str())
    );
    assert_eq!(p.body, body);
}

/// Reads frames from `io` until `done` says that the last one that matters
/// arrived, and returns every frame as header and payload.
async fn read_frames(
    io: &mut (impl AsyncReadExt + Unpin),
    mut done: impl FnMut(&frame::FrameHeader) -> bool,
) -> Vec<(frame::FrameHeader, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut frames = Vec::new();
    let mut preface = true;
    loop {
        if preface && buf.len() >= 24 {
            assert_eq!(&buf[..24], frame::PREFACE);
            buf.drain(..24);
            preface = false;
        }
        while !preface && !buf.is_empty() {
            let Ok((f, rest)) = frame::split(&buf) else {
                break;
            };
            let stop = done(&f.header);
            let consumed = buf.len() - rest.len();
            frames.push((f.header, f.payload.to_vec()));
            buf.drain(..consumed);
            if stop {
                return frames;
            }
        }
        let mut chunk = [0u8; 4096];
        let n = timeout(WAIT, io.read(&mut chunk)).await.unwrap().unwrap();
        assert!(n > 0, "the client closed before the last frame");
        buf.extend_from_slice(&chunk[..n]);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn this_crate_parses_and_decodes_the_frames_of_an_h2_client() {
    let (client_io, mut server_io) = duplex(1 << 20);
    let body: Vec<u8> = (0..=255).rev().cycle().take(30_000).collect();
    let sent = body.clone();
    let client = tokio::spawn(async move {
        let (send, conn) = h2::client::handshake(client_io).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        // The handles stay alive, because dropping a stream resets it.
        let mut held = Vec::new();
        for (path, with_body) in [
            ("/first", false),
            ("/second?a=1&b=2", true),
            ("/first", false),
        ] {
            let request = http::Request::builder()
                .method(if with_body { "POST" } else { "GET" })
                .uri(format!("https://www.example.com{path}"))
                .header("user-agent", "h2-test/1.0")
                .header("accept-encoding", "gzip, deflate")
                .header("x-custom", "some value")
                .header("authorization", "Bearer token-123")
                .body(())
                .unwrap();
            let mut ready = send.clone().ready().await.unwrap();
            let (response, mut stream) = ready.send_request(request, !with_body).unwrap();
            if with_body {
                let mut done = 0;
                while done < sent.len() {
                    stream.reserve_capacity(sent.len() - done);
                    let room = std::future::poll_fn(|cx| stream.poll_capacity(cx))
                        .await
                        .unwrap()
                        .unwrap();
                    let n = room.min(sent.len() - done);
                    let last = done + n == sent.len();
                    stream
                        .send_data(Bytes::copy_from_slice(&sent[done..done + n]), last)
                        .unwrap();
                    done += n;
                }
            }
            held.push((response, stream));
        }
        // Keep the connection open until the test has read the frames.
        tokio::time::sleep(WAIT).await;
    });

    // The server side of the test reads raw frames until both bodyless
    // requests and the body of the POST have ended.
    let (mut bodyless, mut body_ended) = (0, false);
    let frames = read_frames(&mut server_io, |h| {
        bodyless += usize::from(h.ty == ty::HEADERS && h.has(flag::END_STREAM));
        body_ended |= h.ty == ty::DATA && h.has(flag::END_STREAM);
        bodyless == 2 && body_ended
    })
    .await;
    client.abort();

    let mut decoder = Decoder::new();
    let mut requests: Vec<Vec<(String, String)>> = Vec::new();
    let mut got_body = Vec::new();
    let mut block: Vec<u8> = Vec::new();
    for (h, payload) in &frames {
        match h.ty {
            ty::HEADERS | ty::CONTINUATION => {
                block.extend_from_slice(frame::block_fragment(h.ty, h.flags, payload).unwrap());
                if h.has(flag::END_HEADERS) {
                    let fields = decoder.decode(&block).unwrap();
                    block.clear();
                    requests.push(
                        fields
                            .into_iter()
                            .map(|f| {
                                (
                                    String::from_utf8(f.name).unwrap(),
                                    String::from_utf8(f.value).unwrap(),
                                )
                            })
                            .collect(),
                    );
                }
            }
            ty::DATA => got_body.extend_from_slice(frame::strip_padding(h.flags, payload).unwrap()),
            ty::SETTINGS => {
                assert!(frame::settings_pairs(payload).is_ok());
            }
            _ => {}
        }
    }
    assert_eq!(requests.len(), 3, "{requests:?}");
    let field =
        |r: &[(String, String)], n: &str| r.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
    for (r, (method, path)) in requests.iter().zip([
        ("GET", "/first"),
        ("POST", "/second?a=1&b=2"),
        ("GET", "/first"),
    ]) {
        assert_eq!(field(r, ":method").as_deref(), Some(method));
        assert_eq!(field(r, ":path").as_deref(), Some(path));
        assert_eq!(field(r, ":scheme").as_deref(), Some("https"));
        assert_eq!(field(r, ":authority").as_deref(), Some("www.example.com"));
        assert_eq!(field(r, "x-custom").as_deref(), Some("some value"));
        assert_eq!(
            field(r, "authorization").as_deref(),
            Some("Bearer token-123")
        );
    }
    assert_eq!(got_body, body);
    // The h2 encoder indexes between requests, so the decoder holds entries.
    assert!(!decoder.is_empty());
}
