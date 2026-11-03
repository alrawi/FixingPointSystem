// SPDX-License-Identifier: GPL-3.0-only
//! Component test with eight concurrent workers.
//!
//! Each worker thread owns its shard, records into it, and sends a cumulative
//! snapshot to a collector thread every few thousand increments, as the
//! runtime does once per second. The collector keeps the newest snapshot of
//! every worker and merges them on every arrival. The test checks that the
//! aggregate never decreases and that the final aggregate holds every
//! increment and every histogram sample.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::thread;

use fps_stats::{metric, SeriesKey, Shard, Snapshot};

const WORKERS: u32 = 8;
const ITERATIONS: u64 = 200_000;
const SNAPSHOT_EVERY: u64 = 7_919;
const SCENARIOS: [&str; 3] = ["web_get", "web_post", "download"];

fn labels(scenario: &str) -> [(&str, &str); 2] {
    [("application", "http1"), ("scenario", scenario)]
}

fn worker(id: u32, tx: mpsc::Sender<Snapshot>) {
    let mut shard = Shard::new(id);
    let attempted: Vec<_> = SCENARIOS
        .iter()
        .map(|s| {
            shard
                .counter(metric::SESSIONS_ATTEMPTED, &labels(s))
                .unwrap()
        })
        .collect();
    let bytes = shard.counter(metric::BYTES_TX, &labels("web_get")).unwrap();
    let latency = shard
        .histogram(metric::SESSIONS_RESPONSE_TIME, &labels("web_get"))
        .unwrap();
    let concurrent = shard
        .gauge(metric::SESSIONS_CONCURRENT, &labels("web_get"))
        .unwrap();
    for i in 0..ITERATIONS {
        let slot = usize::try_from(i % 3).unwrap();
        shard.inc(attempted[slot]);
        shard.add(bytes, 3);
        shard.record(latency, i % 5_000);
        shard.adjust(concurrent, if i % 2 == 0 { 1.0 } else { -1.0 });
        if i % SNAPSHOT_EVERY == 0 {
            tx.send(shard.snapshot()).unwrap();
        }
    }
    tx.send(shard.snapshot()).unwrap();
}

#[test]
fn eight_workers_lose_no_increments() {
    let (tx, rx) = mpsc::channel();
    let handles: Vec<_> = (0..WORKERS)
        .map(|id| {
            let tx = tx.clone();
            thread::spawn(move || worker(id, tx))
        })
        .collect();
    drop(tx);

    let mut newest: BTreeMap<u32, Snapshot> = BTreeMap::new();
    let mut last_total = 0;
    let mut merges = 0;
    for snap in rx {
        let id = snap.worker_id().unwrap();
        newest.insert(id, snap);
        let merged = Snapshot::merge_all(newest.values()).unwrap();
        let total = merged.counter_total(metric::SESSIONS_ATTEMPTED);
        assert!(
            total >= last_total,
            "aggregate went from {last_total} to {total}"
        );
        last_total = total;
        merges += 1;
    }
    for h in handles {
        h.join().unwrap();
    }
    assert!(merges > usize::try_from(WORKERS).unwrap());

    let merged = Snapshot::merge_all(newest.values()).unwrap();
    let n = u64::from(WORKERS) * ITERATIONS;
    assert_eq!(merged.worker_id(), None);
    assert_eq!(merged.counter_total(metric::SESSIONS_ATTEMPTED), n);
    assert_eq!(merged.counter_total(metric::BYTES_TX), 3 * n);
    for (slot, s) in SCENARIOS.iter().enumerate() {
        let key = SeriesKey::new(metric::SESSIONS_ATTEMPTED, &labels(s)).unwrap();
        let per_worker = (0..ITERATIONS)
            .filter(|i| *i % 3 == u64::try_from(slot).unwrap())
            .count();
        let want = u64::from(WORKERS) * u64::try_from(per_worker).unwrap();
        assert_eq!(
            merged.get(&key),
            Some(&fps_stats::SeriesValue::Count(want)),
            "{s}"
        );
    }
    let key = SeriesKey::new(metric::SESSIONS_RESPONSE_TIME, &labels("web_get")).unwrap();
    match merged.get(&key) {
        Some(fps_stats::SeriesValue::Histogram(h)) => assert_eq!(h.len(), n),
        other => panic!("histogram missing: {other:?}"),
    }
    let key = SeriesKey::new(metric::SESSIONS_CONCURRENT, &labels("web_get")).unwrap();
    assert_eq!(merged.get(&key), Some(&fps_stats::SeriesValue::Gauge(0.0)));
}
