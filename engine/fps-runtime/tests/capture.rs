// SPDX-License-Identifier: GPL-3.0-only
//! Capture of the TCP sessions of the worker runtime (ENG-19).
//!
//! A capture starts in the middle of a run through the cell of the runtime
//! handle. The tests read the ring back with the independent pcapng reader of
//! the `fps-capture` tests, and let tshark parse it where it is installed. The
//! tshark part skips itself when tshark is missing, unless the environment
//! variable `FPS_REQUIRE_CAPTURE_TOOLS` is `1`, which a CI job that installs
//! tshark sets so that a missing tool fails. Run it with
//! `cargo test -p fps-runtime --test capture`.

#[path = "../../fps-capture/tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{
    fresh_dir, packets, read_pcapng, reassemble, skip_without, tool_available, Conn, Packet,
    StepClock,
};
use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_capture::{Capture, CaptureConfig, RingConfig, SamplePolicy};
use fps_flow::Scenario;
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_interp::Side;
use fps_runtime::{
    ClientConfig, FlowRef, Role, Runtime, RuntimeConfig, RuntimeHandle, ServerConfig, SourceHosts,
    WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::metric;

const SEED: u64 = 20_261_109;
/// The requests of one `web_get` session.
const REQUESTS: usize = 4;

fn web_get() -> Arc<Scenario> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/modules/http1/scenarios/web_get.yaml");
    let mut s = Scenario::load(&path).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

/// One server worker on a kernel chosen port of `ip`, whose runtime reads
/// `config.capture`.
fn start_server(scenario: &Arc<Scenario>, ip: IpAddr, capture: Option<Capture>) -> RuntimeHandle {
    let worker = WorkerConfig {
        id: 10,
        core: None,
        role: Role::Server(ServerConfig {
            flow: FlowRef::new(Arc::clone(scenario), 0),
            listen: vec![SocketAddr::new(ip, 0)],
            config: ListenConfig::default(),
        }),
    };
    let config = RuntimeConfig::new(SEED, vec![worker]);
    if let Some(capture) = capture {
        config.capture.set(capture);
    }
    Runtime::start(config).unwrap()
}

/// One client worker that starts `rate` sessions per second for `secs`
/// seconds from sources in `127.<pool>.0.0/24`.
fn client_config(
    scenario: &Arc<Scenario>,
    server: SocketAddr,
    pool: u8,
    rate: f64,
    secs: u64,
) -> RuntimeConfig {
    let prefix = format!("127.{pool}.0.0/24");
    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some(&prefix),
        count: 16,
        ..PoolSpec::default()
    })
    .unwrap();
    let range = hosts.range();
    let profile = LoadProfile::new(vec![Phase::new(
        "steady",
        Duration::from_secs(secs),
        Shape::Hold,
        1.0,
        1.0,
    )])
    .unwrap();
    let mut schedule = ScheduleConfig::new(Objective::SessionsPerSecond(rate), profile, SEED);
    schedule.arrival = Arrival::Constant;
    schedule.stream = stream_id(0, 0);
    let worker = WorkerConfig {
        id: 0,
        core: None,
        role: Role::Client(Box::new(ClientConfig {
            flow: FlowRef::new(Arc::clone(scenario), 0),
            schedule,
            servers: vec![server.ip()],
            port: Some(server.port()),
            sources: SourceHosts {
                selector: HostSelector::new("clients", range, Selection::RoundRobin, SEED, 0)
                    .unwrap(),
                pool: hosts,
                ipv6: false,
            },
            connect: ConnectConfig::default(),
        })),
    };
    let mut config = RuntimeConfig::new(SEED, vec![worker]);
    config.snapshot_interval = Duration::from_millis(100);
    config
}

fn start_capture(name: &str) -> (Capture, PathBuf) {
    let dir = fresh_dir(name);
    let mut ring = RingConfig::new(&dir);
    ring.prefix = "rt".into();
    let mut config = CaptureConfig::new(ring);
    config.sample = SamplePolicy::EveryNth(1);
    config.seed = SEED;
    (
        Capture::start(config, Arc::new(StepClock::default())).unwrap(),
        dir,
    )
}

/// Reassembles every TCP connection of the packets that go to `server`,
/// keyed by the client address, because two source hosts may share a port.
fn connections(packets: &[Packet], server: SocketAddr) -> BTreeMap<(IpAddr, u16), Conn> {
    let mut by_host: BTreeMap<IpAddr, Vec<Packet>> = BTreeMap::new();
    for p in packets.iter().filter(|p| !p.udp) {
        assert!(
            (p.dst == server.ip() && p.dport == server.port())
                || (p.src == server.ip() && p.sport == server.port()),
            "every packet belongs to the server {server}: {p:?}"
        );
        let client = if p.dport == server.port() {
            p.src
        } else {
            p.dst
        };
        by_host.entry(client).or_default().push(p.clone());
    }
    let mut out = BTreeMap::new();
    for (host, list) in by_host {
        for (port, conn) in reassemble(&list, server.port()) {
            out.insert((host, port), conn);
        }
    }
    out
}

/// Checks that a captured connection is one complete `web_get` session.
fn assert_web_get(key: &(IpAddr, u16), c: &Conn) {
    assert!(c.saw_syn && c.saw_syn_ack, "{key:?}: handshake");
    assert!(!c.rst, "{key:?}: no reset");
    assert_eq!(c.fins, 2, "{key:?}: both sides close with FIN");
    let request = "GET /index.html HTTP/1.1\r\nHost: www.example.test\r\n";
    let text = String::from_utf8_lossy(&c.to_server);
    assert_eq!(text.matches(request).count(), REQUESTS, "{key:?}: {text}");
    let response = String::from_utf8_lossy(&c.to_client);
    assert_eq!(
        response.matches("HTTP/1.1 200 OK\r\n").count(),
        REQUESTS,
        "{key:?}"
    );
}

fn tshark(file: &Path, args: &[&str]) -> String {
    let out = Command::new("tshark")
        .arg("-r")
        .arg(file)
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

#[test]
fn a_capture_started_mid_run_holds_the_sampled_client_sessions() {
    let scenario = web_get();
    let server = start_server(&scenario, v4(50, 0, 1), None);
    let addr = server.listen_addrs()[0];
    let mut client = Runtime::start(client_config(&scenario, addr, 51, 20.0, 3)).unwrap();

    // Let about a third of the sessions open without a tap, then start the
    // capture through the cell of the handle.
    std::thread::sleep(Duration::from_millis(1_000));
    let before = client
        .collect()
        .total(Side::Client)
        .map_or(0, |s| s.counter_total(metric::SESSIONS_ATTEMPTED));
    assert!(before > 0, "sessions opened before the capture");
    let (capture, _dir) = start_capture("runtime-client");
    client.capture_cell().set(capture.clone());

    let started = Instant::now();
    let report = client.join();
    assert!(started.elapsed() < Duration::from_secs(10));
    server.stop(Duration::from_secs(1));
    let _ = server.join();
    let attempted = report
        .collector
        .total(Side::Client)
        .unwrap()
        .counter_total(metric::SESSIONS_ATTEMPTED);
    assert_eq!(attempted, 60, "20 per second for 3 seconds");

    let files = capture.flush();
    let stats = capture.stop();
    assert_eq!(stats.sessions_dropped_queue, 0, "{stats:?}");
    assert_eq!(files.len(), 1, "{files:?}");
    let packets = packets(&read_pcapng(&files[0]));
    let conns = connections(&packets, addr);

    // Sessions that opened before the capture are not in the ring, and the
    // ones that opened after it all are, each complete.
    let captured = conns.len() as u64;
    assert_eq!(stats.sessions_written, captured, "{stats:?}");
    assert!(captured >= 10, "{captured} sessions captured");
    assert!(
        captured <= attempted - before,
        "{captured} captured, {attempted} started, {before} before the capture"
    );
    assert!(captured < attempted, "the capture started mid run");
    for (key, c) in &conns {
        assert_web_get(key, c);
        assert_eq!(key.0.to_string().split('.').nth(1), Some("51"));
    }

    if skip_without("tshark", tool_available("tshark", &["--version"])) {
        return;
    }
    let decode = format!("tcp.port=={},http", addr.port());
    let uris = tshark(
        &files[0],
        &[
            "-d",
            &decode,
            "-Y",
            "http.request",
            "-T",
            "fields",
            "-e",
            "http.request.uri",
        ],
    );
    assert_eq!(
        uris.lines().filter(|l| *l == "/index.html").count(),
        conns.len() * REQUESTS,
        "tshark finds every request: {uris:?}"
    );
    let codes = tshark(
        &files[0],
        &[
            "-d",
            &decode,
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
        conns.len() * REQUESTS,
        "tshark finds every response"
    );
    let streams = tshark(
        &files[0],
        &["-T", "fields", "-e", "tcp.stream", "-Y", "tcp"],
    );
    assert_eq!(
        streams
            .lines()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        conns.len(),
        "one tshark conversation per captured session"
    );
    let strict = [
        "-o",
        "ip.check_checksum:TRUE",
        "-o",
        "tcp.check_checksum:TRUE",
    ];
    for filter in [
        "_ws.expert.group == 0x01000000",
        "_ws.malformed",
        "tcp.analysis.retransmission || tcp.analysis.lost_segment || tcp.analysis.out_of_order",
    ] {
        let mut args = strict.to_vec();
        args.extend(["-Y", filter]);
        let hits = tshark(&files[0], &args);
        assert!(hits.trim().is_empty(), "filter {filter}: {hits}");
    }
}

#[test]
fn a_server_worker_taps_the_sessions_it_accepts() {
    let scenario = web_get();
    let (capture, _dir) = start_capture("runtime-server");
    let server = start_server(&scenario, v4(52, 0, 1), Some(capture.clone()));
    let addr = server.listen_addrs()[0];
    let client = Runtime::start(client_config(&scenario, addr, 53, 20.0, 1)).unwrap();
    let report = client.join();
    server.stop(Duration::from_secs(1));
    let _ = server.join();
    assert_eq!(
        report
            .collector
            .total(Side::Client)
            .unwrap()
            .counter_total(metric::SESSIONS_SUCCEEDED),
        20
    );
    let files = capture.flush();
    let stats = capture.stop();
    assert_eq!(files.len(), 1);
    let conns = connections(&packets(&read_pcapng(&files[0])), addr);
    assert_eq!(conns.len(), 20, "every accepted session, {stats:?}");
    for (key, c) in &conns {
        assert_web_get(key, c);
    }
}

#[test]
fn an_empty_cell_taps_nothing_and_a_cleared_cell_stops_the_tap() {
    let scenario = web_get();
    let (capture, _dir) = start_capture("runtime-cleared");
    let server = start_server(&scenario, v4(54, 0, 1), None);
    let addr = server.listen_addrs()[0];
    let config = client_config(&scenario, addr, 55, 20.0, 1);
    config.capture.set(capture.clone());
    config.capture.clear();
    let client = Runtime::start(config).unwrap();
    let report = client.join();
    server.stop(Duration::from_secs(1));
    let _ = server.join();
    assert_eq!(
        report
            .collector
            .total(Side::Client)
            .unwrap()
            .counter_total(metric::SESSIONS_SUCCEEDED),
        20
    );
    let stats = capture.stop();
    assert_eq!(stats.sessions_sampled, 0, "{stats:?}");
}
