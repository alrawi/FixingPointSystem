// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the three workloads against the smoltcp stack.

use std::sync::Arc;
use std::time::Duration;

use fps_tcp::workload::{
    session_rate, socket_scaling, throughput, Direction, RateConfig, RateReport, ScaleConfig,
    ThroughputConfig, ThroughputReport,
};
use fps_tcp::{MonotonicClock, SmolConfig, SmolStack, TcpStack};

fn stack(seed: u64) -> SmolStack {
    let config = SmolConfig {
        seed,
        ..SmolConfig::default()
    };
    SmolStack::new(config, Arc::new(MonotonicClock::new()))
}

#[tokio::test(flavor = "current_thread")]
async fn session_rate_completes_sessions_without_failures() {
    let stack = stack(1);
    let config = RateConfig {
        duration: Duration::from_millis(300),
        concurrency: 8,
        request_bytes: 100,
        response_bytes: 3000,
        ..RateConfig::default()
    };
    let run = session_rate(&stack, &config).await.expect("rate");
    let report = RateReport::from_runs(vec![run.clone()]);
    assert_eq!(report.errors, 0);
    assert!(report.sessions >= 8, "only {} sessions", report.sessions);
    assert!(report.sessions_per_s > 0.0);
    assert!(report.latency.p50_us <= report.latency.p99_us);
    assert!(report.latency.p99_us <= report.latency.max_us);
    // A session needs at least three frames to open, two for the data, and four to close.
    assert!(run.stats.frames > report.sessions * 6);
}

#[tokio::test(flavor = "current_thread")]
async fn throughput_counts_every_byte_in_both_directions() {
    for (port, direction) in [(7100, Direction::Upload), (7101, Direction::Download)] {
        let stack = stack(2);
        let config = ThroughputConfig {
            sessions: 3,
            bytes_per_session: 1_000_003,
            chunk: 4096,
            direction,
            port,
        };
        let run = throughput(&stack, &config).await.expect("throughput");
        assert_eq!(run.bytes, 3_000_009, "{direction:?}");
        let report = ThroughputReport::from_runs(&[run]);
        assert!(report.gbit_per_s > 0.0);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn scaling_holds_the_requested_sockets_and_reports_memory() {
    let shards = [stack(10), stack(20)];
    let config = ScaleConfig {
        steps: vec![400, 1200],
        probe_sessions: 20,
        ..ScaleConfig::default()
    };
    let report = socket_scaling(&shards, &config).await.expect("scale");
    assert_eq!(report.shards, 2);
    assert_eq!(report.steps.len(), 2);
    assert!(report.stopped.is_none());
    for step in &report.steps {
        assert!(step.reached);
        // Two sockets per session, plus the listening sockets of both shards.
        assert!(step.sockets >= step.target_sockets as u64);
        assert!(step.sockets < step.target_sockets as u64 + 100);
        assert!(step.idle_poll_us > 0.0);
        assert!(step.probe_sessions_per_s > 0.0);
    }
    assert!(report.max_sockets() >= 1200);
    if let Some(bytes) = report.steps[1].bytes_per_socket {
        assert!(
            bytes > 100.0 && bytes < 100_000.0,
            "{bytes} bytes per socket"
        );
    }
    let held: u64 = shards.iter().map(|s| s.stats().sockets_open).sum();
    assert!(held >= 1200);
}

#[tokio::test(flavor = "current_thread")]
async fn scaling_stops_at_the_first_step_over_budget() {
    let shards = [stack(30)];
    let config = ScaleConfig {
        steps: vec![200, 400],
        step_budget: Duration::ZERO,
        probe_sessions: 5,
        ..ScaleConfig::default()
    };
    let report = socket_scaling(&shards, &config).await.expect("scale");
    assert_eq!(report.steps.len(), 1);
    assert!(!report.steps[0].reached);
    assert!(report.stopped.is_some());
    assert_eq!(report.max_sockets(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn one_server_port_serves_more_sessions_than_the_backlog_at_scale() {
    // 3000 sessions share one server port that keeps a backlog of eight.
    let shards = [stack(40)];
    let config = ScaleConfig {
        steps: vec![6000],
        batch: 500,
        probe_sessions: 5,
        ..ScaleConfig::default()
    };
    let report = socket_scaling(&shards, &config).await.expect("scale");
    assert!(report.steps[0].reached);
    assert!(report.steps[0].sockets >= 6000);
}
