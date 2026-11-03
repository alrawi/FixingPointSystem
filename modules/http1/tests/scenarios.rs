// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests: every http1 scenario runs with its client and server
//! against each other over the in memory duplex, and the bytes in each
//! direction parse as the HTTP messages that the scenario describes.

mod common;

use common::{run, scenarios};

/// One parsed HTTP/1 message head and the length of its body.
#[derive(Debug)]
struct Message {
    start: String,
    headers: Vec<(String, String)>,
    body_len: usize,
    chunked: bool,
}

impl Message {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Splits a byte stream into HTTP/1 messages with the framing rules of RFC
/// 9112 section 6. `responses` selects the rule that a 1xx, 204, or 304
/// response has no body.
fn parse_stream(mut data: &[u8], responses: bool) -> Vec<Message> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let end = find(data, b"\r\n\r\n").expect("header block ends");
        let head = std::str::from_utf8(&data[..end]).expect("head is UTF-8");
        let mut lines = head.split("\r\n");
        let start = lines.next().unwrap().to_string();
        let headers: Vec<(String, String)> = lines
            .map(|l| {
                let (n, v) = l.split_once(": ").expect("field line");
                (n.to_string(), v.to_string())
            })
            .collect();
        data = &data[end + 4..];
        let mut m = Message {
            start,
            headers,
            body_len: 0,
            chunked: false,
        };
        let status: Option<u16> =
            responses.then(|| m.start.split(' ').nth(1).unwrap().parse().unwrap());
        let bodiless = status.is_some_and(|s| s < 200 || s == 204 || s == 304);
        if bodiless {
        } else if m.header("Transfer-Encoding") == Some("chunked") {
            m.chunked = true;
            loop {
                let line_end = find(data, b"\r\n").unwrap();
                let size =
                    usize::from_str_radix(std::str::from_utf8(&data[..line_end]).unwrap(), 16)
                        .unwrap();
                data = &data[line_end + 2..];
                if size == 0 {
                    assert_eq!(&data[..2], b"\r\n", "empty trailer section");
                    data = &data[2..];
                    break;
                }
                assert_eq!(&data[size..size + 2], b"\r\n", "chunk ends with CRLF");
                m.body_len += size;
                data = &data[size + 2..];
            }
        } else if let Some(len) = m.header("Content-Length") {
            m.body_len = len.parse().unwrap();
            data = &data[m.body_len..];
        } else {
            assert!(!responses, "a response without framing");
        }
        out.push(m);
    }
    out
}

/// The expected exchange of each scenario: the request line prefix of every
/// request, the status of every response, and the framing of the bodies.
struct Expect {
    version: &'static str,
    requests: usize,
    request_method: &'static str,
    status: u16,
    chunked_requests: bool,
    chunked_responses: usize,
}

fn expected(name: &str) -> Expect {
    match name {
        "single_fetch" => Expect {
            version: "HTTP/1.1",
            requests: 1,
            request_method: "GET",
            status: 200,
            chunked_requests: false,
            chunked_responses: 0,
        },
        "keepalive_browse" => Expect {
            version: "HTTP/1.1",
            requests: 7,
            request_method: "GET",
            status: 200,
            chunked_requests: false,
            chunked_responses: 1,
        },
        "upload" => Expect {
            version: "HTTP/1.1",
            requests: 2,
            request_method: "POST",
            status: 201,
            chunked_requests: false,
            chunked_responses: 0,
        },
        "upload_chunked" => Expect {
            version: "HTTP/1.1",
            requests: 1,
            request_method: "PUT",
            status: 201,
            chunked_requests: true,
            chunked_responses: 0,
        },
        "http10_fetch" => Expect {
            version: "HTTP/1.0",
            requests: 1,
            request_method: "GET",
            status: 200,
            chunked_requests: false,
            chunked_responses: 0,
        },
        "http10_keepalive" => Expect {
            version: "HTTP/1.0",
            requests: 3,
            request_method: "GET",
            status: 200,
            chunked_requests: false,
            chunked_responses: 0,
        },
        other => panic!("no expectation for scenario {other}"),
    }
}

#[test]
fn the_manifest_lists_six_scenarios() {
    let names: Vec<String> = scenarios().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "single_fetch",
            "keepalive_browse",
            "upload",
            "upload_chunked",
            "http10_fetch",
            "http10_keepalive"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn every_scenario_succeeds_with_both_sides() {
    for (name, s) in scenarios() {
        for session_id in 1..=5 {
            let r = run(&s, 42, session_id).await;
            assert!(
                r.client.is_ok(),
                "{name} client, session {session_id}: {:?}",
                r.client
            );
            assert!(
                r.server.is_ok(),
                "{name} server, session {session_id}: {:?}",
                r.server
            );
            assert!(r.client_stats.established && r.server_stats.established);
            assert_eq!(r.client_stats.bytes_tx, r.server_stats.bytes_rx, "{name}");
            assert_eq!(r.server_stats.bytes_tx, r.client_stats.bytes_rx, "{name}");
            assert_eq!(r.client_stats.bytes_tx, r.c2s.len() as u64, "{name}");
            assert_eq!(r.server_stats.bytes_tx, r.s2c.len() as u64, "{name}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn every_scenario_exchanges_well_framed_messages() {
    for (name, s) in scenarios() {
        let e = expected(&name);
        let r = run(&s, 42, 1).await;
        let requests = parse_stream(&r.c2s, false);
        let responses = parse_stream(&r.s2c, true);
        assert_eq!(requests.len(), e.requests, "{name} requests");
        assert_eq!(responses.len(), e.requests, "{name} responses");
        for q in &requests {
            assert!(q.start.starts_with(e.request_method), "{name}: {}", q.start);
            assert!(q.start.ends_with(e.version), "{name}: {}", q.start);
            assert_eq!(q.header("Host"), Some("www.example.test"), "{name}");
            assert_eq!(q.chunked, e.chunked_requests, "{name}");
        }
        for p in &responses {
            assert!(
                p.start.starts_with(&format!("{} {}", e.version, e.status)),
                "{name}: {}",
                p.start
            );
        }
        let chunked = responses.iter().filter(|p| p.chunked).count();
        assert_eq!(chunked, e.chunked_responses, "{name} chunked responses");
    }
}

#[tokio::test(start_paused = true)]
async fn bodies_have_the_configured_sizes() {
    let all = scenarios();
    let get = |n: &str| &all.iter().find(|(name, _)| name == n).unwrap().1;

    let r = run(get("keepalive_browse"), 42, 1).await;
    let responses = parse_stream(&r.s2c, true);
    assert_eq!(responses[0].body_len, 18000);
    assert!(responses[1..].iter().all(|p| p.body_len == 12000));
    let requests = parse_stream(&r.c2s, false);
    assert!(requests[1..]
        .iter()
        .all(|q| q.start.starts_with("GET /static/img") && q.start.contains(".png ")));

    let r = run(get("upload"), 42, 1).await;
    let requests = parse_stream(&r.c2s, false);
    assert!(requests.iter().all(|q| q.body_len == 65536));
    assert!(requests
        .iter()
        .all(|q| q.header("Content-Type") == Some("application/octet-stream")));

    let r = run(get("upload_chunked"), 42, 1).await;
    let requests = parse_stream(&r.c2s, false);
    assert_eq!(requests[0].body_len, 65536);
    assert_eq!(requests[0].header("Content-Length"), None);
}

#[tokio::test(start_paused = true)]
async fn keepalive_and_close_headers_match_the_versions() {
    for (name, s) in scenarios() {
        let r = run(&s, 42, 1).await;
        let last = parse_stream(&r.s2c, true).pop().unwrap();
        let connection = last.header("Connection");
        match name.as_str() {
            "single_fetch" | "upload_chunked" => assert_eq!(connection, Some("close")),
            "http10_keepalive" => assert_eq!(connection, Some("keep-alive")),
            _ => assert_eq!(connection, None, "{name}"),
        }
    }
}
