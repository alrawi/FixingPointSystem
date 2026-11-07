// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the session capture.
//!
//! They run sessions over the in memory duplex, capture them into a ring, and
//! read the ring back with the independent pcapng reader of `common`. The
//! tshark tests skip themselves when tshark is missing, unless the environment
//! variable `FPS_REQUIRE_CAPTURE_TOOLS` is `1`, which CI jobs that install the
//! tool use so that a missing tool fails.

mod common;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use common::{
    fresh_dir, packets, read_pcapng, reassemble, skip_without, tool_available, StepClock, ACK, RST,
    SYN,
};
use fps_capture::{
    Capture, CaptureConfig, FlowEndpoints, KeyLogMode, RingConfig, SamplePolicy, Side, Transport,
};
use fps_io::memory::{datagram_pair, duplex};
use fps_io::{DatagramIoExt, SessionIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const REQUEST: &[u8] = b"GET /fps-capture-sample HTTP/1.1\r\nHost: capture.example.test\r\n\r\n";

fn config(dir: &Path, sample: SamplePolicy) -> CaptureConfig {
    let mut ring = RingConfig::new(dir);
    ring.prefix = "t".into();
    let mut c = CaptureConfig::new(ring);
    c.sample = sample;
    c.seed = 42;
    c
}

fn start(c: CaptureConfig) -> Capture {
    Capture::start(c, Arc::new(StepClock::default())).unwrap()
}

fn v4(client_port: u16, server_port: u16) -> FlowEndpoints {
    FlowEndpoints::new(
        SocketAddr::from(([10, 0, 0, 1], client_port)),
        SocketAddr::from(([10, 0, 0, 2], server_port)),
    )
    .unwrap()
}

fn v6(client_port: u16, server_port: u16) -> FlowEndpoints {
    FlowEndpoints::new(
        format!("[2001:db8::1]:{client_port}").parse().unwrap(),
        format!("[2001:db8::2]:{server_port}").parse().unwrap(),
    )
    .unwrap()
}

/// One request and one response over a tapped duplex, closed by both sides.
async fn exchange(
    capture: &Capture,
    id: u64,
    flow: FlowEndpoints,
    request: &[u8],
    response: &[u8],
) {
    let (c, mut s) = duplex(64 * 1024);
    let mut c = capture.tap(id, flow, Side::Client, c);
    let serve = async {
        let mut buf = vec![0u8; request.len()];
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, request);
        s.write_all(response).await.unwrap();
        s.shutdown().await.unwrap();
    };
    let ask = async {
        c.write_all(request).await.unwrap();
        let mut reply = Vec::new();
        c.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, response);
        c.shutdown().await.unwrap();
    };
    tokio::join!(serve, ask);
}

fn body(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(salt))
        .collect()
}

#[tokio::test]
async fn ring_rotates_and_every_retained_file_parses() {
    let dir = fresh_dir("rotate");
    let mut cfg = config(&dir, SamplePolicy::Rate(1.0));
    cfg.ring.file_bytes = 24 * 1024;
    cfg.ring.max_files = 3;
    let capture = start(cfg);
    for i in 0..30u64 {
        let flow = v4(40_000 + i as u16, 80);
        let (c, mut s) = duplex(64 * 1024);
        let mut c = capture.tap(i, flow, Side::Client, c);
        c.handle()
            .unwrap()
            .add_key_lines(&format!("CLIENT_RANDOM {i:064x} {i:096x}\n"));
        let response = body(3_000, i as u8);
        let serve = async {
            let mut buf = vec![0u8; REQUEST.len()];
            s.read_exact(&mut buf).await.unwrap();
            s.write_all(&response).await.unwrap();
            s.shutdown().await.unwrap();
        };
        let ask = async {
            c.write_all(REQUEST).await.unwrap();
            let mut reply = Vec::new();
            c.read_to_end(&mut reply).await.unwrap();
            c.shutdown().await.unwrap();
        };
        tokio::join!(serve, ask);
    }
    let files = capture.flush();
    let stats = capture.stop();

    assert_eq!(stats.sessions_written, 30);
    assert!(
        files.len() <= 3,
        "the ring keeps at most three files, has {files:?}"
    );
    assert_eq!(files.len(), 3);
    assert_eq!(stats.files_opened - stats.files_evicted, 3);
    assert!(
        stats.files_evicted >= 2,
        "30 sessions of 4 KiB rotate a 24 KiB ring: {stats:?}"
    );

    let mut seen = BTreeSet::new();
    for file in &files {
        assert!(
            std::fs::metadata(file).unwrap().len() <= 24 * 1024,
            "{file:?} stays within the size limit"
        );
        let parsed = read_pcapng(file);
        assert_eq!(
            parsed.secrets.len(),
            parsed
                .secrets
                .iter()
                .filter(|(k, _)| *k == 0x544c_534b)
                .count()
        );
        let conns = reassemble(&packets(&parsed), 80);
        assert!(!conns.is_empty());
        for (port, conn) in conns {
            let i = u64::from(port - 40_000);
            assert!(
                conn.saw_syn && conn.saw_syn_ack && conn.fins == 2 && !conn.rst,
                "port {port}: {conn:?}"
            );
            assert_eq!(conn.to_server, REQUEST);
            assert_eq!(conn.to_client, body(3_000, i as u8));
            assert!(seen.insert(i), "session {i} sits in one file only");
        }
        // The key lines of exactly the sessions in this file sit next to it.
        let keys = std::fs::read_to_string(format!("{}.keys", file.display())).unwrap();
        let embedded: String = parsed
            .secrets
            .iter()
            .map(|(_, d)| String::from_utf8(d.clone()).unwrap())
            .collect();
        assert_eq!(keys, embedded, "the sidecar repeats the embedded key log");
        assert_eq!(keys.lines().count(), parsed.secrets.len());
    }
    // The newest sessions survive, in order, and the oldest are gone.
    assert!(seen.contains(&29) && !seen.contains(&0));

    // Nothing but the retained files and their key logs remains in the directory.
    let on_disk: BTreeSet<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let expected: BTreeSet<PathBuf> = files
        .iter()
        .flat_map(|f| [f.clone(), PathBuf::from(format!("{}.keys", f.display()))])
        .collect();
    assert_eq!(on_disk, expected);
}

#[test]
fn one_percent_of_ten_thousand_sessions_captures_between_70_and_130() {
    let dir = fresh_dir("one-percent");
    let mut cfg = config(&dir, SamplePolicy::Rate(0.01));
    cfg.ring.file_bytes = 64 << 20;
    let capture = start(cfg);
    for id in 0..10_000u64 {
        if let Some(h) = capture.begin(
            id,
            v4(1024 + (id % 60_000) as u16, 80),
            Transport::Tcp,
            Side::Client,
        ) {
            h.record_out(b"ping");
            h.record_in(b"pong");
            h.record_local_close();
            h.record_peer_close();
        }
    }
    let files = capture.flush();
    let stats = capture.stop();
    assert!(
        (70..=130).contains(&stats.sessions_written),
        "sampled {} of 10000",
        stats.sessions_written
    );
    assert_eq!(stats.sessions_sampled, stats.sessions_written);
    assert_eq!(stats.sessions_dropped_queue, 0);
    // The files hold exactly that many sessions, one SYN each.
    let mut syns = 0;
    for f in &files {
        syns += packets(&read_pcapng(f))
            .iter()
            .filter(|p| p.flags == SYN)
            .count();
    }
    assert_eq!(syns as u64, stats.sessions_written);
}

#[test]
fn every_nth_sampling_is_exact() {
    let dir = fresh_dir("every-nth");
    let capture = start(config(&dir, SamplePolicy::EveryNth(100)));
    for id in 0..10_000u64 {
        if let Some(h) = capture.begin(
            id,
            v4(1024 + (id % 60_000) as u16, 80),
            Transport::Tcp,
            Side::Client,
        ) {
            h.record_out(b"x");
        }
    }
    capture.flush();
    assert_eq!(capture.stop().sessions_written, 100);
}

#[tokio::test]
async fn sampling_off_starts_nothing_and_wraps_nothing() {
    let dir = fresh_dir("off");
    let capture = start(config(&dir, SamplePolicy::Off));
    assert!(!capture.is_enabled());
    assert!(!dir.exists(), "no directory for a capture that is off");
    let (c, _s) = duplex(1024);
    let tapped = capture.tap(1, v4(1, 2), Side::Client, c);
    assert!(tapped.handle().is_none());
    assert!(matches!(tapped, fps_capture::Tapped::Plain(_)));
    assert!(capture
        .begin(1, v4(1, 2), Transport::Tcp, Side::Client)
        .is_none());
    assert_eq!(capture.stats(), Default::default());
    assert!(capture.flush().is_empty());
    assert_eq!(capture.stop(), Default::default());
    // The same holds for the explicit constructor.
    assert!(Capture::disabled()
        .begin(1, v4(1, 2), Transport::Udp, Side::Server)
        .is_none());
}

#[tokio::test]
async fn unsampled_sessions_are_not_recorded() {
    let dir = fresh_dir("unsampled");
    let capture = start(config(&dir, SamplePolicy::EveryNth(2)));
    for id in 0..10u64 {
        exchange(&capture, id, v4(40_000 + id as u16, 80), REQUEST, b"ok").await;
    }
    capture.flush();
    let stats = capture.stop();
    assert_eq!((stats.sessions_sampled, stats.sessions_written), (5, 5));
}

#[tokio::test]
async fn ipv6_udp_and_abort_frames_parse() {
    let dir = fresh_dir("families");
    let capture = start(config(&dir, SamplePolicy::Rate(1.0)));

    // IPv6 TCP, closed by an abort of the server side.
    let (c, s) = duplex(1024);
    let mut s = capture.tap(1, v6(50_000, 80), Side::Server, s);
    let mut c = c;
    c.write_all(b"hello").await.unwrap();
    let mut buf = [0u8; 5];
    s.read_exact(&mut buf).await.unwrap();
    s.abort();
    drop(s);

    // IPv4 UDP with an empty datagram and a full size one.
    let (a, b) = datagram_pair(8);
    let mut a = capture.tap_datagram(2, v4(50_001, 53), Side::Client, a);
    let mut b = b;
    a.send_datagram(&[7; 100]).await.unwrap();
    a.send_datagram(&[]).await.unwrap();
    let mut buf = vec![0u8; 70_000];
    assert_eq!(b.recv_datagram(&mut buf[..]).await.unwrap(), 100);
    b.send_datagram(&[9; 65_507]).await.unwrap();
    let n = a.recv_datagram(&mut buf[..]).await.unwrap();
    assert_eq!(n, 65_507);
    drop(a);

    // IPv6 UDP.
    let (a, mut b) = datagram_pair(8);
    let mut a = capture.tap_datagram(3, v6(50_002, 53), Side::Client, a);
    a.send_datagram(b"v6 datagram").await.unwrap();
    b.recv_datagram(&mut buf[..]).await.unwrap();
    drop(a);

    let files = capture.flush();
    capture.stop();
    let all: Vec<_> = files
        .iter()
        .flat_map(|f| packets(&read_pcapng(f)))
        .collect();

    let tcp: Vec<_> = all.iter().filter(|p| !p.udp && p.src.is_ipv6()).collect();
    assert_eq!(
        tcp.iter().map(|p| p.flags).collect::<Vec<_>>(),
        vec![SYN, SYN | ACK, ACK, ACK | 0x08, RST | ACK]
    );
    assert_eq!(tcp[3].payload, b"hello");
    assert_eq!(
        tcp[3].sport, 50_000,
        "the server side tap reads what the client wrote"
    );
    assert_eq!(tcp[4].sport, 80, "the aborting server sends the reset");

    let udp4: Vec<_> = all.iter().filter(|p| p.udp && p.src.is_ipv4()).collect();
    assert_eq!(
        udp4.iter().map(|p| p.payload.len()).collect::<Vec<_>>(),
        vec![100, 0, 65_507]
    );
    assert_eq!((udp4[0].dport, udp4[2].sport), (53, 53));

    let udp6: Vec<_> = all.iter().filter(|p| p.udp && p.src.is_ipv6()).collect();
    assert_eq!(udp6.len(), 1);
    assert_eq!(udp6[0].payload, b"v6 datagram");
}

#[tokio::test]
async fn long_sessions_are_truncated_at_the_limit() {
    let dir = fresh_dir("truncate");
    let mut cfg = config(&dir, SamplePolicy::Rate(1.0));
    cfg.max_session_bytes = 1_000;
    let capture = start(cfg);
    exchange(&capture, 1, v4(40_000, 80), REQUEST, &body(5_000, 3)).await;
    let files = capture.flush();
    let stats = capture.stop();
    assert_eq!(stats.sessions_truncated, 1);
    let conns = reassemble(&packets(&read_pcapng(&files[0])), 80);
    let conn = &conns[&40_000];
    assert_eq!(conn.to_server, REQUEST);
    assert_eq!(
        conn.to_client,
        body(5_000, 3)[..1_000 - REQUEST.len()],
        "the stream keeps its prefix"
    );
    assert_eq!(conn.fins, 2, "a truncated session still closes");
}

#[tokio::test]
async fn key_log_modes_place_the_keys() {
    for (mode, embedded, sidecar) in [
        (KeyLogMode::Embedded, true, false),
        (KeyLogMode::Sidecar, false, true),
        (KeyLogMode::Both, true, true),
    ] {
        let dir = fresh_dir("modes");
        let mut cfg = config(&dir, SamplePolicy::Rate(1.0));
        cfg.key_log = mode;
        let capture = start(cfg);
        let h = capture
            .begin(1, v4(40_000, 443), Transport::Tcp, Side::Client)
            .unwrap();
        h.record_out(b"x");
        h.add_key_lines("CLIENT_RANDOM aa bb");
        drop(h);
        let files = capture.flush();
        capture.stop();
        let parsed = read_pcapng(&files[0]);
        assert_eq!(!parsed.secrets.is_empty(), embedded, "{mode:?}");
        if embedded {
            assert_eq!(parsed.secrets[0].1, b"CLIENT_RANDOM aa bb\n");
            // The secrets block comes before the packets of its session.
            assert_eq!(parsed.order[2], 0x0A);
            assert_eq!(parsed.order[3], 0x06);
        }
        let keys = PathBuf::from(format!("{}.keys", files[0].display()));
        assert_eq!(keys.exists(), sidecar, "{mode:?}");
    }
}

#[tokio::test]
async fn the_same_seed_and_clock_give_identical_files() {
    let mut bytes = Vec::new();
    for run in 0..2 {
        let dir = fresh_dir(&format!("determinism{run}"));
        let capture = start(config(&dir, SamplePolicy::Rate(0.5)));
        for id in 0..40u64 {
            exchange(
                &capture,
                id,
                v4(40_000 + id as u16, 80),
                REQUEST,
                &body(2_000, id as u8),
            )
            .await;
        }
        let files = capture.flush();
        capture.stop();
        bytes.push(
            files
                .iter()
                .map(|f| std::fs::read(f).unwrap())
                .collect::<Vec<_>>(),
        );
    }
    assert!(!bytes[0].is_empty());
    assert_eq!(bytes[0], bytes[1]);
}

#[tokio::test]
async fn a_full_queue_drops_sessions_and_counts_them() {
    let dir = fresh_dir("queue");
    let mut cfg = config(&dir, SamplePolicy::Rate(1.0));
    cfg.queue_depth = 1;
    let capture = start(cfg);
    // Finish many sessions in a burst. The writer may keep up or not, and
    // every session is either written or counted as dropped.
    for id in 0..500u64 {
        let h = capture
            .begin(id, v4(1024 + id as u16, 80), Transport::Tcp, Side::Client)
            .unwrap();
        h.record_out(&body(2_000, 1));
    }
    capture.flush();
    let stats = capture.stop();
    assert_eq!(
        stats.sessions_written + stats.sessions_dropped_queue,
        500,
        "{stats:?}"
    );
}

/// Runs tshark with the given extra arguments and returns stdout.
fn tshark(capture: &Path, args: &[&str]) -> String {
    let out = Command::new("tshark")
        .arg("-r")
        .arg(capture)
        .args(args)
        .output()
        .expect("tshark starts");
    assert!(
        out.status.success(),
        "tshark failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn dns_query() -> Vec<u8> {
    let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["fps", "example", "test"] {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]);
    q
}

#[tokio::test]
async fn tshark_parses_the_synthesized_streams() {
    if skip_without("tshark", tool_available("tshark", &["--version"])) {
        return;
    }
    let dir = fresh_dir("tshark-plain");
    let capture = start(config(&dir, SamplePolicy::Rate(1.0)));
    let request = |path: &str| {
        format!("GET {path} HTTP/1.1\r\nHost: capture.example.test\r\n\r\n").into_bytes()
    };
    let response = [
        b"HTTP/1.1 200 OK\r\nContent-Length: 4000\r\n\r\n".to_vec(),
        body(4_000, 0),
    ]
    .concat();
    exchange(
        &capture,
        1,
        v4(40_001, 80),
        &request("/over-ipv4"),
        &response,
    )
    .await;
    exchange(
        &capture,
        2,
        v6(40_002, 80),
        &request("/over-ipv6"),
        &response,
    )
    .await;
    for (id, flow) in [(3, v4(40_003, 53)), (4, v6(40_004, 53))] {
        let (a, mut b) = datagram_pair(8);
        let mut a = capture.tap_datagram(id, flow, Side::Client, a);
        a.send_datagram(&dns_query()).await.unwrap();
        let mut buf = [0u8; 512];
        b.recv_datagram(&mut buf).await.unwrap();
    }
    let files = capture.flush();
    capture.stop();
    assert_eq!(files.len(), 1);
    let file = &files[0];

    let uris = tshark(
        file,
        &[
            "-Y",
            "http.request",
            "-T",
            "fields",
            "-e",
            "http.request.uri",
        ],
    );
    assert!(
        uris.contains("/over-ipv4") && uris.contains("/over-ipv6"),
        "HTTP over both families: {uris:?}"
    );
    let codes = tshark(
        file,
        &[
            "-Y",
            "http.response",
            "-T",
            "fields",
            "-e",
            "http.response.code",
        ],
    );
    assert_eq!(
        codes.lines().filter(|l| *l == "200").count(),
        2,
        "both responses reassemble: {codes:?}"
    );
    let names = tshark(
        file,
        &[
            "-Y",
            "dns.flags.response == 0",
            "-T",
            "fields",
            "-e",
            "dns.qry.name",
        ],
    );
    assert_eq!(
        names.lines().filter(|l| *l == "fps.example.test").count(),
        2,
        "DNS over both families: {names:?}"
    );
    let streams = tshark(file, &["-T", "fields", "-e", "tcp.stream", "-Y", "tcp"]);
    assert_eq!(
        streams.lines().collect::<BTreeSet<_>>().len(),
        2,
        "two TCP conversations"
    );

    // With every checksum verified, nothing is bad, and TCP analysis finds
    // no loss, retransmission, or reordering.
    let strict = [
        "-o",
        "ip.check_checksum:TRUE",
        "-o",
        "tcp.check_checksum:TRUE",
        "-o",
        "udp.check_checksum:TRUE",
    ];
    for filter in [
        "_ws.expert.group == 0x01000000",
        "_ws.malformed",
        "tcp.analysis.retransmission || tcp.analysis.lost_segment || tcp.analysis.ack_lost_segment || tcp.analysis.out_of_order",
    ] {
        let mut args = strict.to_vec();
        args.extend(["-Y", filter]);
        let hits = tshark(file, &args);
        assert!(hits.trim().is_empty(), "filter {filter}: {hits}");
    }
}

mod tls {
    use super::*;
    use fps_module_tls::{
        ClientProfile, LeafKey, NssKeyLog, RunAuthority, ServerProfile, TlsAcceptor, TlsConnector,
        TlsVersion,
    };

    const URI: &str = "/fps-capture-tls";

    /// Runs one HTTP exchange over TLS with the tap below the TLS layer, and
    /// hands the session's key log to the recorder.
    async fn tls_session(capture: &Capture, id: u64, version: TlsVersion) {
        let keys = Arc::new(NssKeyLog::in_memory());
        let ca = RunAuthority::generate("fps capture authority").unwrap();
        let identity = ca.issue(&["tls.example.test"], LeafKey::EcdsaP256).unwrap();
        let client = ClientProfile {
            versions: vec![version],
            alpn: vec![b"http/1.1".to_vec()],
            key_log: Some(keys.clone()),
            ..ClientProfile::default()
        };
        let server = ServerProfile {
            versions: vec![version],
            key_log: Some(keys.clone()),
            ..ServerProfile::default()
        };
        let connector = TlsConnector::new(&client, ca.root_store().unwrap()).unwrap();
        let acceptor = TlsAcceptor::new(&server, &identity).unwrap();

        let (c, s) = duplex(64 * 1024);
        let c = capture.tap(id, v4(41_000 + id as u16, 443), Side::Client, c);
        let handle = c.handle().cloned().expect("the sample takes every session");
        let (c, s) = tokio::join!(connector.connect(c, "tls.example.test"), acceptor.accept(s));
        let (mut c, mut s) = (c.unwrap(), s.unwrap());
        let request = format!("GET {URI}-{id} HTTP/1.1\r\nHost: tls.example.test\r\n\r\n");
        let serve = async {
            let mut buf = vec![0u8; request.len()];
            s.read_exact(&mut buf).await.unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nhello tshark")
                .await
                .unwrap();
            s.shutdown().await.unwrap();
        };
        let ask = async {
            c.write_all(request.as_bytes()).await.unwrap();
            let mut reply = Vec::new();
            c.read_to_end(&mut reply).await.unwrap();
        };
        tokio::join!(serve, ask);
        handle.add_key_lines(&keys.contents());
        drop(c);
    }

    async fn decrypts(mode: KeyLogMode, version: TlsVersion) {
        let dir = fresh_dir(&format!("tshark-tls-{mode:?}-{version:?}"));
        let mut cfg = config(&dir, SamplePolicy::Rate(1.0));
        cfg.key_log = mode;
        let capture = start(cfg);
        for id in 0..3 {
            tls_session(&capture, id, version).await;
        }
        let files = capture.flush();
        capture.stop();
        let file = &files[0];
        let sidecar = format!("{}.keys", file.display());
        let option = format!("tls.keylog_file:{sidecar}");
        let args: Vec<&str> = match mode {
            // The embedded secrets must be enough on their own.
            KeyLogMode::Embedded | KeyLogMode::Both => vec![],
            KeyLogMode::Sidecar => vec!["-o", &option],
        };
        let mut all = args.clone();
        all.extend([
            "-Y",
            "http.request",
            "-T",
            "fields",
            "-e",
            "http.request.uri",
        ]);
        let uris = tshark(file, &all);
        for id in 0..3 {
            assert!(
                uris.contains(&format!("{URI}-{id}")),
                "{mode:?} {version:?}: session {id} decrypts, got {uris:?}"
            );
        }
        // The sidecar alone also decrypts a capture without embedded secrets.
        if mode == KeyLogMode::Both {
            let mut with_sidecar = vec!["-o", &option];
            with_sidecar.extend([
                "-Y",
                "http.response.code == 200",
                "-T",
                "fields",
                "-e",
                "http.response.code",
            ]);
            assert_eq!(tshark(file, &with_sidecar).lines().count(), 3);
        }
    }

    #[tokio::test]
    async fn tshark_decrypts_with_embedded_secrets() {
        if skip_without("tshark", tool_available("tshark", &["--version"])) {
            return;
        }
        decrypts(KeyLogMode::Embedded, TlsVersion::Tls13).await;
        decrypts(KeyLogMode::Both, TlsVersion::Tls12).await;
    }

    #[tokio::test]
    async fn tshark_decrypts_with_the_sidecar_key_log() {
        if skip_without("tshark", tool_available("tshark", &["--version"])) {
            return;
        }
        decrypts(KeyLogMode::Sidecar, TlsVersion::Tls13).await;
    }

    /// Without a tool, the key lines still are the ones that rustls logged.
    #[tokio::test]
    async fn the_ring_holds_the_nss_lines_of_every_tls_session() {
        let dir = fresh_dir("tls-lines");
        let capture = start(config(&dir, SamplePolicy::Rate(1.0)));
        for id in 0..2 {
            tls_session(&capture, id, TlsVersion::Tls13).await;
        }
        let files = capture.flush();
        capture.stop();
        let parsed = read_pcapng(&files[0]);
        assert_eq!(parsed.secrets.len(), 2);
        for (kind, data) in &parsed.secrets {
            assert_eq!(*kind, 0x544c_534b);
            let text = String::from_utf8(data.clone()).unwrap();
            assert!(
                text.lines()
                    .any(|l| l.starts_with("CLIENT_TRAFFIC_SECRET_0 ")),
                "{text}"
            );
            assert!(text.lines().all(|l| l.split(' ').count() == 3));
        }
        // The packets of a TLS 1.3 session carry an encrypted record stream.
        let conns = reassemble(&packets(&parsed), 443);
        assert_eq!(conns.len(), 2);
        assert!(conns
            .values()
            .all(|c| c.to_server.first() == Some(&22) && c.fins == 2 && c.packets >= 8));
        assert!(
            conns
                .values()
                .all(|c| !c.to_client.windows(4).any(|w| w == b"HTTP")),
            "the capture is encrypted"
        );
    }
}
