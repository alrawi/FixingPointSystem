// SPDX-License-Identifier: GPL-3.0-only
//! One test per failure reason of the metric catalog. Each test runs a
//! client and a server program against each other over the in memory
//! duplex, or replaces one side's transport with a fault, and checks the
//! reason, the action index, and the action type of the failure.

mod common;

use std::io;
use std::time::Duration;

use common::{compile, run_pair, run_with, Failing, Hanging, OneShot, Opts, Tap};
use fps_interp::{Failure, FailureKind, Limits, Reason};
use fps_io::memory::duplex;

#[track_caller]
fn assert_failure(r: &Result<(), Failure>, reason: Reason, action_index: usize, action_type: &str) {
    let f = r.as_ref().expect_err("the side should fail");
    assert_eq!(f.reason, reason, "{f}: {:?}", f.kind);
    assert_eq!(f.action_index, action_index, "{f}");
    assert_eq!(f.action_type, action_type, "{f}");
}

const SIMPLE: &str = r#"
scenario: simple
flows:
  - id: s
    module: m
    client:
      - connect: { port: 80 }
      - send: "hi"
    server:
      - accept: { port: 80 }
      - expect: { pattern: "hi", timeout_ms: 1000 }
"#;

#[tokio::test(start_paused = true)]
async fn connect_timeout_when_the_open_never_completes() {
    let s = compile(SIMPLE);
    let (_a, b) = duplex(64);
    let opts = Opts {
        client_limits: Limits {
            open_timeout: Duration::from_millis(100),
            ..Limits::default()
        },
        server_limits: Limits {
            open_timeout: Duration::from_millis(100),
            ..Limits::default()
        },
        ..Opts::default()
    };
    let pair = run_with(&s, &mut Hanging, &mut OneShot::new(b), opts).await;
    assert_failure(&pair.client, Reason::ConnectTimeout, 0, "connect");
    assert!(!pair.client_stats.established);
    // The server's peer end stays open but silent, so its expect times out.
    assert_failure(&pair.server, Reason::ExpectTimeout, 1, "expect");
}

#[tokio::test(start_paused = true)]
async fn connect_timeout_when_the_transport_times_out() {
    let s = compile(SIMPLE);
    let (_a, b) = duplex(64);
    let pair = run_with(
        &s,
        &mut Failing(io::ErrorKind::TimedOut),
        &mut OneShot::new(b),
        Opts::default(),
    )
    .await;
    assert_failure(&pair.client, Reason::ConnectTimeout, 0, "connect");
}

#[tokio::test]
async fn connect_refused_when_the_peer_refuses() {
    let s = compile(SIMPLE);
    let (a, b) = duplex(64);
    drop(b);
    let pair = run_with(
        &s,
        &mut Failing(io::ErrorKind::ConnectionRefused),
        &mut OneShot::new(a),
        Opts::default(),
    )
    .await;
    assert_failure(&pair.client, Reason::ConnectRefused, 0, "connect");
    // The server got a transport whose peer end was dropped, so it reads end of stream.
    assert_failure(&pair.server, Reason::PeerClosed, 1, "expect");
}

#[tokio::test(start_paused = true)]
async fn expect_timeout_when_the_peer_is_slow() {
    let text = r#"
scenario: slow
flows:
  - id: s
    module: m
    client:
      - connect: { port: 1 }
      - send: "q"
      - expect: { pattern: "never", timeout_ms: 50 }
    server:
      - accept: { port: 1 }
      - delay: { ms: 300 }
"#;
    let pair = run_pair(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::ExpectTimeout, 2, "expect");
    let f = pair.client.unwrap_err();
    assert!(matches!(
        f.kind,
        FailureKind::ExpectTimeout {
            timeout_ms: 50,
            received: 0
        }
    ));
    pair.server.unwrap();
}

#[tokio::test(start_paused = true)]
async fn expect_timeout_when_the_peer_never_closes_after_fin() {
    let text = r#"
scenario: linger
flows:
  - id: l
    module: m
    client:
      - connect: { port: 1 }
      - close: fin
    server:
      - accept: { port: 1 }
      - delay: { ms: 5000 }
"#;
    let opts = Opts {
        client_limits: Limits {
            drain_timeout: Duration::from_millis(100),
            ..Limits::default()
        },
        ..Opts::default()
    };
    let pair = run_pair(text, opts).await;
    assert_failure(&pair.client, Reason::ExpectTimeout, 1, "close");
    assert!(matches!(
        pair.client.unwrap_err().kind,
        FailureKind::DrainTimeout { timeout_ms: 100 }
    ));
}

#[tokio::test]
async fn pattern_mismatch_when_unmatched_data_exceeds_the_limit() {
    let text = r#"
scenario: flood
flows:
  - id: f
    module: m
    client:
      - connect: { port: 1 }
      - expect: { pattern: "\r\n\r\n", timeout_ms: 1000 }
    server:
      - accept: { port: 1 }
      - send: "{{ fill(70000) }}"
      - close: fin
"#;
    let opts = Opts {
        client_limits: Limits {
            max_pattern_buffer: 4096,
            ..Limits::default()
        },
        ..Opts::default()
    };
    let pair = run_pair(text, opts).await;
    assert_failure(&pair.client, Reason::PatternMismatch, 1, "expect");
    // The client dropped its transport after failing, so the rest of the send fails.
    assert_failure(&pair.server, Reason::PeerClosed, 1, "send");
}

#[tokio::test]
async fn unexpected_reset_when_the_peer_aborts() {
    let text = r#"
scenario: reset
flows:
  - id: r
    module: m
    client:
      - connect: { port: 1 }
      - send: "q"
      - expect: { bytes: 10, timeout_ms: 1000 }
    server:
      - accept: { port: 1 }
      - expect: { bytes: 1, timeout_ms: 1000 }
      - send: "part"
      - close: rst
"#;
    let pair = run_pair(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::UnexpectedReset, 2, "expect");
    pair.server.unwrap();
}

#[tokio::test]
async fn peer_closed_when_the_peer_ends_before_the_expect() {
    let text = r#"
scenario: short
flows:
  - id: s
    module: m
    client:
      - connect: { port: 1 }
      - expect: { bytes: 10, timeout_ms: 1000 }
    server:
      - accept: { port: 1 }
      - send: "short"
      - close: half
"#;
    let pair = run_pair(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::PeerClosed, 1, "expect");
    assert!(matches!(
        pair.client.unwrap_err().kind,
        FailureKind::PeerClosed { received: 5 }
    ));
    pair.server.unwrap();
}

#[tokio::test]
async fn peer_closed_when_a_send_meets_a_closed_peer() {
    let text = r#"
scenario: gone
flows:
  - id: g
    module: m
    client:
      - connect: { port: 1 }
      - delay: { ms: 20 }
      - send: "anyone there"
    server:
      - accept: { port: 1 }
"#;
    let pair = run_pair(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::PeerClosed, 2, "send");
    assert!(matches!(
        pair.client.unwrap_err().kind,
        FailureKind::BrokenPipe { .. }
    ));
    pair.server.unwrap();
}

#[tokio::test]
async fn tls_failure_when_the_transport_reports_invalid_data() {
    let text = r#"
scenario: tls
flows:
  - id: t
    module: m
    client:
      - connect: { port: 443 }
      - send: "hello"
      - expect: { min_bytes: 1, timeout_ms: 1000 }
    server:
      - accept: { port: 443 }
      - expect: { bytes: 5, timeout_ms: 1000 }
      - send: "alert"
"#;
    let s = compile(text);
    let (a, b) = duplex(64);
    let a = Tap::failing_reads(a, io::ErrorKind::InvalidData);
    let pair = run_with(
        &s,
        &mut OneShot::new(a),
        &mut OneShot::new(b),
        Opts::default(),
    )
    .await;
    assert_failure(&pair.client, Reason::TlsFailure, 2, "expect");
    assert_failure(&pair.server, Reason::PeerClosed, 2, "send");
}

#[tokio::test(start_paused = true)]
async fn aborted_when_the_engine_stops_the_session() {
    let text = r#"
scenario: stop
flows:
  - id: s
    module: m
    client:
      - connect: { port: 1 }
      - send: "q"
      - delay: { ms: 10000 }
    server:
      - accept: { port: 1 }
      - expect: { bytes: 1, timeout_ms: 60000 }
      - expect: { bytes: 1, timeout_ms: 60000 }
"#;
    let opts = Opts {
        client_stop: Some(Duration::from_millis(100)),
        ..Opts::default()
    };
    let start = tokio::time::Instant::now();
    let pair = run_pair(text, opts).await;
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_failure(&pair.client, Reason::Aborted, 2, "delay");
    // The abort resets the transport, which the waiting server observes.
    assert_failure(&pair.server, Reason::UnexpectedReset, 2, "expect");
}

#[tokio::test]
async fn internal_when_a_module_function_fails() {
    let text = r#"
scenario: ext
flows:
  - id: e
    module: m
    client:
      - connect: { port: 1 }
      - loop:
          count: 2
          do:
            - send: "{{ dns.query('example.test') }}"
    server:
      - accept: { port: 1 }
"#;
    let pair = run_pair(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::Internal, 2, "send");
    assert_eq!(
        pair.client.unwrap_err().extension_label().as_deref(),
        Some("extension_unsupported")
    );
}

#[tokio::test]
async fn internal_when_a_rendered_parameter_is_out_of_range() {
    let text = r#"
scenario: range
vars: { size: -1, port: 70000 }
flows:
  - id: r
    module: m
    client:
      - connect: { port: 1 }
      - expect: { bytes: "{{ vars.size }}", timeout_ms: 1000 }
    server:
      - accept: { port: "{{ vars.port }}" }
"#;
    let pair = run_pair(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::Internal, 1, "expect");
    assert_failure(&pair.server, Reason::Internal, 0, "accept");
    assert!(pair.client.unwrap_err().extension_label().is_none());
}
