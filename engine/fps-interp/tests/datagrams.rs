// SPDX-License-Identifier: GPL-3.0-only
//! UDP flows under the datagram rules of contract v0.2, with a client and a
//! server program running against each other over the in memory datagram
//! pair of `fps-io`.

mod common;

use std::future::Future;
use std::io;
use std::task::{Context, Poll};
use std::time::Duration;

use common::compile;
use fps_flow::function::NoFunctions;
use fps_flow::template::Filler;
use fps_flow::Scenario;
use fps_interp::{
    session_rng, DatagramOpener, Failure, FailureKind, Limits, OpenKind, OpenRequest, Reason,
    Session, SessionCtx, Side, SideStats,
};
use fps_io::memory::{datagram_pair, MemoryDatagram};
use fps_io::{DatagramIo, MAX_DATAGRAM};

/// The DNS lookup example of the scenario contract.
const LOOKUP: &str = include_str!("../../../contracts/examples/modules/dns/scenarios/lookup.yaml");

/// Hands out one end of the datagram pair. For `accept` it waits for the
/// first datagram and leaves it queued, as the contract requires.
struct MemOpener {
    io: Option<MemoryDatagram>,
    requests: Vec<(OpenKind, Option<Vec<u8>>, u16)>,
}

impl MemOpener {
    fn new(io: MemoryDatagram) -> Self {
        MemOpener {
            io: Some(io),
            requests: Vec::new(),
        }
    }
}

impl DatagramOpener for MemOpener {
    type Io = MemoryDatagram;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<MemoryDatagram>> {
        self.requests
            .push((request.kind, request.host.map(<[u8]>::to_vec), request.port));
        let io = self.io.take();
        let accept = request.kind == OpenKind::Accept;
        async move {
            let io = io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used"))?;
            if accept {
                io.readable().await?;
            }
            Ok(io)
        }
    }
}

/// A socket whose receives fail as a connected UDP socket fails after ICMP
/// port unreachable.
struct Refused;

impl DatagramIo for Refused {
    fn poll_send(&mut self, _cx: &mut Context<'_>, _d: &[u8]) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_recv(&mut self, _cx: &mut Context<'_>, _buf: &mut [u8]) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::ErrorKind::ConnectionRefused.into()))
    }

    fn close(&mut self) {}
}

struct RefusedOpener;

impl DatagramOpener for RefusedOpener {
    type Io = Refused;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<Refused> {
        Ok(Refused)
    }
}

/// Options of one paired UDP run.
struct Opts {
    queue: usize,
    client_max_datagram: Option<usize>,
    server_limits: Limits,
    client_stop: Option<Duration>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            queue: 16,
            client_max_datagram: None,
            server_limits: Limits::default(),
            client_stop: None,
        }
    }
}

#[derive(Debug)]
struct Pair {
    client: Result<(), Failure>,
    server: Result<(), Failure>,
    client_stats: SideStats,
    server_stats: SideStats,
}

async fn stop_after(d: Option<Duration>) {
    match d {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

fn session<'a>(
    s: &'a Scenario,
    filler: &'a Filler,
    side: Side,
    functions: &'a mut NoFunctions,
    limits: Limits,
) -> Session<'a> {
    Session::new(SessionCtx {
        vars: &s.vars,
        filler,
        flow: &s.flows[0].id,
        side,
        session_id: 3,
        rng: session_rng(42, 3, side),
        functions,
        limits,
    })
}

async fn run_with<C: DatagramOpener, S: DatagramOpener>(
    s: &Scenario,
    client_opener: &mut C,
    server_opener: &mut S,
    opts: Opts,
) -> Pair {
    let filler = Filler::new(42);
    let (mut cf, mut sf) = (NoFunctions, NoFunctions);
    let mut client = session(s, &filler, Side::Client, &mut cf, Limits::default());
    if let Some(m) = opts.client_max_datagram {
        client.set_max_datagram(m);
    }
    let mut server = session(s, &filler, Side::Server, &mut sf, opts.server_limits);
    let flow = &s.flows[0];
    let (c, sv) = tokio::join!(
        client.run_datagram_program(client_opener, &flow.client, stop_after(opts.client_stop)),
        server.run_datagram_program(
            server_opener,
            flow.server.as_ref().unwrap(),
            std::future::pending()
        ),
    );
    Pair {
        client: c,
        server: sv,
        client_stats: client.stats(),
        server_stats: server.stats(),
    }
}

async fn run_udp(text: &str, opts: Opts) -> Pair {
    let s = compile(text);
    let (a, b) = datagram_pair(opts.queue);
    run_with(&s, &mut MemOpener::new(a), &mut MemOpener::new(b), opts).await
}

#[track_caller]
fn assert_failure(r: &Result<(), Failure>, reason: Reason, action_index: usize, action_type: &str) {
    let f = r.as_ref().expect_err("the side should fail");
    assert_eq!(f.reason, reason, "{f}: {:?}", f.kind);
    assert_eq!(f.action_index, action_index, "{f}");
    assert_eq!(f.action_type, action_type, "{f}");
}

/// A UDP flow with one exchange. The client sends `query` and expects with
/// `rule`, and the server answers with `answer`.
fn exchange(query: &str, answer: &str, rule: &str) -> String {
    format!(
        r#"
scenario: x
flows:
  - id: x
    module: m
    l4: udp
    client:
      - connect: {{ port: 53 }}
      - send: "{query}"
      - expect: {{ {rule}, timeout_ms: 1000 }}
      - close: fin
    server:
      - accept: {{ port: 53 }}
      - expect: {{ min_bytes: 1, timeout_ms: 1000 }}
      - send: "{answer}"
      - close: fin
"#
    )
}

#[tokio::test]
async fn the_dns_lookup_example_completes() {
    let s = compile(LOOKUP);
    let (a, b) = datagram_pair(4);
    let (mut client, mut server) = (MemOpener::new(a), MemOpener::new(b));
    let pair = run_with(&s, &mut client, &mut server, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    // A 34 byte query for www.example.test and a 50 byte answer with one A record.
    assert_eq!(pair.client_stats.bytes_tx, 34);
    assert_eq!(pair.server_stats.bytes_rx, 34);
    assert_eq!(pair.server_stats.bytes_tx, 50);
    assert_eq!(pair.client_stats.bytes_rx, 50);
    assert!(pair.client_stats.established && pair.server_stats.established);
    assert_eq!(client.requests, [(OpenKind::Connect, None, 53)]);
    assert_eq!(server.requests, [(OpenKind::Accept, None, 53)]);
}

/// The server echoes each query, so `last` must hold the whole datagram,
/// and two sends in a row stay two datagrams.
#[tokio::test]
async fn each_send_is_one_datagram_and_last_is_the_whole_datagram() {
    let text = r#"
scenario: echo
flows:
  - id: e
    module: m
    l4: udp
    client:
      - connect: { host: "10.0.0.{{ session.id }}", port: 5353 }
      - loop:
          count: 3
          do:
            - send: "{{ hex('abcd0100000100000000000003777777') }}"
            - expect: { bytes: 16, timeout_ms: 1000 }
      - send: "a"
      - send: "bb"
      - expect: { bytes: 1, timeout_ms: 1000 }
      - expect: { bytes: 2, timeout_ms: 1000 }
      - close: fin
    server:
      - accept: { port: 5353 }
      - loop:
          count: 3
          do:
            - expect: { min_bytes: 12, timeout_ms: 1000 }
            - send: "{{ last }}"
      - expect: { pattern: "a", timeout_ms: 1000 }
      - send: "{{ last }}"
      - expect: { bytes: 2, timeout_ms: 1000 }
      - send: "{{ last }}"
      - close: fin
"#;
    let s = compile(text);
    let (a, b) = datagram_pair(4);
    let mut client = MemOpener::new(a);
    let pair = run_with(&s, &mut client, &mut MemOpener::new(b), Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(pair.client_stats.bytes_tx, 3 * 16 + 3);
    assert_eq!(pair.client_stats.bytes_rx, 3 * 16 + 3);
    assert_eq!(client.requests[0].1.as_deref(), Some(&b"10.0.0.3"[..]));
}

#[tokio::test]
async fn a_datagram_of_the_wrong_size_fails_bytes_without_a_remainder() {
    // A TCP expect would consume 4 bytes and leave 1, a UDP expect fails.
    let pair = run_udp(&exchange("q", "12345", "bytes: 4"), Opts::default()).await;
    assert_failure(&pair.client, Reason::PatternMismatch, 2, "expect");
    let f = pair.client.unwrap_err();
    assert!(
        matches!(*f.kind, FailureKind::DatagramMismatch { size: 5, ref rule } if rule == "bytes 4"),
        "{:?}",
        f.kind
    );
    pair.server.unwrap();
}

#[tokio::test]
async fn min_bytes_and_pattern_check_the_one_datagram() {
    let pair = run_udp(&exchange("q", "short", "min_bytes: 6"), Opts::default()).await;
    assert_failure(&pair.client, Reason::PatternMismatch, 2, "expect");
    let pair = run_udp(&exchange("q", "short", "min_bytes: 5"), Opts::default()).await;
    pair.client.unwrap();
    let pair = run_udp(
        &exchange("q", "no match", "pattern: \"OK\""),
        Opts::default(),
    )
    .await;
    assert_failure(&pair.client, Reason::PatternMismatch, 2, "expect");
    let pair = run_udp(&exchange("q", "is OK.", "pattern: \"OK\""), Opts::default()).await;
    pair.client.unwrap();
}

#[tokio::test]
async fn an_expect_never_joins_two_datagrams() {
    // Two datagrams of 3 bytes do not make one of 6.
    let text = r#"
scenario: j
flows:
  - id: j
    module: m
    l4: udp
    client:
      - connect: { port: 9 }
      - send: "go"
      - expect: { min_bytes: 6, timeout_ms: 1000 }
    server:
      - accept: { port: 9 }
      - expect: { bytes: 2, timeout_ms: 1000 }
      - send: "abc"
      - send: "def"
"#;
    let pair = run_udp(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::PatternMismatch, 2, "expect");
    pair.server.unwrap();
}

#[tokio::test]
async fn a_datagram_above_max_datagram_fails_the_session() {
    let answer = "0123456789abcdef";
    let at_limit = Opts {
        client_max_datagram: Some(16),
        ..Opts::default()
    };
    let pair = run_udp(&exchange("q", answer, "min_bytes: 1"), at_limit).await;
    pair.client.unwrap();

    let above = Opts {
        client_max_datagram: Some(15),
        ..Opts::default()
    };
    let pair = run_udp(&exchange("q", answer, "min_bytes: 1"), above).await;
    assert_failure(&pair.client, Reason::PatternMismatch, 2, "expect");
    let f = pair.client.unwrap_err();
    assert!(
        matches!(
            *f.kind,
            FailureKind::DatagramTooLarge {
                size: 16,
                limit: 15
            }
        ),
        "{:?}",
        f.kind
    );
    // The oversized datagram still counts as received.
    assert_eq!(pair.client_stats.bytes_rx, 16);
}

#[tokio::test]
async fn the_default_limit_accepts_the_largest_udp_payload() {
    let text = exchange("q", "{{ fill(65507) }}", "bytes: 65507");
    let pair = run_udp(&text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(pair.client_stats.bytes_rx, MAX_DATAGRAM as u64);
}

#[tokio::test]
async fn a_send_above_the_udp_limit_fails_the_session() {
    let text = exchange("q", "{{ fill(65508) }}", "min_bytes: 1");
    let pair = run_udp(&text, Opts::default()).await;
    assert_failure(&pair.server, Reason::Internal, 2, "send");
    assert_eq!(pair.server_stats.bytes_tx, 0);
}

#[test]
fn set_max_datagram_clamps_to_the_udp_range() {
    let s = compile(LOOKUP);
    let filler = Filler::new(1);
    let mut f = NoFunctions;
    let mut client = session(&s, &filler, Side::Client, &mut f, Limits::default());
    assert_eq!(client.max_datagram(), MAX_DATAGRAM);
    client.set_max_datagram(0);
    assert_eq!(client.max_datagram(), 1);
    client.set_max_datagram(1 << 20);
    assert_eq!(client.max_datagram(), MAX_DATAGRAM);
    client.set_max_datagram(512);
    assert_eq!(client.max_datagram(), 512);
}

#[tokio::test(start_paused = true)]
async fn expect_times_out_when_no_datagram_arrives() {
    let text = r#"
scenario: t
flows:
  - id: t
    module: m
    l4: udp
    client:
      - connect: { port: 53 }
      - send: "q"
      - expect: { min_bytes: 1, timeout_ms: 500 }
    server:
      - accept: { port: 53 }
      - expect: { min_bytes: 1, timeout_ms: 1000 }
"#;
    let start = tokio::time::Instant::now();
    let pair = run_udp(text, Opts::default()).await;
    assert_failure(&pair.client, Reason::ExpectTimeout, 2, "expect");
    let f = pair.client.unwrap_err();
    assert!(matches!(
        *f.kind,
        FailureKind::ExpectTimeout {
            timeout_ms: 500,
            received: 0
        }
    ));
    pair.server.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(500));
}

#[tokio::test(start_paused = true)]
async fn a_lost_datagram_is_not_retransmitted() {
    // A queue of zero drops every datagram, as a lossy path does.
    let opts = Opts {
        queue: 0,
        server_limits: Limits {
            open_timeout: Duration::from_millis(300),
            ..Limits::default()
        },
        ..Opts::default()
    };
    let pair = run_udp(&exchange("q", "a", "min_bytes: 1"), opts).await;
    assert_failure(&pair.client, Reason::ExpectTimeout, 2, "expect");
    // Accept waits for the first datagram, so the server never opens.
    assert_failure(&pair.server, Reason::ConnectTimeout, 0, "accept");
    assert!(!pair.server_stats.established);
}

#[tokio::test(start_paused = true)]
async fn the_timeout_bounds_each_datagram_not_the_whole_answer() {
    // Three datagrams 300 ms apart take 900 ms, and each wait stays below 400 ms.
    let text = r#"
scenario: m
flows:
  - id: m
    module: m
    l4: udp
    client:
      - connect: { port: 53 }
      - send: "q"
      - loop:
          count: 3
          do:
            - expect: { min_bytes: 1, timeout_ms: 400 }
    server:
      - accept: { port: 53 }
      - expect: { min_bytes: 1, timeout_ms: 1000 }
      - loop:
          count: 3
          do:
            - delay: { ms: 300 }
            - send: "part"
"#;
    let start = tokio::time::Instant::now();
    let pair = run_udp(text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(900));
    assert_eq!(pair.client_stats.bytes_rx, 12);
}

#[tokio::test]
async fn icmp_port_unreachable_fails_with_connect_refused() {
    let s = compile(&exchange("q", "a", "min_bytes: 1"));
    let (_a, b) = datagram_pair(4);
    let pair = run_with(
        &s,
        &mut RefusedOpener,
        &mut MemOpener::new(b),
        Opts::default(),
    )
    .await;
    assert_failure(&pair.client, Reason::ConnectRefused, 2, "expect");
}

#[tokio::test(start_paused = true)]
async fn stop_closes_the_socket_and_aborts() {
    let text = r#"
scenario: s
flows:
  - id: s
    module: m
    l4: udp
    client:
      - connect: { port: 53 }
      - send: "q"
      - expect: { min_bytes: 1, timeout_ms: 5000 }
    server:
      - accept: { port: 53 }
      - expect: { min_bytes: 1, timeout_ms: 1000 }
      - delay: { ms: 500 }
      - send: "late"
"#;
    let s = compile(text);
    let (a, b) = datagram_pair(4);
    let mut server = MemOpener::new(b);
    let opts = Opts {
        client_stop: Some(Duration::from_millis(100)),
        ..Opts::default()
    };
    let pair = run_with(&s, &mut MemOpener::new(a), &mut server, opts).await;
    assert_failure(&pair.client, Reason::Aborted, 2, "expect");
    // UDP has no reset, so the server's late answer goes nowhere and it succeeds.
    pair.server.unwrap();
}

#[tokio::test]
async fn every_close_mode_releases_the_socket() {
    for mode in ["fin", "rst", "half"] {
        let text =
            exchange("q", "a", "min_bytes: 1").replace("close: fin", &format!("close: {mode}"));
        let pair = run_udp(&text, Opts::default()).await;
        pair.client.unwrap();
        pair.server.unwrap();
    }
}
