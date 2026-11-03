// SPDX-License-Identifier: GPL-3.0-only
//! Property tests of the session ids and the collector.

use fps_interp::Side;
use fps_runtime::{global_session_id, Collector, WorkerSnapshot};
use fps_stats::{metric, Shard};
use proptest::prelude::*;

fn snapshot(worker: u32, side: Side, sequence: u64, attempted: u64) -> WorkerSnapshot {
    let mut shard = Shard::new(worker);
    let id = shard
        .counter(metric::SESSIONS_ATTEMPTED, &[("scenario", "web_get")])
        .unwrap();
    shard.add(id, attempted);
    WorkerSnapshot {
        worker,
        side,
        sequence,
        is_final: false,
        snapshot: shard.snapshot(),
    }
}

proptest! {
    /// Sessions of different workers never share an id, so they never share
    /// a generator stream.
    #[test]
    fn session_ids_are_unique_across_workers(
        a in 0u32..1 << 16, b in 0u32..1 << 16,
        x in 0u64..1 << 48, y in 0u64..1 << 48,
    ) {
        prop_assume!((a, x) != (b, y));
        prop_assert_ne!(global_session_id(a, x), global_session_id(b, y));
    }

    /// The collector total equals the sum of the newest cumulative snapshot of
    /// every worker, whatever order the snapshots arrive in.
    #[test]
    fn the_total_sums_the_newest_snapshot_of_each_worker(
        mut sends in proptest::collection::vec((0u32..4, 1u64..20), 1..40),
    ) {
        // Cumulative counters grow with the sequence number.
        let value = |w: u32, seq: u64| seq * 10 + u64::from(w);
        let mut newest = std::collections::BTreeMap::new();
        for (w, seq) in &sends {
            let e = newest.entry(*w).or_insert(0);
            *e = (*e).max(*seq);
        }
        sends.reverse();
        let mut c = Collector::new();
        for (w, seq) in &sends {
            c.absorb(snapshot(*w, Side::Client, *seq, value(*w, *seq)));
        }
        let want: u64 = newest.iter().map(|(w, s)| value(*w, *s)).sum();
        let got = c.total(Side::Client).unwrap().counter_total(metric::SESSIONS_ATTEMPTED);
        prop_assert_eq!(got, want);
        prop_assert_eq!(c.total(Side::Server).unwrap().counter_total(metric::SESSIONS_ATTEMPTED), 0);
    }
}
