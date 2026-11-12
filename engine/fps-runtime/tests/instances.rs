// SPDX-License-Identifier: GPL-3.0-only
//! Multi flow scenario instances in the worker runtime (ENG-23).
//!
//! The acceptance test runs `mixed_session` of the ftp module through a
//! client runtime and a server runtime over loopback, and counts the
//! connections twice: from the session counters of the workers, and from a
//! capture of the client sessions, which must show one control connection and
//! one data connection per transfer for every session. The other tests cover
//! the pieces around it: a data connection that dials before its listener
//! exists, the ordering of `after` flows, a failing flow, extensions in every
//! flow, and the configuration check.

#[path = "../../fps-capture/tests/common/mod.rs"]
mod capture_common;
#[path = "../../fps-wasm/tests/support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use capture_common::{fresh_dir, parse_frame, read_pcapng, StepClock, ACK, RST, SYN};
use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_capture::{Capture, CaptureConfig, RingConfig, SamplePolicy};
use fps_flow::{FnCall, FnError, FnErrorKind, Manifest, ModuleSet, Scenario, TemplateFn, Value};
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_module_content::Content;
use fps_module_ftp::Ftp;
use fps_runtime::{
    ClientConfig, ExtensionSet, FlowRef, FunctionFactory, Role, RunReport, Runtime, RuntimeConfig,
    RuntimeHandle, ServerConfig, SourceHosts, WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::{metric, SeriesValue, Snapshot};
use rand::RngCore;
use support::component_bytes;

const SEED: u64 = 20_261_111;

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

/// Routes the calls of the two modules that the ftp scenarios use.
#[derive(Default)]
struct Dispatch {
    ftp: Ftp,
    content: Content,
}

impl TemplateFn for Dispatch {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        match call.module {
            "ftp" => self.ftp.render(call, rng, out),
            "content" => self.content.render(call, rng, out),
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("no module provides '{other}.{}'", call.function),
            )),
        }
    }
}

fn ftp_functions() -> FunctionFactory {
    Arc::new(|| Box::new(Dispatch::default()))
}

/// Loads an ftp scenario as the module test does, with `vars` replaced.
fn ftp_scenario(name: &str, vars: &[(&str, Value)]) -> Arc<Scenario> {
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
    let mut set = ModuleSet::default();
    for m in ["ftp", "content"] {
        set.insert(Manifest::load(&module.join(m).join("manifest.yaml")).unwrap());
    }
    let ctx = set.context("ftp").unwrap();
    let path = module.join("ftp").join("scenarios").join(name);
    let mut s = Scenario::load_instance(&path, Some(&ctx)).unwrap();
    let own: Vec<(String, Value)> = vars
        .iter()
        .filter(|(k, _)| s.vars.contains_key(*k))
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    s.override_vars(&own).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn compile(text: &str) -> Arc<Scenario> {
    let mut s = Scenario::parse_instance(text, None).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn flow_ref(scenario: &Arc<Scenario>, functions: Option<FunctionFactory>) -> FlowRef {
    let mut f = FlowRef::new(Arc::clone(scenario), 0);
    if let Some(functions) = functions {
        f.functions = functions;
    }
    f
}

fn server_worker(flow: FlowRef, ip: IpAddr) -> WorkerConfig {
    WorkerConfig {
        id: 10,
        core: None,
        role: Role::Server(ServerConfig {
            flow,
            listen: vec![SocketAddr::new(ip, 0)],
            config: ListenConfig::default(),
        }),
    }
}

/// One client worker that starts `rate` sessions per second for `secs`
/// seconds from sources in `127.<pool>.0.0/24`, one source host per session.
fn client_config(
    flow: FlowRef,
    server: SocketAddr,
    pool: u8,
    rate: f64,
    secs: u64,
) -> RuntimeConfig {
    let prefix = format!("127.{pool}.0.0/24");
    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some(&prefix),
        count: 64,
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
    let mut config = RuntimeConfig::new(SEED, vec![worker]);
    config.snapshot_interval = Duration::from_millis(100);
    config
}

/// Runs `client` against `server` and returns both reports.
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

fn exit(report: &RunReport, worker: u32) -> &Snapshot {
    &report.workers[&worker].as_ref().unwrap().sessions
}

/// The sum of the counters of `metric` whose labels include `label`.
fn counted(snapshot: &Snapshot, metric: fps_stats::MetricId, label: (&str, &str)) -> u64 {
    snapshot
        .iter()
        .filter(|(k, _)| {
            k.metric() == metric
                && k.labels()
                    .iter()
                    .any(|(l, v)| *l == label.0 && v == label.1)
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(c) => *c,
            _ => 0,
        })
        .sum()
}

/// The failed sessions of a snapshot with their labels, so that an assertion
/// on a count says which flows failed and why. Empty when nothing failed.
fn failed_series(snapshot: &Snapshot) -> String {
    snapshot
        .iter()
        .filter(|(k, _)| k.metric() == metric::SESSIONS_FAILED)
        .filter_map(|(k, v)| match v {
            SeriesValue::Count(c) if *c > 0 => Some(format!("{:?} x{c}", k.labels())),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn start_capture(name: &str) -> Capture {
    let dir = fresh_dir(name);
    let mut ring = RingConfig::new(&dir);
    ring.prefix = "inst".into();
    let mut config = CaptureConfig::new(ring);
    config.sample = SamplePolicy::EveryNth(1);
    config.seed = SEED;
    Capture::start(config, Arc::new(StepClock::default())).unwrap()
}

/// The sessions that each run of `mixed_session` has: the control connection
/// and one data connection per transfer.
const FLOW_SESSIONS: u64 = 4;
const TRANSFERS: u64 = 3;
const INSTANCES: u64 = 8;

/// The acceptance test. `mixed_session` runs end to end through a client
/// worker and a server worker, and every instance has one data connection
/// per transfer, which the counters and the capture both show.
#[test]
fn mixed_session_runs_with_one_data_connection_per_transfer() {
    let server_ip = v4(60, 0, 1);
    let vars = [
        ("server_ip", Value::Str(server_ip.to_string())),
        ("data_port_min", Value::Int(52_000)),
        ("data_port_max", Value::Int(52_999)),
    ];
    let scenario = ftp_scenario("mixed_session.yaml", &vars);
    assert_eq!(scenario.flows.len(), 4);

    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(
            flow_ref(&scenario, Some(ftp_functions())),
            server_ip,
        )],
    ))
    .unwrap();
    let control = server.listen_addrs()[0];
    let config = client_config(
        flow_ref(&scenario, Some(ftp_functions())),
        control,
        61,
        INSTANCES as f64,
        1,
    );
    let capture = start_capture("instances-ftp");
    config.capture.set(capture.clone());
    let (client, server) = run_pair(server, config);

    // The counters: every instance started the control flow and three data
    // flows on both sides, and every side succeeded.
    let sessions = INSTANCES * FLOW_SESSIONS;
    for (report, worker, side) in [(&client, 0, "client"), (&server, 10, "server")] {
        let s = exit(report, worker);
        let side = ("side", side);
        assert_eq!(failed_series(s), "", "{side:?}");
        assert_eq!(
            counted(s, metric::SESSIONS_ATTEMPTED, side),
            sessions,
            "{side:?}"
        );
        assert_eq!(
            counted(s, metric::SESSIONS_ESTABLISHED, side),
            sessions,
            "{side:?}"
        );
        assert_eq!(
            counted(s, metric::SESSIONS_SUCCEEDED, side),
            sessions,
            "{side:?}"
        );
        assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0, "{side:?}");
        // The series carry the labels of the single flow sessions.
        assert_eq!(
            counted(s, metric::SESSIONS_ATTEMPTED, ("application", "ftp")),
            sessions
        );
        assert_eq!(
            counted(s, metric::SESSIONS_ATTEMPTED, ("scenario", "mixed_session")),
            sessions
        );
    }
    // The bytes of the three files cross the data connections in the
    // direction of the transfer, so the sides see matching totals.
    let (tx, rx) = (
        exit(&client, 0).counter_total(metric::BYTES_TX),
        exit(&server, 10).counter_total(metric::BYTES_RX),
    );
    assert_eq!(tx, rx, "what the client sent, the server received");
    assert!(tx > INSTANCES * 40_000, "{tx} bytes uploaded");

    // The capture: every session has one control connection and three data
    // connections from its own source host, and no connection resets.
    let files = capture.flush();
    let stats = capture.stop();
    assert_eq!(stats.sessions_dropped_queue, 0, "{stats:?}");
    assert_eq!(files.len(), 1, "{files:?}");
    // The ring writes a session when it ends, so the control connection
    // follows its data connections and the file is not in time order. The
    // parse checks the frames, and the test reads connections, not time.
    let packets: Vec<_> = read_pcapng(&files[0])
        .packets
        .iter()
        .map(|(ts, f)| parse_frame(*ts, f))
        .collect();
    assert!(packets.iter().all(|p| p.flags & RST == 0), "no reset");
    let mut dialed: BTreeMap<IpAddr, Vec<u16>> = BTreeMap::new();
    let mut connections = std::collections::BTreeSet::new();
    for p in packets
        .iter()
        .filter(|p| p.flags & SYN != 0 && p.flags & ACK == 0)
    {
        // A connection is the four tuple, not the source port. The client
        // sockets bind with IP_BIND_ADDRESS_NO_PORT, so the kernel picks the
        // port at connect time and gives a source host the same port for two
        // connections to different destination ports, such as the control
        // connection and a data connection of one instance.
        assert!(
            connections.insert((p.src, p.sport, p.dst, p.dport)),
            "one SYN per connection: {p:?}"
        );
        dialed.entry(p.src).or_default().push(p.dport);
    }
    assert_eq!(
        dialed.len() as u64,
        INSTANCES,
        "one source host per session"
    );
    assert_eq!(stats.sessions_written, sessions, "{stats:?}");
    for (host, dports) in &dialed {
        let to_control = dports.iter().filter(|p| **p == control.port()).count();
        let to_data = dports
            .iter()
            .filter(|p| (52_000..=52_999).contains(*p))
            .count();
        assert_eq!(to_control, 1, "{host}: one control connection: {dports:?}");
        assert_eq!(
            to_data as u64, TRANSFERS,
            "{host}: one data connection per transfer: {dports:?}"
        );
        let mut distinct = dports.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len() as u64, FLOW_SESSIONS, "{host}: {dports:?}");
    }
}

/// The client dials the data port 150 ms before the server binds it, and the
/// instance still succeeds, because the opener retries a refused connect.
const LATE: &str = r#"
scenario: late
vars: { data_port: 52500 }
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
      - delay: { dist: uniform, min_ms: 150, max_ms: 150 }
      - spawn: { flow: data }
      - send: "done\r\n"
      - close: fin
  - id: data
    module: t
    spawned: true
    client:
      - connect: { port: "{{ vars.data_port }}" }
      - send: "hello\r\n"
      - close: fin
    server:
      - accept: { port: "{{ vars.data_port }}" }
      - expect: { pattern: "hello\r\n", timeout_ms: 5000 }
      - close: fin
"#;

#[test]
fn a_data_connect_before_the_listener_is_retried() {
    let scenario = compile(LATE);
    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(flow_ref(&scenario, None), v4(62, 0, 1))],
    ))
    .unwrap();
    let addr = server.listen_addrs()[0];
    let config = client_config(flow_ref(&scenario, None), addr, 63, 1.0, 1);
    let (client, server) = run_pair(server, config);
    for (report, worker) in [(&client, 0), (&server, 10)] {
        let s = exit(report, worker);
        // Each of the two flow sessions counts.
        assert_eq!(s.counter_total(metric::SESSIONS_ATTEMPTED), 2);
        assert_eq!(s.counter_total(metric::SESSIONS_SUCCEEDED), 2);
        assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0);
    }
}

/// A flow with `after` starts when its predecessor completed, and the entry
/// connection is the only one that the port override redirects. Both flows
/// are not spawned, so the second flow dials its own port, which the server
/// binds when its flow starts.
const AFTER: &str = r#"
scenario: ordered
flows:
  - id: first
    module: t
    client:
      - connect: { port: 9000 }
      - send: "one\r\n"
      - expect: { pattern: "ok\r\n", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: 9000 }
      - expect: { pattern: "one\r\n", timeout_ms: 5000 }
      - send: "ok\r\n"
      - close: fin
  - id: second
    module: t
    after: [first]
    client:
      - connect: { port: 52510 }
      - send: "two\r\n"
      - expect: { pattern: "ok\r\n", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: 52510 }
      - expect: { pattern: "two\r\n", timeout_ms: 5000 }
      - send: "ok\r\n"
      - close: fin
"#;

#[test]
fn a_flow_with_after_runs_when_its_predecessor_is_done() {
    let scenario = compile(AFTER);
    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(flow_ref(&scenario, None), v4(64, 0, 1))],
    ))
    .unwrap();
    let addr = server.listen_addrs()[0];
    let config = client_config(flow_ref(&scenario, None), addr, 65, 2.0, 1);
    let (client, server) = run_pair(server, config);
    for (report, worker) in [(&client, 0), (&server, 10)] {
        let s = exit(report, worker);
        assert_eq!(s.counter_total(metric::SESSIONS_ATTEMPTED), 4);
        assert_eq!(s.counter_total(metric::SESSIONS_SUCCEEDED), 4);
        assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0);
    }
}

/// A data flow whose peer never comes fails the instance: the client side of
/// the data flow expects bytes that the server never sends, the instance
/// aborts the control flow, and every side counts under its own reason.
const STALLED: &str = r#"
scenario: stalled
flows:
  - id: control
    module: t
    client:
      - connect: { port: 9000 }
      - spawn: { flow: data }
      - expect: { pattern: "never\r\n", timeout_ms: 10000 }
    server:
      - accept: { port: 9000 }
      - spawn: { flow: data }
      - expect: { pattern: "never\r\n", timeout_ms: 10000 }
  - id: data
    module: t
    spawned: true
    client:
      - connect: { port: 52520 }
      - expect: { pattern: "never\r\n", timeout_ms: 300 }
    server:
      - accept: { port: 52520 }
      - expect: { pattern: "never\r\n", timeout_ms: 10000 }
"#;

#[test]
fn a_failing_flow_fails_the_instance_and_aborts_the_others() {
    let scenario = compile(STALLED);
    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(flow_ref(&scenario, None), v4(66, 0, 1))],
    ))
    .unwrap();
    let addr = server.listen_addrs()[0];
    let config = client_config(flow_ref(&scenario, None), addr, 67, 1.0, 1);
    let started = std::time::Instant::now();
    let (client, server) = run_pair(server, config);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the abort ends the sides"
    );
    let c = exit(&client, 0);
    assert_eq!(c.counter_total(metric::SESSIONS_ATTEMPTED), 2);
    assert_eq!(c.counter_total(metric::SESSIONS_SUCCEEDED), 0);
    assert_eq!(
        counted(c, metric::SESSIONS_FAILED, ("reason", "expect_timeout")),
        1
    );
    assert_eq!(
        counted(c, metric::SESSIONS_FAILED, ("reason", "aborted")),
        1
    );
    // The server sides end with the instance of their own worker: the data
    // flow stays quiet too, so they time out or abort when the client resets.
    let s = exit(&server, 10);
    assert_eq!(s.counter_total(metric::SESSIONS_ATTEMPTED), 2);
    assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 2);
    // The concurrent gauge returns to zero on both sides.
    for snapshot in [c, s] {
        for (k, v) in snapshot.iter() {
            if k.metric() == metric::SESSIONS_CONCURRENT {
                assert!(
                    matches!(v, SeriesValue::Gauge(g) if *g == 0.0),
                    "{k:?}: {v:?}"
                );
            }
        }
    }
}

/// Both flows call the extension, and the data flow of `SPIN` runs an endless
/// loop of it.
fn extension_scenario(spin: bool) -> Arc<Scenario> {
    let call = if spin { "t.spin()" } else { "t.echo('d')" };
    compile(&format!(
        r#"
scenario: ext
flows:
  - id: control
    module: t
    client:
      - connect: {{ port: 9000 }}
      - send: "{{{{ t.echo('c') }}}}\r\n"
      - spawn: {{ flow: data }}
      - expect: {{ pattern: "c\r\n", timeout_ms: 5000 }}
      - close: fin
    server:
      - accept: {{ port: 9000 }}
      - spawn: {{ flow: data }}
      - expect: {{ pattern: "c\r\n", timeout_ms: 5000 }}
      - send: "{{{{ t.echo('c') }}}}\r\n"
      - close: fin
  - id: data
    module: t
    spawned: true
    client:
      - connect: {{ port: 52530 }}
      - send: "{{{{ {call} }}}}\r\n"
      - close: fin
    server:
      - accept: {{ port: 52530 }}
      - expect: {{ pattern: "d\r\n", timeout_ms: 5000 }}
      - close: fin
"#
    ))
}

fn extension_flow(scenario: &Arc<Scenario>) -> FlowRef {
    let mut set = ExtensionSet::new().unwrap();
    set.load("t", &component_bytes(), fps_wasm::Limits::default(), None)
        .unwrap();
    let mut f = flow_ref(scenario, None);
    f.extensions = Some(Arc::new(set));
    f
}

#[test]
fn every_flow_of_an_instance_calls_its_extensions() {
    let good = extension_scenario(false);
    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(extension_flow(&good), v4(68, 0, 1))],
    ))
    .unwrap();
    let addr = server.listen_addrs()[0];
    let (client, server) = run_pair(
        server,
        client_config(extension_flow(&good), addr, 69, 2.0, 1),
    );
    for (report, worker) in [(&client, 0), (&server, 10)] {
        let s = exit(report, worker);
        assert_eq!(s.counter_total(metric::SESSIONS_ATTEMPTED), 4);
        assert_eq!(s.counter_total(metric::SESSIONS_SUCCEEDED), 4);
        assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0);
    }

    // The client of the data flow spins, so its side fails with
    // extension_error, which only a call from the data flow can cause.
    let spin = extension_scenario(true);
    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(extension_flow(&spin), v4(70, 0, 1))],
    ))
    .unwrap();
    let addr = server.listen_addrs()[0];
    let mut f = extension_flow(&spin);
    // A small fuel limit makes the loop fail fast.
    let mut set = ExtensionSet::new().unwrap();
    set.load(
        "t",
        &component_bytes(),
        fps_wasm::Limits {
            fuel_per_call: 200_000,
            ..fps_wasm::Limits::default()
        },
        None,
    )
    .unwrap();
    f.extensions = Some(Arc::new(set));
    let (client, _server) = run_pair(server, client_config(f, addr, 71, 1.0, 1));
    let c = exit(&client, 0);
    assert_eq!(
        counted(c, metric::SESSIONS_FAILED, ("reason", "extension_error")),
        1
    );
}

#[test]
fn the_config_check_accepts_a_udp_flow_and_refuses_a_spawned_entry_flow() {
    // A UDP flow among TCP flows is a flow that a worker opens (ENG-26).
    let udp = compile(
        r#"
scenario: mixed
flows:
  - id: control
    module: t
    client:
      - connect: { port: 9000 }
      - spawn: { flow: media }
    server:
      - accept: { port: 9000 }
  - id: media
    module: t
    l4: udp
    spawned: true
    client:
      - connect: { port: 9001 }
      - send: "x"
    server:
      - accept: { port: 9001 }
"#,
    );
    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(flow_ref(&udp, None), v4(72, 0, 1))],
    ))
    .unwrap();
    server.stop(Duration::from_millis(100));
    server.join();

    let scenario = compile(LATE);
    let mut spawned_entry = flow_ref(&scenario, None);
    spawned_entry.flow = 1;
    let err = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(spawned_entry, v4(72, 0, 2))],
    ))
    .unwrap_err();
    assert!(err.to_string().contains("spawned"), "{err}");
}

/// A free TCP port, which two concurrent instances then draw every time.
fn free_port() -> u16 {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Every instance draws the same data port for all of its transfers, so
/// concurrent instances meet one listener (AGT-18). Each instance dials from
/// its own source host and takes the connection of that host, and all of them
/// succeed.
#[test]
fn concurrent_instances_that_draw_the_same_data_port_all_succeed() {
    let server_ip = v4(64, 0, 1);
    let port = i64::from(free_port());
    let vars = [
        ("server_ip", Value::Str(server_ip.to_string())),
        ("data_port_min", Value::Int(port)),
        ("data_port_max", Value::Int(port)),
    ];
    let scenario = ftp_scenario("mixed_session.yaml", &vars);
    let server = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(
            flow_ref(&scenario, Some(ftp_functions())),
            server_ip,
        )],
    ))
    .unwrap();
    let control = server.listen_addrs()[0];
    // Eight instances start within one second, and each runs for at least
    // 120 ms of delays, so most of them overlap.
    let config = client_config(
        flow_ref(&scenario, Some(ftp_functions())),
        control,
        65,
        INSTANCES as f64,
        1,
    );
    let capture = start_capture("instances-same-port");
    config.capture.set(capture.clone());
    let (client, server) = run_pair(server, config);
    let sessions = INSTANCES * FLOW_SESSIONS;
    for (report, worker, side) in [(&client, 0, "client"), (&server, 10, "server")] {
        let s = exit(report, worker);
        let side = ("side", side);
        assert_eq!(failed_series(s), "", "{side:?}");
        assert_eq!(
            counted(s, metric::SESSIONS_SUCCEEDED, side),
            sessions,
            "{side:?}"
        );
        assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0, "{side:?}");
    }
    // The capture shows that every data connection went to the one port and
    // that none was reset.
    let files = capture.flush();
    let stats = capture.stop();
    assert_eq!(stats.sessions_dropped_queue, 0, "{stats:?}");
    let packets: Vec<_> = read_pcapng(&files[0])
        .packets
        .iter()
        .map(|(ts, f)| parse_frame(*ts, f))
        .collect();
    assert!(packets.iter().all(|p| p.flags & RST == 0), "no reset");
    let to_data = packets
        .iter()
        .filter(|p| p.flags & SYN != 0 && p.flags & ACK == 0 && i64::from(p.dport) == port)
        .count() as u64;
    assert_eq!(
        to_data,
        INSTANCES * TRANSFERS,
        "every transfer dialed the one port"
    );
}
