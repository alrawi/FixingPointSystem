// SPDX-License-Identifier: GPL-3.0-only
//! The three workloads of the harness on the own stack (TRN-18), through the
//! `TcpStack` adapter. The component tests of the stack are in `fps-tcp-own`.

use std::time::Duration;

use fps_tcp::workload::{
    session_rate, socket_scaling, throughput, Direction, RateConfig, ScaleConfig, ThroughputConfig,
};
use fps_tcp::OwnStack;

#[tokio::test(flavor = "current_thread")]
async fn the_three_workloads_run_on_the_own_stack() {
    let stack = OwnStack::default();
    let rate = session_rate(
        &stack,
        &RateConfig {
            duration: Duration::from_millis(300),
            concurrency: 8,
            request_bytes: 100,
            response_bytes: 3000,
            ..RateConfig::default()
        },
    )
    .await
    .expect("rate");
    assert_eq!(rate.errors, 0);
    assert!(rate.sessions >= 8, "only {} sessions", rate.sessions);
    // The listener stays, and the closing side of every session holds its
    // tuple in TIME-WAIT for the hold of one second.
    assert!(rate.stats.sockets_open >= 1);
    assert!(rate.stats.sockets_open <= 1 + rate.sessions + 8);
    assert!(rate.stats.frames > rate.sessions * 6);

    for (port, direction) in [(7200, Direction::Upload), (7201, Direction::Download)] {
        let stack = OwnStack::default();
        let run = throughput(
            &stack,
            &ThroughputConfig {
                sessions: 3,
                bytes_per_session: 1_000_003,
                chunk: 4096,
                direction,
                port,
            },
        )
        .await
        .expect("throughput");
        assert_eq!(run.bytes, 3_000_009, "{direction:?}");
        assert_eq!(stack.counters().client.retransmits, 0);
        assert_eq!(stack.counters().server.retransmits, 0);
    }

    let shards = [OwnStack::default(), OwnStack::default()];
    let report = socket_scaling(
        &shards,
        &ScaleConfig {
            steps: vec![400, 1600],
            probe_sessions: 20,
            ..ScaleConfig::default()
        },
    )
    .await
    .expect("scale");
    assert!(report.stopped.is_none(), "{:?}", report.stopped);
    assert!(report.steps.iter().all(|s| s.reached));
    assert!(report.max_sockets() >= 1600);
    assert!(report.steps[1].probe_sessions_per_s > 0.0);
}
