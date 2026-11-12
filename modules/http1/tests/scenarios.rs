// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests: every http1 scenario runs with its client and server
//! against each other over the in memory duplex, and the bytes in each
//! direction parse as the HTTP messages that the scenario describes. The https
//! scenarios run over TLS, and their plaintext is what the tests parse, see
//! `https.rs` for the checks of the TLS layer.

mod common;

use common::http::parse_stream;
use common::{run, scenarios};

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
        "https_fetch" | "https_fetch_tls12" => expected("single_fetch"),
        "https_keepalive_browse" | "https_keepalive_browse_tls12" => expected("keepalive_browse"),
        other => panic!("no expectation for scenario {other}"),
    }
}

#[test]
fn the_manifest_lists_ten_scenarios() {
    let names: Vec<String> = scenarios().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "single_fetch",
            "keepalive_browse",
            "upload",
            "upload_chunked",
            "http10_fetch",
            "http10_keepalive",
            "https_fetch",
            "https_keepalive_browse",
            "https_fetch_tls12",
            "https_keepalive_browse_tls12"
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
            "single_fetch" | "upload_chunked" | "https_fetch" | "https_fetch_tls12" => {
                assert_eq!(connection, Some("close"));
            }
            "http10_keepalive" => assert_eq!(connection, Some("keep-alive")),
            _ => assert_eq!(connection, None, "{name}"),
        }
    }
}

/// The sessions that the estimate averages over. Sizes that the session
/// generator draws vary from session to session, so one session says little
/// about the mean that the controller needs.
const ESTIMATE_SESSIONS: u64 = 20;

/// A session of each scenario at the default parameters moves the payload
/// bytes of `expected_bytes` in the manifest, both directions of the stream
/// above any TLS layer, within 2 percent. A scenario whose size depends on the
/// session generator is averaged over [`ESTIMATE_SESSIONS`] session ids. The
/// controller turns byte weights into session shares with these estimates, so a
/// change of a template or a fixture that alters the size of a session must
/// change the manifest.
#[tokio::test(start_paused = true)]
async fn default_parameters_move_the_expected_bytes() {
    let doc: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(common::module_dir().join("manifest.yaml")).unwrap(),
    )
    .unwrap();
    let all = common::scenarios();
    let mut bad = Vec::new();
    if let Some(keys) = doc["expected_bytes"].as_mapping() {
        for key in keys.keys() {
            let key = key.as_str().unwrap_or_default();
            if !all.iter().any(|(name, _)| name == key) {
                bad.push(format!("{key}: an estimate for no scenario"));
            }
        }
    }
    for (name, s) in all {
        let mut sizes = Vec::new();
        for id in 1..=ESTIMATE_SESSIONS {
            let r = common::run(&s, common::SEED, id).await;
            assert!(r.client.is_ok(), "{name}: {:?}", r.client);
            assert!(r.server.is_ok(), "{name}: {:?}", r.server);
            sizes.push(r.c2s.len() + r.s2c.len());
        }
        let mean = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        match doc["expected_bytes"][name.as_str()].as_f64() {
            Some(want) if (mean - want).abs() / want < 0.02 => {}
            Some(want) => bad.push(format!(
                "{name}: {mean:.0} bytes against the estimate {want}"
            )),
            None => bad.push(format!("{name}: {mean:.0} bytes and no estimate")),
        }
        eprintln!(
            "MEASURED {name} mean {mean:.1} min {} max {}",
            sizes.iter().min().unwrap(),
            sizes.iter().max().unwrap()
        );
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
