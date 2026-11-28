// SPDX-License-Identifier: GPL-3.0-only
//! The spike scenarios at small N, with every check of the run.

use fps_spike_smoltcp_flow_table::{baseline, scenario};

#[test]
fn flow_table_runs_all_phases_at_64_connections() {
    let r = scenario::run(64, 2, 16);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    assert_eq!(r.flows_left, 0);
    assert_eq!(r.dropped, 64, "one dropped request per connection");
    assert!(
        r.retransmit_fires >= 64,
        "every dropped request needs a timer driven retransmission, got {}",
        r.retransmit_fires
    );
    assert_eq!(r.orphans, 0);
    assert!(r.loss.fire.count >= 64);
}

#[test]
fn flow_table_runs_with_one_connection() {
    let r = scenario::run(1, 1, 1);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    assert_eq!(r.retransmit_fires, 1);
}

#[test]
fn baseline_loop_completes_round_trips() {
    let b = baseline::run(64, 16);
    assert!(b.failures.is_empty(), "{:?}", b.failures);
    assert!(b.turn.count > 0);
}
