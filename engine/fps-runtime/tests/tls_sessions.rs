// SPDX-License-Identifier: GPL-3.0-only
//! TLS per session connectors, instance TLS, and the `tls.*` metrics (ENG-24).
//!
//! * Three consecutive sessions of a Chrome profile run against a server that
//!   issues tickets. A recording relay between the two runtimes keeps the first
//!   bytes of every connection, and `realism/ja4.py` computes the JA4 of each
//!   ClientHello. With a connector factory the three fingerprints are the
//!   `expected_ja4` of the profile. With one shared connector (`with_client`)
//!   the later sessions resume, and their ClientHello differs, which shows that
//!   the first check can fail.
//! * A flow that a `spawn` starts upgrades its transport to TLS, so the flows
//!   of a scenario instance have an upgrader.
//! * The handshakes appear in the snapshot of the run as the `tls.*` series of
//!   the catalog, with the `port` and `side` labels, and a failed handshake
//!   counts with its alerts.
//!
//! The JA4 check needs `python3`. Run it with
//! `cargo test -p fps-runtime --test tls_sessions`.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_flow::Scenario;
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_interp::Side;
use fps_module_tls::browser::BROWSER_PROFILES;
use fps_module_tls::{LeafKey, RunAuthority, ServerProfile, TlsAcceptor, TlsConnector, TlsVersion};
use fps_runtime::{
    ClientConfig, FlowRef, Role, RunReport, Runtime, RuntimeConfig, ServerConfig, SourceHosts,
    TlsProfiles, WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::{metric, MetricId, SeriesValue, Snapshot, SnapshotMeta, Timestamp};

const SEED: u64 = 20_261_112;
const SESSIONS: u64 = 3;
const SERVER_NAME: &str = "www.example.test";

/// One TLS session over the connection. The ClientHello is the first thing on
/// the wire, and the exchange after the handshake makes the client read the
/// session tickets of the server.
const HTTPS: &str = r#"
scenario: tls_ping
flows:
  - id: main
    module: t
    client:
      - connect: { port: 9000 }
      - tls_upgrade: { profile: chrome, server_name: www.example.test }
      - send: "ping\r\n"
      - expect: { pattern: "pong\r\n", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: 9000 }
      - tls_upgrade: { profile: site }
      - expect: { pattern: "ping\r\n", timeout_ms: 5000 }
      - send: "pong\r\n"
      - close: fin
"#;

/// The control flow spawns a data flow, whose connection runs over TLS.
const SPAWNED: &str = r#"
scenario: tls_data
vars: { data_port: 52700 }
flows:
  - id: control
    module: t
    client:
      - connect: { port: 9000 }
      - expect: { pattern: "go\r\n", timeout_ms: 5000 }
      - spawn: { flow: data }
      - expect: { pattern: "done\r\n", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: 9000 }
      - send: "go\r\n"
      - spawn: { flow: data }
      - send: "done\r\n"
      - close: fin
  - id: data
    module: t
    spawned: true
    client:
      - connect: { port: "{{ vars.data_port }}" }
      - tls_upgrade: { profile: chrome, server_name: www.example.test }
      - send: "hello\r\n"
      - expect: { pattern: "world\r\n", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: "{{ vars.data_port }}" }
      - tls_upgrade: { profile: site }
      - expect: { pattern: "hello\r\n", timeout_ms: 5000 }
      - send: "world\r\n"
      - close: fin
"#;

fn compile(text: &str) -> Arc<Scenario> {
    let mut s = Scenario::parse_instance(text, None).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

/// How the client registers its Chrome profile.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Client {
    /// One connector per session, from a factory.
    Factory,
    /// One connector for the run.
    Shared,
    /// A factory over roots of another authority, which no server satisfies.
    Untrusted,
}

/// The profiles of a run: a ticket issuing server, and a Chrome client.
fn profiles(client: Client) -> (Arc<TlsProfiles>, Arc<TlsProfiles>) {
    let ca = RunAuthority::generate("fps eng-24 authority").unwrap();
    let identity = ca.issue(&[SERVER_NAME], LeafKey::EcdsaP256).unwrap();
    let server = ServerProfile {
        name: "site".into(),
        versions: vec![TlsVersion::Tls13],
        ..ServerProfile::default()
    };
    let acceptor = TlsAcceptor::new(&server, &identity).unwrap();
    let chrome = BROWSER_PROFILES[0].client_profile();
    let roots = match client {
        Client::Untrusted => RunAuthority::generate("another authority")
            .unwrap()
            .root_store()
            .unwrap(),
        _ => ca.root_store().unwrap(),
    };
    let tls = match client {
        Client::Shared => {
            TlsProfiles::new().with_client("chrome", TlsConnector::new(&chrome, roots).unwrap())
        }
        _ => TlsProfiles::new()
            .with_client_factory("chrome", move || TlsConnector::new(&chrome, roots.clone())),
    };
    (
        Arc::new(tls),
        Arc::new(TlsProfiles::new().with_server("site", acceptor)),
    )
}

fn start_server(scenario: &Arc<Scenario>, ip: IpAddr, tls: Arc<TlsProfiles>) -> RuntimeHandle {
    let mut flow = FlowRef::new(Arc::clone(scenario), 0);
    flow.port = Some("https".into());
    let worker = WorkerConfig {
        id: 10,
        core: None,
        role: Role::Server(ServerConfig {
            flow,
            listen: vec![SocketAddr::new(ip, 0)],
            config: ListenConfig::default(),
        }),
    };
    Runtime::start(RuntimeConfig::new(SEED, vec![worker]).with_tls(tls)).unwrap()
}

type RuntimeHandle = fps_runtime::RuntimeHandle;

/// A client worker that starts [`SESSIONS`] sessions in one second, from
/// sources in `127.<pool>.0.0/24`.
fn client_config(
    scenario: &Arc<Scenario>,
    server: SocketAddr,
    pool: u8,
    tls: Arc<TlsProfiles>,
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
        Duration::from_secs(1),
        Shape::Hold,
        1.0,
        1.0,
    )])
    .unwrap();
    let mut schedule =
        ScheduleConfig::new(Objective::SessionsPerSecond(SESSIONS as f64), profile, SEED);
    schedule.arrival = Arrival::Constant;
    schedule.stream = stream_id(0, 0);
    let mut flow = FlowRef::new(Arc::clone(scenario), 0);
    flow.port = Some("https".into());
    let worker = WorkerConfig {
        id: 0,
        core: None,
        role: Role::Client(Box::new(ClientConfig {
            flow,
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
    let mut config = RuntimeConfig::new(SEED, vec![worker]).with_tls(tls);
    config.snapshot_interval = Duration::from_millis(100);
    config
}

/// Runs the client against `server` and returns both reports.
fn run_pair(server: RuntimeHandle, client: RuntimeConfig) -> (RunReport, RunReport) {
    let client = Runtime::start(client).unwrap().join();
    server.stop(Duration::from_secs(2));
    let server = server.join();
    for report in [&client, &server] {
        for (id, r) in &report.workers {
            assert!(r.is_ok(), "worker {id}: {r:?}");
        }
    }
    (client, server)
}

// The relay that records the ClientHello of every connection. -----------------

type Hellos = Arc<Mutex<Vec<Vec<u8>>>>;

/// Forwards every connection to `server` and keeps the first TLS record that
/// the client sends, in the order of the connections. The threads end with
/// the process.
fn relay(server: SocketAddr) -> (SocketAddr, Hellos) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let hellos: Hellos = Arc::default();
    let log = Arc::clone(&hellos);
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let Ok(mut client) = client else { break };
            let Ok(mut upstream) = TcpStream::connect(server) else {
                continue;
            };
            let slot = {
                let mut log = log.lock().unwrap();
                log.push(Vec::new());
                log.len() - 1
            };
            let (mut c2, mut u2) = (client.try_clone().unwrap(), upstream.try_clone().unwrap());
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = client.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    {
                        let mut log = log.lock().unwrap();
                        let hello = &mut log[slot];
                        let complete = hello.len() >= 5
                            && hello.len()
                                >= 5 + usize::from(u16::from_be_bytes([hello[3], hello[4]]));
                        if !complete {
                            hello.extend_from_slice(&buf[..n]);
                        }
                    }
                    if upstream.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                let _ = upstream.shutdown(std::net::Shutdown::Write);
            });
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = u2.read(&mut buf) {
                    if n == 0 || c2.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                let _ = c2.shutdown(std::net::Shutdown::Write);
            });
        }
    });
    (addr, hellos)
}

/// The JA4 of a ClientHello, which `realism/ja4.py` computes from its bytes.
fn ja4(name: &str, hello: &[u8]) -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = manifest.join("../../modules/tls/tests/ja4_of_hello.py");
    let root = manifest.join("../..");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("eng24-{name}.hello"));
    std::fs::write(&file, hello).unwrap();
    let out = Command::new("python3")
        .arg("-I")
        .arg(&script)
        .arg(&root)
        .arg(&file)
        .output()
        .expect("python3 starts");
    assert!(
        out.status.success(),
        "{name}: ja4_of_hello.py failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("ja4="))
        .unwrap_or_else(|| panic!("{name}: no ja4 line"))
        .to_owned()
}

/// Runs [`SESSIONS`] sessions of the Chrome profile through the relay and
/// returns the JA4 of every ClientHello and both reports.
fn fingerprints(client: Client, pool: u8) -> (Vec<String>, RunReport, RunReport) {
    let scenario = compile(HTTPS);
    let (client_tls, server_tls) = profiles(client);
    let server = start_server(&scenario, v4(pool, 0, 1), server_tls);
    let (relay_addr, hellos) = relay(server.listen_addrs()[0]);
    let config = client_config(&scenario, relay_addr, pool + 1, client_tls);
    let (client_report, server_report) = run_pair(server, config);
    let hellos = hellos.lock().unwrap().clone();
    assert_eq!(hellos.len() as u64, SESSIONS, "one connection per session");
    let prints = hellos
        .iter()
        .enumerate()
        .map(|(i, h)| ja4(&format!("{pool}-{i}"), h))
        .collect();
    (prints, client_report, server_report)
}

// Reading the snapshot. -------------------------------------------------------

/// The sum of the counters of `metric` whose labels include all of `labels`.
fn counted(snapshot: &Snapshot, metric: MetricId, labels: &[(&str, &str)]) -> u64 {
    snapshot
        .iter()
        .filter(|(k, _)| {
            k.metric() == metric
                && labels
                    .iter()
                    .all(|(l, v)| k.labels().iter().any(|(kl, kv)| kl == l && kv == v))
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(c) => *c,
            _ => 0,
        })
        .sum()
}

/// The number of samples of the histogram `metric` with `labels`.
fn samples(snapshot: &Snapshot, metric: MetricId, labels: &[(&str, &str)]) -> u64 {
    snapshot
        .iter()
        .filter(|(k, _)| {
            k.metric() == metric
                && labels
                    .iter()
                    .all(|(l, v)| k.labels().iter().any(|(kl, kv)| kl == l && kv == v))
        })
        .map(|(_, v)| match v {
            SeriesValue::Histogram(h) => h.len(),
            _ => 0,
        })
        .sum()
}

fn label(side: Side) -> &'static str {
    match side {
        Side::Client => "client",
        Side::Server => "server",
    }
}

fn total(report: &RunReport, side: Side) -> Snapshot {
    report.collector.total(side).unwrap()
}

#[test]
fn three_sessions_keep_the_ja4_of_their_profile_against_a_ticket_server() {
    let (prints, client, server) = fingerprints(Client::Factory, 70);
    let expected = BROWSER_PROFILES[0].expected_ja4;
    for (i, print) in prints.iter().enumerate() {
        assert_eq!(print, expected, "session {i}: {prints:?}");
    }

    let c = total(&client, Side::Client);
    let s = total(&server, Side::Server);
    assert_eq!(c.counter_total(metric::SESSIONS_SUCCEEDED), SESSIONS);
    assert_eq!(s.counter_total(metric::SESSIONS_SUCCEEDED), SESSIONS);
    // No session resumed.
    let l = [("port", "https"), ("side", "client")];
    assert_eq!(counted(&c, metric::TLS_HANDSHAKES, &l), SESSIONS);
    assert_eq!(counted(&c, metric::TLS_RESUMPTIONS, &l), 0);
    assert_eq!(counted(&c, metric::TLS_HANDSHAKE_FAILURES, &l), 0);
    assert_eq!(samples(&c, metric::TLS_HANDSHAKE_TIME, &l), SESSIONS);
    let l = [("port", "https"), ("side", "server")];
    assert_eq!(counted(&s, metric::TLS_HANDSHAKES, &l), SESSIONS);
    assert_eq!(counted(&s, metric::TLS_RESUMPTIONS, &l), 0);
}

#[test]
fn a_shared_connector_resumes_and_changes_the_client_hello() {
    let (prints, client, _server) = fingerprints(Client::Shared, 72);
    // The first session is a full handshake. Every later one resumes: its
    // ClientHello carries a pre_shared_key extension, so its JA4 differs.
    assert_ne!(prints[0], prints[1], "{prints:?}");
    assert_ne!(prints[0], prints[2], "{prints:?}");
    let c = total(&client, Side::Client);
    let l = [("port", "https"), ("side", "client")];
    assert_eq!(counted(&c, metric::TLS_HANDSHAKES, &l), SESSIONS);
    assert_eq!(counted(&c, metric::TLS_RESUMPTIONS, &l), SESSIONS - 1);
}

#[test]
fn the_tls_metrics_reach_the_run_snapshot() {
    let (_, client, server) = fingerprints(Client::Factory, 74);
    let meta = SnapshotMeta {
        time: Timestamp {
            seconds: 1_793_675_400,
            nanos: 0,
        },
        ..SnapshotMeta::default()
    };
    for (report, side) in [(&client, Side::Client), (&server, Side::Server)] {
        let json = total(report, side).to_proto_json(&meta).unwrap();
        let expected = SESSIONS.to_string();
        let counters = json["counters"].as_array().unwrap();
        let found = counters.iter().any(|c| {
            c["metric"] == "tls.handshakes"
                && c["labels"]["port"] == "https"
                && c["labels"]["side"] == label(side)
                && c["count"].as_str() == Some(expected.as_str())
        });
        assert!(found, "{side:?}: no tls.handshakes in {counters:?}");
        let histograms = json["histograms"].as_array().unwrap();
        assert!(
            histograms
                .iter()
                .any(|h| h["metric"] == "tls.handshake_time" && h["labels"]["side"] == label(side)),
            "{side:?}: no tls.handshake_time"
        );
    }
}

#[test]
fn a_failed_handshake_counts_with_its_alerts() {
    let scenario = compile(HTTPS);
    let (client_tls, server_tls) = profiles(Client::Untrusted);
    let server = start_server(&scenario, v4(76, 0, 1), server_tls);
    let addr = server.listen_addrs()[0];
    let (client, server) = run_pair(server, client_config(&scenario, addr, 77, client_tls));
    let c = total(&client, Side::Client);
    let s = total(&server, Side::Server);
    let l = [("port", "https"), ("side", "client")];
    assert_eq!(counted(&c, metric::TLS_HANDSHAKES, &l), 0);
    assert_eq!(counted(&c, metric::TLS_HANDSHAKE_FAILURES, &l), SESSIONS);
    // The client rejects the certificate and tells the server so.
    assert_eq!(
        counted(&c, metric::TLS_ALERTS_TX, &[("alert", "unknown_ca")]),
        SESSIONS
    );
    let l = [("port", "https"), ("side", "server")];
    assert_eq!(counted(&s, metric::TLS_HANDSHAKE_FAILURES, &l), SESSIONS);
    assert_eq!(
        counted(&s, metric::TLS_ALERTS_RX, &[("alert", "unknown_ca")]),
        SESSIONS
    );
}

#[test]
fn a_spawned_flow_upgrades_to_tls() {
    let scenario = compile(SPAWNED);
    let (client_tls, server_tls) = profiles(Client::Factory);
    let server = start_server(&scenario, v4(78, 0, 1), server_tls);
    let addr = server.listen_addrs()[0];
    let (client, server) = run_pair(server, client_config(&scenario, addr, 79, client_tls));
    for (report, side) in [(&client, Side::Client), (&server, Side::Server)] {
        let s = total(report, side);
        // The control and the data flow of every instance count as sessions.
        assert_eq!(s.counter_total(metric::SESSIONS_SUCCEEDED), 2 * SESSIONS);
        assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0);
        // Only the data flow runs over TLS.
        let l = [("port", "https"), ("side", label(side))];
        assert_eq!(counted(&s, metric::TLS_HANDSHAKES, &l), SESSIONS);
        assert_eq!(counted(&s, metric::TLS_HANDSHAKE_FAILURES, &l), 0);
    }
}
