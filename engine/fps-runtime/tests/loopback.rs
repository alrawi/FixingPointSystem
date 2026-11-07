// SPDX-License-Identifier: GPL-3.0-only
//! Loopback component tests of the worker runtime over the kernel backend.
//!
//! Each test runs a server runtime and a client runtime in one process over
//! its own addresses in 127.0.0.0/8, so TIME_WAIT entries of one test never
//! collide with the sessions of another.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_flow::Scenario;
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_interp::{Limits, Side};
use fps_runtime::{
    ClientConfig, FlowRef, Role, RunReport, Runtime, RuntimeConfig, RuntimeHandle, ServerConfig,
    SourceHosts, WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::{metric, MetricId, Snapshot};

const SEED: u64 = 20_261_103;

fn web_get() -> Arc<Scenario> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/modules/http1/scenarios/web_get.yaml");
    let mut s = Scenario::load(&path).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn compile(text: &str) -> Arc<Scenario> {
    let mut s = Scenario::parse(text).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

/// Starts one server worker per address, each on a kernel chosen port.
fn start_servers(scenario: &Arc<Scenario>, addrs: &[IpAddr], first_id: u32) -> RuntimeHandle {
    let workers = addrs
        .iter()
        .zip(first_id..)
        .map(|(ip, id)| WorkerConfig {
            id,
            core: None,
            role: Role::Server(ServerConfig {
                flow: FlowRef::new(Arc::clone(scenario), 0),
                listen: vec![SocketAddr::new(*ip, 0)],
                config: ListenConfig::default(),
            }),
        })
        .collect();
    Runtime::start(RuntimeConfig::new(SEED, workers)).unwrap()
}

/// Returns client workers at a constant rate, one per server address, each
/// with its own range of a source pool in `127.<pool>.0.0/24`.
fn clients(
    scenario: &Arc<Scenario>,
    servers: &[SocketAddr],
    pool: u8,
    rate: f64,
    duration: Duration,
    limits: Limits,
) -> Vec<WorkerConfig> {
    let prefix = format!("127.{pool}.0.0/24");
    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some(&prefix),
        count: 16,
        ..PoolSpec::default()
    })
    .unwrap();
    let ranges = hosts.range().split_even(servers.len()).unwrap();
    servers
        .iter()
        .zip(ranges)
        .zip(0u32..)
        .map(|((server, range), id)| {
            let profile =
                LoadProfile::new(vec![Phase::new("steady", duration, Shape::Hold, 1.0, 1.0)])
                    .unwrap();
            let mut schedule =
                ScheduleConfig::new(Objective::SessionsPerSecond(rate), profile, SEED);
            schedule.arrival = Arrival::Constant;
            schedule.stream = stream_id(id, 0);
            let mut flow = FlowRef::new(Arc::clone(scenario), 0);
            flow.limits = limits;
            WorkerConfig {
                id,
                core: None,
                role: Role::Client(Box::new(ClientConfig {
                    flow,
                    schedule,
                    servers: vec![server.ip()],
                    port: Some(server.port()),
                    sources: SourceHosts {
                        selector: HostSelector::new(
                            "clients",
                            range,
                            Selection::RoundRobin,
                            SEED,
                            id,
                        )
                        .unwrap(),
                        pool: hosts.clone(),
                        ipv6: false,
                    },
                    connect: ConnectConfig::default(),
                })),
            }
        })
        .collect()
}

fn total(report: &RunReport, side: Side) -> Snapshot {
    report.collector.total(side).unwrap()
}

fn count(s: &Snapshot, m: MetricId) -> u64 {
    s.counter_total(m)
}

fn assert_all_workers_ok(report: &RunReport) {
    for (id, r) in &report.workers {
        assert!(r.is_ok(), "worker {id}: {r:?}");
    }
    assert!(report.collector.all_final());
}

fn join_within(handle: RuntimeHandle, limit: Duration) -> RunReport {
    let started = Instant::now();
    let report = handle.join();
    let took = started.elapsed();
    assert!(took <= limit, "join took {took:?}, limit {limit:?}");
    report
}

#[test]
fn web_get_at_a_fixed_rate_reports_matching_counters() {
    let scenario = web_get();
    let servers = start_servers(&scenario, &[v4(40, 0, 1), v4(40, 0, 2)], 10);
    let addrs = servers.listen_addrs();
    assert_eq!(addrs.len(), 2);
    let config = RuntimeConfig::new(
        SEED,
        clients(
            &scenario,
            &addrs,
            41,
            50.0,
            Duration::from_secs(2),
            Limits::default(),
        ),
    );
    let started = Instant::now();
    let client = Runtime::start(config).unwrap();
    // Constant arrivals at 50 per second for 2 seconds give 100 starts per worker.
    let client = join_within(client, Duration::from_secs(10));
    let run_time = started.elapsed();
    servers.stop(Duration::from_secs(1));
    let server = join_within(servers, Duration::from_secs(2));
    assert_all_workers_ok(&client);
    assert_all_workers_ok(&server);

    let c = total(&client, Side::Client);
    let s = total(&server, Side::Server);
    assert_eq!(count(&c, metric::SESSIONS_ATTEMPTED), 200);
    assert_eq!(count(&c, metric::SESSIONS_ESTABLISHED), 200);
    assert_eq!(count(&c, metric::SESSIONS_SUCCEEDED), 200);
    assert_eq!(count(&c, metric::SESSIONS_FAILED), 0);
    assert_eq!(count(&s, metric::SESSIONS_ATTEMPTED), 200);
    assert_eq!(count(&s, metric::SESSIONS_SUCCEEDED), 200);
    assert_eq!(count(&s, metric::SESSIONS_FAILED), 0);
    // Every session series carries the side label of its worker, including
    // the concurrent gauge and the duration histogram.
    for (snapshot, side) in [(&c, "client"), (&s, "server")] {
        for m in [
            metric::SESSIONS_ATTEMPTED,
            metric::SESSIONS_FAILED,
            metric::SESSIONS_CONCURRENT,
            metric::SESSIONS_DURATION,
            metric::BYTES_TX,
        ] {
            let series: Vec<_> = snapshot.iter().filter(|(k, _)| k.metric() == m).collect();
            assert!(!series.is_empty(), "{side}: no series for {m:?}");
            for (k, _) in series {
                assert!(
                    k.labels().iter().any(|(l, v)| *l == "side" && v == side),
                    "{side}: {:?}",
                    k.labels()
                );
            }
        }
    }
    let duration_samples = |snapshot: &Snapshot| {
        snapshot
            .iter()
            .filter(|(k, _)| k.metric() == metric::SESSIONS_DURATION)
            .map(|(_, v)| match v {
                fps_stats::SeriesValue::Histogram(h) => h.len(),
                other => panic!("duration is not a histogram: {other:?}"),
            })
            .sum::<u64>()
    };
    assert_eq!(duration_samples(&c), 200);
    assert_eq!(duration_samples(&s), 200);
    // Four requests and four responses with 4096 byte bodies per session.
    assert_eq!(count(&c, metric::BYTES_TX), count(&s, metric::BYTES_RX));
    assert_eq!(count(&c, metric::BYTES_RX), count(&s, metric::BYTES_TX));
    let response_bytes = count(&s, metric::BYTES_TX) / 200;
    assert!(
        response_bytes > 4 * 4096,
        "{response_bytes} bytes per session"
    );
    // Each worker sent one snapshot per second and a final one.
    for id in 0..2 {
        let n = client.collector.received(id);
        assert!(
            n >= 2 && n <= 2 + run_time.as_secs() + 1,
            "worker {id}: {n}"
        );
        assert_eq!(
            count(
                &client.collector.latest(id).unwrap().snapshot,
                metric::SESSIONS_ATTEMPTED
            ),
            100
        );
    }
}

#[test]
fn ramp_down_lets_short_sessions_finish() {
    let scenario = web_get();
    let servers = start_servers(&scenario, &[v4(42, 0, 1)], 10);
    let addrs = servers.listen_addrs();
    let config = RuntimeConfig::new(
        SEED,
        clients(
            &scenario,
            &addrs,
            43,
            40.0,
            Duration::from_secs(30),
            Limits::default(),
        ),
    );
    let mut client = Runtime::start(config).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    let ramp_down = Duration::from_secs(2);
    let stopped = Instant::now();
    client.stop(ramp_down);
    let attempted_at_stop = count(
        &client.collect().total(Side::Client).unwrap(),
        metric::SESSIONS_ATTEMPTED,
    );
    let client = join_within(client, ramp_down + Duration::from_secs(1));
    assert!(stopped.elapsed() <= ramp_down + Duration::from_secs(1));
    servers.stop(Duration::from_secs(1));
    let server = join_within(servers, Duration::from_secs(2));
    assert_all_workers_ok(&client);
    assert_all_workers_ok(&server);
    let c = total(&client, Side::Client);
    let attempted = count(&c, metric::SESSIONS_ATTEMPTED);
    assert!(attempted >= attempted_at_stop);
    assert!(
        attempted > 0 && attempted < 40,
        "{attempted} sessions started"
    );
    assert_eq!(count(&c, metric::SESSIONS_SUCCEEDED), attempted);
    assert_eq!(count(&c, metric::SESSIONS_FAILED), 0);
    let s = total(&server, Side::Server);
    assert_eq!(count(&s, metric::SESSIONS_SUCCEEDED), attempted);
}

/// A flow whose server answers after 20 seconds, so every session is still
/// open when the run stops.
const SLOW: &str = r#"
scenario: slow
flows:
  - id: slow
    module: http1
    client:
      - connect: { port: 8080 }
      - send: "GET / HTTP/1.1\r\n\r\n"
      - expect: { pattern: "\r\n\r\n", timeout_ms: 60000 }
      - close: fin
    server:
      - accept: { port: 8080 }
      - expect: { pattern: "\r\n\r\n", timeout_ms: 60000 }
      - delay: { ms: 20000 }
      - send: "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
      - close: fin
"#;

#[test]
fn stop_resets_sessions_still_open_after_the_ramp_down() {
    let scenario = compile(SLOW);
    let servers = start_servers(&scenario, &[v4(44, 0, 1)], 10);
    let addrs = servers.listen_addrs();
    let config = RuntimeConfig::new(
        SEED,
        clients(
            &scenario,
            &addrs,
            45,
            20.0,
            Duration::from_secs(30),
            Limits::default(),
        ),
    );
    let client = Runtime::start(config).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let ramp_down = Duration::from_millis(700);
    let stopped = Instant::now();
    client.stop(ramp_down);
    let client = join_within(client, ramp_down + Duration::from_secs(1));
    let took = stopped.elapsed();
    assert!(
        took >= ramp_down,
        "stop ended after {took:?}, before the ramp down"
    );
    // The server side sees the resets; abort ends its sessions at once.
    let aborted = Instant::now();
    servers.abort();
    let server = join_within(servers, Duration::from_secs(1));
    assert!(aborted.elapsed() < Duration::from_secs(1));
    assert_all_workers_ok(&client);
    assert_all_workers_ok(&server);
    let c = total(&client, Side::Client);
    let attempted = count(&c, metric::SESSIONS_ATTEMPTED);
    assert!(attempted >= 5, "{attempted} sessions started");
    assert_eq!(count(&c, metric::SESSIONS_SUCCEEDED), 0);
    assert_eq!(count(&c, metric::SESSIONS_FAILED), attempted);
    let aborted_series = c
        .iter()
        .filter(|(k, _)| {
            k.metric() == metric::SESSIONS_FAILED
                && k.labels()
                    .iter()
                    .any(|(l, v)| *l == "reason" && v == "aborted")
        })
        .count();
    assert_eq!(aborted_series, 1);
    let s = total(&server, Side::Server);
    assert_eq!(count(&s, metric::SESSIONS_SUCCEEDED), 0);
    assert_eq!(count(&s, metric::SESSIONS_ATTEMPTED), attempted);
}

#[test]
fn abort_ends_every_worker_at_once() {
    let scenario = compile(SLOW);
    let servers = start_servers(&scenario, &[v4(46, 0, 1)], 10);
    let addrs = servers.listen_addrs();
    let config = RuntimeConfig::new(
        SEED,
        clients(
            &scenario,
            &addrs,
            47,
            20.0,
            Duration::from_secs(30),
            Limits::default(),
        ),
    );
    let client = Runtime::start(config).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    client.abort();
    servers.abort();
    let client = join_within(client, Duration::from_secs(1));
    let server = join_within(servers, Duration::from_secs(1));
    assert_all_workers_ok(&client);
    assert_all_workers_ok(&server);
    let c = total(&client, Side::Client);
    assert!(count(&c, metric::SESSIONS_ATTEMPTED) > 0);
    assert_eq!(
        count(&c, metric::SESSIONS_FAILED),
        count(&c, metric::SESSIONS_ATTEMPTED)
    );
}

#[test]
fn a_worker_pins_to_an_allowed_core() {
    let scenario = web_get();
    let core = core_affinity::get_core_ids()
        .and_then(|ids| ids.first().copied())
        .map(|c| c.id);
    let workers = vec![WorkerConfig {
        id: 3,
        core,
        role: Role::Server(ServerConfig {
            flow: FlowRef::new(Arc::clone(&scenario), 0),
            listen: vec![SocketAddr::new(v4(48, 0, 1), 0)],
            config: ListenConfig::default(),
        }),
    }];
    let handle = Runtime::start(RuntimeConfig::new(SEED, workers)).unwrap();
    handle.abort();
    let report = handle.join();
    let exit = report.workers[&3].as_ref().unwrap();
    assert_eq!(exit.pinned, core.map(|_| true));
}

#[test]
fn a_bind_failure_fails_the_start() {
    let scenario = web_get();
    let taken = std::net::TcpListener::bind(SocketAddr::new(v4(49, 0, 1), 0)).unwrap();
    let addr = taken.local_addr().unwrap();
    let workers = vec![WorkerConfig {
        id: 0,
        core: None,
        role: Role::Server(ServerConfig {
            flow: FlowRef::new(Arc::clone(&scenario), 0),
            listen: vec![addr],
            config: ListenConfig {
                reuse_address: false,
                ..ListenConfig::default()
            },
        }),
    }];
    let err = Runtime::start(RuntimeConfig::new(SEED, workers)).unwrap_err();
    assert!(err.to_string().contains("listen"), "{err}");
}

#[test]
fn the_config_check_rejects_duplicate_ids_and_missing_servers() {
    let scenario = web_get();
    let server = |id| WorkerConfig {
        id,
        core: None,
        role: Role::Server(ServerConfig {
            flow: FlowRef::new(Arc::clone(&scenario), 0),
            listen: vec![SocketAddr::new(v4(50, 0, 1), 0)],
            config: ListenConfig::default(),
        }),
    };
    let err = Runtime::start(RuntimeConfig::new(SEED, vec![server(1), server(1)])).unwrap_err();
    assert!(err.to_string().contains("appears twice"), "{err}");
    let mut c = clients(
        &scenario,
        &[SocketAddr::new(v4(50, 0, 2), 80)],
        51,
        1.0,
        Duration::from_secs(1),
        Limits::default(),
    );
    if let Role::Client(cfg) = &mut c[0].role {
        cfg.servers.clear();
    }
    let err = Runtime::start(RuntimeConfig::new(SEED, c)).unwrap_err();
    assert!(err.to_string().contains("no server address"), "{err}");
    assert!(Runtime::start(RuntimeConfig::new(SEED, Vec::new())).is_err());
}
