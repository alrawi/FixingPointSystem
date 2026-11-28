// SPDX-License-Identifier: GPL-3.0-only
//! A plan runs through the agent on the datapath, on in memory links (AGT-25).
//!
//! The AF_XDP test of this crate (`afxdp_veth.rs`) needs root and a veth pair.
//! These tests run the same path without either: the agent prepares the
//! example plan (http1 and dns) as an agent that selected AF_XDP, starts the
//! datapath on `fps_backend_afxdp::memory::frame_pair` in place of the two
//! AF_XDP queues (`backend::start_on_queues`), and `Prepared::start_on` hands
//! the datapath to the TCP and UDP workers. The plan has two plan workers, so
//! two server workers listen on port 8080 and two on port 53, each with its
//! own server host, which is the case the datapath listener cannot serve by
//! itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fps_agent::assemble::Sides;
use fps_agent::backend::{start_on_queues, BackendError, StartedBackend};
use fps_agent::control::CaptureCell;
use fps_agent::plan::Plan;
use fps_agent::run::{prepare_on, RunError, SERVER_RAMP_DOWN};
use fps_agent::tls::KeyTap;
use fps_agent::trust::ModulePolicy;
use fps_backend_afxdp::memory::frame_pair;
use fps_backend_afxdp::{BackendKind, FrameIo};
use fps_stats::{metric, MetricId, SeriesValue, Snapshot};
use serde_json::Value as Json;

fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

/// The example plan on the two ports of a datapath, with two plan workers
/// that each own one client slice and one server host.
fn plan_json() -> Json {
    let text = std::fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json"))
        .expect("example plan");
    let mut plan: Json = serde_json::from_str(&text).expect("example plan JSON");
    plan["backend"] = "BACKEND_AF_XDP".into();
    plan["hostPools"][0]["osProfile"] = "windows11".into();
    plan["hostPools"][1]["osProfile"] = "macos14".into();
    for w in plan["workers"].as_array_mut().expect("workers") {
        w["rates"][0]["sessionsPerSecond"] = 20.into();
        w["rates"][1]["sessionsPerSecond"] = 10.into();
    }
    plan["load"]["phases"] = serde_json::json!([
        { "name": "steady", "duration": "2s", "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    plan["warmup"] = "0s".into();
    plan
}

fn queues() -> Vec<(Box<dyn FrameIo>, Box<dyn FrameIo>)> {
    queue_pairs(1)
}

fn queue_pairs(n: usize) -> Vec<(Box<dyn FrameIo>, Box<dyn FrameIo>)> {
    (0..n)
        .map(|_| {
            let (client, server) = frame_pair(2048, 1600);
            (
                Box::new(client) as Box<dyn FrameIo>,
                Box::new(server) as Box<dyn FrameIo>,
            )
        })
        .collect()
}

fn try_prepare(name: &str, plan: &Json) -> Result<fps_agent::run::Prepared, Box<RunError>> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-datapath-run");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(plan).expect("text")).expect("write");
    prepare_on(
        Plan::load(&path).expect("the plan loads"),
        &contracts().join("examples/modules"),
        Sides::Both,
        &ModulePolicy::default(),
        &KeyTap::new(),
        Some(BackendKind::AfXdp),
    )
    .map_err(Box::new)
}

fn prepare(name: &str, plan: &Json) -> fps_agent::run::Prepared {
    try_prepare(name, plan).expect("the plan prepares on AF_XDP")
}

/// The counter of `metric` for `application` on `side`.
fn count(total: &Snapshot, name: MetricId, application: &str, side: &str) -> u64 {
    total
        .iter()
        .filter(|(k, _)| {
            k.metric() == name
                && k.labels()
                    .contains(&("application", application.to_owned()))
                && k.labels().contains(&("side", side.to_owned()))
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(n) => *n,
            _ => 0,
        })
        .sum()
}

fn run_to_the_end(prepared: fps_agent::run::Prepared) -> fps_agent::run::Finished {
    run_on(prepared, queues())
}

fn run_on(
    prepared: fps_agent::run::Prepared,
    queues: Vec<(Box<dyn FrameIo>, Box<dyn FrameIo>)>,
) -> fps_agent::run::Finished {
    let backend = start_on_queues(prepared.plan(), Sides::Both, queues).expect("the datapath");
    let mut active = prepared
        .start_on(
            StartedBackend::AfXdp(Box::new(backend)),
            Duration::from_millis(200),
            CaptureCell::new(),
        )
        .expect("the workers start on the datapath");
    while !active.is_done(Duration::from_secs(1)) {
        let _ = active.enforce_grace();
        std::thread::sleep(Duration::from_millis(20));
    }
    active.finish(SERVER_RAMP_DOWN).expect("the run ends")
}

#[test]
fn the_http1_and_dns_modules_run_through_the_agent_on_the_datapath() {
    let finished = run_to_the_end(prepare("run.json", &plan_json()));
    let (client, server) = (
        finished.client.as_ref().expect("client totals"),
        finished.server.as_ref().expect("server totals"),
    );
    for (application, expected) in [("http1", 30), ("dns", 15)] {
        let attempted = count(client, metric::SESSIONS_ATTEMPTED, application, "client");
        let ok = count(client, metric::SESSIONS_SUCCEEDED, application, "client");
        let served = count(server, metric::SESSIONS_SUCCEEDED, application, "server");
        // Two seconds at 2 x 20 and 2 x 10 sessions per second, less the
        // first and the last tick of the schedule.
        assert!(
            attempted >= expected,
            "{application}: only {attempted} sessions attempted"
        );
        assert_eq!(ok, attempted, "{application}: every client session passes");
        assert_eq!(served, attempted, "{application}: the servers saw them all");
    }
    assert_eq!(client.counter_total(metric::SESSIONS_FAILED), 0);
    assert_eq!(server.counter_total(metric::SESSIONS_FAILED), 0);

    // Every session of both modules went through the queues of the lane: the
    // 2 x 4 requests of an http1 session are TCP frames, and the dns query and
    // its answer are UDP datagrams.
    assert_eq!(finished.lanes.len(), 1);
    let lane = &finished.lanes[0];
    assert!(!lane.panicked);
    let http = count(client, metric::SESSIONS_SUCCEEDED, "http1", "client");
    let dns = count(client, metric::SESSIONS_SUCCEEDED, "dns", "client");
    assert!(lane.stats.client.tcp >= http * 8, "{:?}", lane.stats);
    assert_eq!(lane.stats.udp_client.tx_datagrams, dns);
    assert_eq!(lane.stats.udp_server.tx_datagrams, dns);
    assert_eq!(lane.stats.udp_server.rx_accepted, dns);
    assert_eq!(lane.stats.udp_client.rx_no_listener, 0);
    assert_eq!(lane.stats.server.udp_unsupported, 0);
}

#[test]
fn a_host_pool_that_names_an_unknown_os_profile_fails_before_the_backend_starts() {
    let mut plan = plan_json();
    plan["hostPools"][1]["osProfile"] = "beos".into();
    let e = try_prepare("profile.json", &plan).expect_err("an unknown profile is a plan error");
    assert!(
        matches!(*e, RunError::Backend(BackendError::Plan(_))),
        "{e:?}"
    );
    let message = e.to_string();
    assert!(message.contains("os_profile 'beos'"), "{message}");
    assert!(message.contains("host pool servers"), "{message}");
}

#[test]
fn a_dropped_run_stops_the_datapath_and_its_listeners() {
    let prepared = prepare("dropped.json", &plan_json());
    let backend = start_on_queues(prepared.plan(), Sides::Both, queues()).expect("the datapath");
    let transport = backend.transport();
    let active = prepared
        .start_on(
            StartedBackend::AfXdp(Box::new(backend)),
            Duration::from_millis(200),
            CaptureCell::new(),
        )
        .expect("start");
    // The servers listen on 8080 and 53 for every worker, one port each.
    assert_eq!(transport.open_ports(), (1, 1));
    active.abort();
    // `abort` ends the workers and stops the backend.
    assert_eq!(transport.open_ports(), (0, 0));
}

/// AGT-30: with a queue pair for each of the two plan workers, the datapath
/// starts two lanes, each with the hosts of its worker, and both carry
/// sessions. Queue pairs beyond the workers stay unused.
#[test]
fn the_lanes_follow_the_workers_of_the_plan() {
    for pairs in [2, 3] {
        let finished = run_on(prepare("lanes.json", &plan_json()), queue_pairs(pairs));
        assert_eq!(finished.lanes.len(), 2, "{pairs} queue pairs, two workers");
        let (client, server) = (
            finished.client.as_ref().expect("client totals"),
            finished.server.as_ref().expect("server totals"),
        );
        for (k, lane) in finished.lanes.iter().enumerate() {
            assert!(!lane.panicked, "lane {k}");
            assert!(lane.stats.client.tcp > 0, "lane {k}: {:?}", lane.stats);
            assert!(lane.stats.server.tcp > 0, "lane {k}: {:?}", lane.stats);
            assert!(lane.stats.udp_client.tx_datagrams > 0, "lane {k}");
        }
        let dns = count(client, metric::SESSIONS_SUCCEEDED, "dns", "client");
        let sent: u64 = finished
            .lanes
            .iter()
            .map(|l| l.stats.udp_client.tx_datagrams)
            .sum();
        assert_eq!(sent, dns, "every dns session is one datagram on one lane");
        assert_eq!(client.counter_total(metric::SESSIONS_FAILED), 0);
        assert_eq!(server.counter_total(metric::SESSIONS_FAILED), 0);
    }
}

/// Bug #689: `strike/conn_churn` on the datapath. Each instance holds a keeper
/// connection and spawns 32 connections that reset at once, matched by server
/// accepts that reset the same way. The own stack must keep a connection that
/// the peer reset after the handshake and before `accept` (the accept returns
/// it and its first read reports the reset), and `connect` must succeed when
/// the reset comes right after the SYN-ACK, as on a kernel. Without both,
/// about a third of the sessions failed (`connect_timeout` on the server side,
/// `unexpected_reset` on the client side).
#[test]
fn strike_conn_churn_runs_through_the_agent_on_the_datapath_without_a_failed_session() {
    use fps_agent::modules::{sha256_digest, tree_digest};
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
    let source = std::fs::read_to_string(modules.join("strike/scenarios/conn_churn.yaml"))
        .expect("scenario source");
    let manifest = std::fs::read_to_string(modules.join("strike/manifest.yaml")).expect("manifest");
    let version = manifest
        .lines()
        .find_map(|l| l.strip_prefix("version:"))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .expect("version");
    let mut plan = plan_json();
    plan["modules"] = serde_json::json!([{
        "name": "strike",
        "version": version,
        "digest": tree_digest("strike", &modules.join("strike")).expect("digest"),
    }]);
    plan["scenarios"] = serde_json::json!([{
        "ref": "strike/conn_churn",
        "module": "strike",
        "digest": sha256_digest(source.as_bytes()),
        "source": source,
    }]);
    plan["mix"] = serde_json::json!([{
        "id": "conn_churn",
        "scenario": "strike/conn_churn",
        "weight": 1,
        "params": { "server_port": 8081, "churn_port": 8082 },
        "clientPool": "clients",
        "serverPool": "servers",
    }]);
    for w in plan["workers"].as_array_mut().expect("workers") {
        w["rates"] = serde_json::json!([{ "mixEntry": "conn_churn", "sessionsPerSecond": 5 }]);
    }
    plan["load"]["phases"] = serde_json::json!([
        { "name": "steady", "duration": "4s", "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-datapath-run");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join("conn_churn.json");
    std::fs::write(&path, serde_json::to_string_pretty(&plan).expect("text")).expect("write");
    let prepared = prepare_on(
        Plan::load(&path).expect("the plan loads"),
        &modules,
        Sides::Both,
        &ModulePolicy::default(),
        &KeyTap::new(),
        Some(BackendKind::AfXdp),
    )
    .expect("the plan prepares on AF_XDP");
    let finished = run_to_the_end(prepared);
    let (client, server) = (
        finished.client.as_ref().expect("client totals"),
        finished.server.as_ref().expect("server totals"),
    );
    let attempted = count(client, metric::SESSIONS_ATTEMPTED, "strike", "client");
    assert!(attempted >= 20, "only {attempted} sessions attempted");
    assert_eq!(
        count(client, metric::SESSIONS_SUCCEEDED, "strike", "client"),
        attempted
    );
    assert_eq!(
        count(server, metric::SESSIONS_SUCCEEDED, "strike", "server"),
        attempted
    );
    assert_eq!(client.counter_total(metric::SESSIONS_FAILED), 0);
    assert_eq!(server.counter_total(metric::SESSIONS_FAILED), 0);
}
