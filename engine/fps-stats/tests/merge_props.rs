// SPDX-License-Identifier: GPL-3.0-only
//! Property tests of the shard merge.
//!
//! Recording a stream of events into several shards and merging them gives
//! the same snapshot as recording the stream into one shard, whatever the
//! split and whatever the merge order.

use fps_stats::{metric, Shard, Snapshot, SnapshotMeta};
use proptest::prelude::*;

const SCENARIOS: [&str; 4] = ["a", "b", "c", "d"];

#[derive(Debug, Clone)]
struct Event {
    worker: usize,
    scenario: usize,
    bytes: u64,
    latency: u64,
}

fn event(workers: usize) -> impl Strategy<Value = Event> {
    (
        0..workers,
        0..SCENARIOS.len(),
        0..1_000_000u64,
        0..10_000_000u64,
    )
        .prop_map(|(worker, scenario, bytes, latency)| Event {
            worker,
            scenario,
            bytes,
            latency,
        })
}

fn apply(shard: &mut Shard, e: &Event) {
    let l = [
        ("application", "http1"),
        ("scenario", SCENARIOS[e.scenario]),
    ];
    let c = shard.counter(metric::BYTES_RX, &l).unwrap();
    shard.add(c, e.bytes);
    let h = shard.histogram(metric::SESSIONS_DURATION, &l).unwrap();
    shard.record(h, e.latency);
}

fn json(s: &Snapshot) -> String {
    s.to_proto_json_string(&SnapshotMeta::default()).unwrap()
}

proptest! {
    #[test]
    fn split_and_merge_equals_one_shard(
        events in (1usize..9).prop_flat_map(|w| (Just(w), prop::collection::vec(event(w), 0..200))),
        reverse in any::<bool>(),
    ) {
        let (workers, events) = events;
        let mut single = Shard::new(0);
        let mut shards: Vec<Shard> = (0..workers)
            .map(|w| Shard::new(u32::try_from(w).unwrap()))
            .collect();
        for e in &events {
            apply(&mut single, e);
            apply(&mut shards[e.worker], e);
        }
        let mut snaps: Vec<Snapshot> = shards.iter().map(Shard::snapshot).collect();
        if reverse {
            snaps.reverse();
        }
        let merged = Snapshot::merge_all(&snaps).unwrap();
        let expected = Snapshot::merge_all([&single.snapshot()]).unwrap();
        prop_assert_eq!(json(&merged), json(&expected));
        prop_assert_eq!(
            merged.counter_total(metric::BYTES_RX),
            events.iter().map(|e| e.bytes).sum::<u64>()
        );
    }
}
