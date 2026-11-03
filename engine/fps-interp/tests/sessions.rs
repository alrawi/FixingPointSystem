// SPDX-License-Identifier: GPL-3.0-only
//! Client and server programs that complete against each other over the in
//! memory duplex.

mod common;

use std::time::Duration;

use common::{compile, run_pair, run_recorded, run_with, OneShot, Opts, Upper};
use fps_interp::OpenKind;
use fps_io::memory::duplex;

const PINGPONG: &str = r#"
scenario: pingpong
vars: { n: 3, size: 70000, port: 8080 }
flows:
  - id: p
    module: m
    client:
      - connect: { host: "10.0.0.{{ session.id }}", port: "{{ vars.port }}" }
      - set: { tag: "{{ rand_int(5, 5) }}" }
      - loop:
          count: "{{ vars.n }}"
          do:
            - send: "REQ {{ vars.tag }}\r\n\r\n"
            - expect: { pattern: "\r\n\r\n", timeout_ms: 2000 }
            - expect: { bytes: "{{ vars.size }}", timeout_ms: 2000 }
      - delay: { dist: uniform, min_ms: 1, max_ms: 2 }
      - close: fin
    server:
      - accept: { port: "{{ vars.port }}" }
      - loop:
          count: "{{ vars.n }}"
          do:
            - expect: { pattern: "\r\n\r\n", timeout_ms: 2000 }
            - send: "RESP\r\n\r\n{{ fill(vars.size) }}"
      - close: fin
"#;

#[tokio::test]
async fn client_and_server_complete_and_count_bytes() {
    let s = compile(PINGPONG);
    let (a, b) = duplex(8192);
    let (mut client, mut server) = (OneShot::new(a), OneShot::new(b));
    let pair = run_with(&s, &mut client, &mut server, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    let request = "REQ 5\r\n\r\n".len() as u64;
    assert_eq!(pair.client_stats.bytes_tx, 3 * request);
    assert_eq!(pair.client_stats.bytes_rx, 3 * (70_000 + 8));
    assert_eq!(pair.server_stats.bytes_rx, pair.client_stats.bytes_tx);
    assert_eq!(pair.server_stats.bytes_tx, pair.client_stats.bytes_rx);
    assert!(pair.client_stats.established && pair.server_stats.established);
    assert_eq!(
        client.requests,
        [(OpenKind::Connect, Some(b"10.0.0.7".to_vec()), 8080)]
    );
    assert_eq!(server.requests, [(OpenKind::Accept, None, 8080)]);
}

#[tokio::test]
async fn pattern_split_across_one_byte_reads() {
    // A smaller body keeps one byte reads fast in debug builds.
    let pair = run_pair(
        &PINGPONG.replace("size: 70000", "size: 700"),
        Opts {
            capacity: 1,
            ..Opts::default()
        },
    )
    .await;
    pair.client.unwrap();
    pair.server.unwrap();
}

/// The server echoes `last` after each of the three stop rules, so the
/// client sees exactly what each rule consumed.
#[tokio::test]
async fn expect_modes_consume_and_bind_last() {
    let text = r#"
scenario: modes
flows:
  - id: e
    module: m
    client:
      - connect: { port: 1 }
      - send: "head\r\n\r\nrest"
      - expect: { pattern: "<head\r\n\r\n>", timeout_ms: 1000 }
      - send: "0123456789"
      - expect: { pattern: "<rest01>", timeout_ms: 1000 }
      - expect: { pattern: "<23456789>", timeout_ms: 1000 }
      - send: "abc"
      - expect: { min_bytes: 6, timeout_ms: 1000 }
      - close: fin
    server:
      - accept: { port: 1 }
      - expect: { pattern: "\r\n\r\n", timeout_ms: 1000 }
      - send: "<{{ last }}>"
      - expect: { bytes: 6, timeout_ms: 1000 }
      - send: "<{{ last }}>"
      - expect: { min_bytes: 8, timeout_ms: 1000 }
      - send: "<{{ last }}>"
      - expect: { min_bytes: 3, timeout_ms: 1000 }
      - send: "{{ last }}{{ last }}"
      - close: fin
"#;
    let (pair, c2s, s2c) = run_recorded(text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(c2s, b"head\r\n\r\nrest0123456789abc");
    // Bytes after the pattern stay buffered, so the bytes rule sees "rest" first.
    assert_eq!(s2c, b"<head\r\n\r\n><rest01><23456789>abcabc");
}

#[tokio::test]
async fn bytes_past_the_count_stay_for_the_next_expect() {
    let text = r#"
scenario: split
flows:
  - id: e
    module: m
    client:
      - connect: { port: 1 }
      - expect: { bytes: 100000, timeout_ms: 1000 }
      - expect: { pattern: "END", timeout_ms: 1000 }
      - close: fin
    server:
      - accept: { port: 1 }
      - send: "{{ fill(100000) }}END"
      - close: fin
"#;
    let pair = run_pair(
        text,
        Opts {
            capacity: 100_003,
            ..Opts::default()
        },
    )
    .await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(pair.client_stats.bytes_rx, 100_003);
}

const RANDOM: &str = r#"
scenario: random
flows:
  - id: r
    module: m
    client:
      - connect: { port: 1 }
      - loop:
          count: 20
          do:
            - branch:
                - { p: 0.5, do: [ { send: "a" } ] }
                - { p: 0.5, do: [ { send: "b{{ rand_int(0, 9) }}" } ] }
      - send: "|{{ fill(32) }}|{{ session.id }}"
      - close: fin
    server:
      - accept: { port: 1 }
      - close: fin
"#;

#[tokio::test]
async fn same_seed_and_session_reproduce_the_bytes() {
    let run = |seed, session_id| async move {
        let opts = Opts {
            seed,
            session_id,
            ..Opts::default()
        };
        let (pair, c2s, _) = run_recorded(RANDOM, opts).await;
        pair.client.unwrap();
        pair.server.unwrap();
        c2s
    };
    let first = run(42, 7).await;
    assert_eq!(first, run(42, 7).await);
    assert_ne!(first, run(42, 6).await);
    assert_ne!(first, run(43, 7).await);
    // Both arms of the branch occur in 20 draws.
    assert!(first.contains(&b'a') && first.contains(&b'b'));
}

#[tokio::test]
async fn loops_branches_and_set_drive_the_program() {
    let text = r#"
scenario: control
vars: { n: 4, greeting: none }
flows:
  - id: c
    module: m
    client:
      - connect: { port: 1 }
      - set: { greeting: "hi", count: "{{ vars.n }}" }
      - loop:
          count: "{{ vars.count }}"
          do:
            - loop:
                count: 2
                do:
                  - send: "{{ vars.greeting }}"
      - loop:
          count: 0
          do:
            - send: "never"
      - branch:
          - { p: 1.0, do: [ { send: "!" } ] }
      - close: fin
    server:
      - accept: { port: 1 }
      - expect: { bytes: 17, timeout_ms: 1000 }
      - close: fin
"#;
    let (pair, c2s, _) = run_recorded(text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(c2s, b"hihihihihihihihi!");
}

#[tokio::test(start_paused = true)]
async fn delays_wait_on_the_tokio_clock() {
    let text = r#"
scenario: delays
vars: { wait: 250 }
flows:
  - id: d
    module: m
    client:
      - connect: { port: 1 }
      - delay: { ms: "{{ vars.wait }}" }
      - delay: { dist: uniform, min_ms: 100, max_ms: 200 }
      - delay: { dist: exponential, mean_ms: 10 }
      - delay: { dist: lognormal, mu: 0.0, sigma: 0.1, unit: ms }
      - delay: { dist: pareto, alpha: 2.0, scale_ms: 5 }
      - send: "done"
      - close: fin
    server:
      - accept: { port: 1 }
      - expect: { pattern: "done", timeout_ms: 60000 }
      - close: fin
"#;
    let start = tokio::time::Instant::now();
    let pair = run_pair(text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    let elapsed = start.elapsed();
    // Fixed 250, uniform at least 100, Pareto at least 5, the rest positive.
    assert!(elapsed >= Duration::from_millis(355), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

#[tokio::test]
async fn module_functions_render_through_the_session_table() {
    let text = r#"
scenario: modules
flows:
  - id: m
    module: text
    client:
      - connect: { port: 1 }
      - send: "{{ text.upper('hello') }}"
      - expect: { bytes: 5, timeout_ms: 1000 }
      - send: "{{ text.upper(last) }}"
      - close: fin
    server:
      - accept: { port: 1 }
      - expect: { bytes: 5, timeout_ms: 1000 }
      - send: "quiet"
      - expect: { bytes: 5, timeout_ms: 1000 }
      - close: fin
"#;
    let opts = Opts {
        client_functions: Box::new(Upper),
        ..Opts::default()
    };
    let (pair, c2s, _) = run_recorded(text, opts).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(c2s, b"HELLOQUIET");
}

#[tokio::test]
async fn reset_after_our_fin_ends_cleanly() {
    let text = r#"
scenario: rst
flows:
  - id: r
    module: m
    client:
      - connect: { port: 1 }
      - send: "bye"
      - close: fin
    server:
      - accept: { port: 1 }
      - expect: { pattern: "bye", timeout_ms: 1000 }
      - delay: { ms: 20 }
      - close: rst
"#;
    let pair = run_pair(text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
}

#[tokio::test]
async fn half_close_keeps_the_read_side_open() {
    let text = r#"
scenario: half
flows:
  - id: h
    module: m
    client:
      - connect: { port: 1 }
      - send: "upload"
      - close: half
      - expect: { pattern: "ok", timeout_ms: 1000 }
    server:
      - accept: { port: 1 }
      - expect: { bytes: 6, timeout_ms: 1000 }
      - send: "ok"
      - close: fin
"#;
    let pair = run_pair(text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
}
