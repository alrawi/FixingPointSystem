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

/// The sessions that the client workers have started so far, from the newest
/// snapshots. The sample waits two snapshot intervals first, so a start that
/// just happened is in it. Both ends of a window see the same staleness.
fn started(client: &mut RuntimeHandle) -> (Instant, u64) {
    std::thread::sleep(Duration::from_millis(2 * SNAPSHOT_MS));
    let at = Instant::now();
    let c = client.collect().total(Side::Client).unwrap();
    (at, count(&c, metric::SESSIONS_ATTEMPTED))
}

const SNAPSHOT_MS: u64 = 20;

/// Returns the rate that the client workers realized over `window`.
fn realized(client: &mut RuntimeHandle, window: Duration) -> f64 {
    let (t0, n0) = started(client);
    std::thread::sleep(window);
    let (t1, n1) = started(client);
    (n1 - n0) as f64 / (t1 - t0).as_secs_f64()
}

/// ENG-18. The runtime runs on the wall clock of its threads, so this test
/// cannot use a simulated clock as `fps-sched` does. It therefore measures
/// the realized rate over windows of 1.2 seconds, between two snapshots whose
/// staleness is the same at both ends, and accepts 15 percent. The scheduler
/// itself holds 1 percent for constant arrivals on a simulated clock (ENG-13).
/// The 15 percent cover the 100 ms poll of a worker after a change, thread
/// scheduling on a shared two CPU host, and the 20 ms snapshot grain, which
/// together move a window of 1.2 seconds by at most about 10 sessions at the
/// lowest rate of 36 sessions per second.
#[test]
fn rate_control_pauses_resumes_and_changes_the_rate_of_a_running_scenario() {
    const TOLERANCE: f64 = 0.15;
    let scenario = web_get();
    let servers = start_servers(&scenario, &[v4(51, 0, 1), v4(51, 0, 2)], 10);
    let addrs = servers.listen_addrs();
    let mut workers = clients(
        &scenario,
        &addrs,
        52,
        30.0,
        Duration::from_secs(60),
        Limits::default(),
    );
    // The mix entry name differs from the scenario name.
    for w in &mut workers {
        if let Role::Client(c) = &mut w.role {
            c.flow.entry = Some("browse".to_owned());
        }
    }
    let mut config = RuntimeConfig::new(SEED, workers);
    config.snapshot_interval = Duration::from_millis(SNAPSHOT_MS);
    let mut client = Runtime::start(config).unwrap();

    // Two client workers registered under the entry name, at 30 each.
    let control = client.rate_control();
    assert_eq!(control.scenarios(), vec!["browse".to_owned()]);
    assert_eq!(
        control.total("browse"),
        Some(Objective::SessionsPerSecond(60.0))
    );
    let window = Duration::from_millis(1200);
    let within = |got: f64, want: f64| {
        assert!(
            (got - want).abs() <= want * TOLERANCE,
            "realized {got:.1} sessions per second, expected {want} within {TOLERANCE}"
        );
    };
    within(realized(&mut client, window), 60.0);

    // An unknown id changes nothing.
    assert!(control
        .update(&[("web_get", Objective::SessionsPerSecond(5.0))])
        .is_err());
    assert_eq!(
        control.total("browse"),
        Some(Objective::SessionsPerSecond(60.0))
    );

    // Pause. Once the workers have read the handle, no session starts.
    control
        .update(&[("browse", Objective::SessionsPerSecond(0.0))])
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let (_, paused_at) = started(&mut client);
    std::thread::sleep(Duration::from_millis(1000));
    let (_, still) = started(&mut client);
    assert_eq!(still, paused_at, "a paused scenario started sessions");

    // Resume. A worker with nothing due must wake on its own: the first start
    // comes within the 100 ms poll plus one start spacing and the snapshot lag.
    let resumed = Instant::now();
    control
        .update(&[("browse", Objective::SessionsPerSecond(60.0))])
        .unwrap();
    while started(&mut client).1 == paused_at {
        assert!(
            resumed.elapsed() < Duration::from_millis(500),
            "no session started 500 ms after the resume"
        );
    }
    std::thread::sleep(Duration::from_millis(200));
    within(realized(&mut client, window), 60.0);

    // Two changes, up and down, on the same running workers.
    control
        .update(&[("browse", Objective::SessionsPerSecond(120.0))])
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    within(realized(&mut client, window), 120.0);
    control
        .update(&[("browse", Objective::SessionsPerSecond(36.0))])
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    within(realized(&mut client, window), 36.0);

    client.stop(Duration::from_secs(2));
    let client = join_within(client, Duration::from_secs(5));
    servers.stop(Duration::from_secs(1));
    let server = join_within(servers, Duration::from_secs(2));
    assert_all_workers_ok(&client);
    assert_all_workers_ok(&server);
    let c = total(&client, Side::Client);
    assert_eq!(count(&c, metric::SESSIONS_FAILED), 0);
}

/// ENG-18. A worker with nothing due must wake on its own, so this test
/// gives the workers a snapshot interval of 30 seconds, which would otherwise
/// wake them, and watches the connections that they open to a plain listener.
/// A worker paused at rate 0 sleeps with `next_due` at `None`, and the first
/// connection after a resume must arrive within the 100 ms poll, one start
/// spacing of 20 ms, and a margin for a shared host.
#[test]
fn a_paused_worker_wakes_within_the_poll_interval_of_a_resume() {
    let scenario = web_get();
    let listener = std::net::TcpListener::bind(SocketAddr::new(v4(53, 0, 1), 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = listener.local_addr().unwrap();
    let mut workers = clients(
        &scenario,
        &[server],
        54,
        50.0,
        Duration::from_secs(60),
        Limits::default(),
    );
    for w in &mut workers {
        if let Role::Client(c) = &mut w.role {
            c.flow.entry = Some("idle".to_owned());
        }
    }
    let mut config = RuntimeConfig::new(SEED, workers);
    config.snapshot_interval = Duration::from_secs(30);
    let client = Runtime::start(config).unwrap();
    let control = client.rate_control();
    let mut accepted = Vec::new();
    let mut poll = |limit: Duration| {
        let t = Instant::now();
        while t.elapsed() < limit {
            if let Ok((s, _)) = listener.accept() {
                accepted.push(s);
                return Some(t.elapsed());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    };
    for round in 1..=2 {
        // Pause, and wait until the workers read the handle and the
        // connections already made are taken.
        control
            .update(&[("idle", Objective::SessionsPerSecond(0.0))])
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        while poll(Duration::from_millis(100)).is_some() {}
        assert!(
            poll(Duration::from_millis(400)).is_none(),
            "a paused worker connected"
        );
        control
            .update(&[("idle", Objective::SessionsPerSecond(50.0))])
            .unwrap();
        let wake = poll(Duration::from_secs(2)).expect("no connection 2 s after the resume");
        assert!(
            wake <= Duration::from_millis(300),
            "round {round}: the worker woke {wake:?} after the resume"
        );
    }
    client.abort();
    let report = client.join();
    assert!(report.workers.values().all(Result::is_ok));
}
