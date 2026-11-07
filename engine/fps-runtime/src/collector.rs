// SPDX-License-Identifier: GPL-3.0-only
//! The snapshots that workers send and the collector that merges them.

use std::collections::BTreeMap;

use fps_interp::Side;
use fps_stats::{Snapshot, StatsError};

/// One cumulative snapshot of one worker shard.
#[derive(Debug, Clone)]
pub struct WorkerSnapshot {
    /// The worker id.
    pub worker: u32,
    /// The side that the worker runs.
    pub side: Side,
    /// The number of the snapshot within the worker, counting from 1.
    pub sequence: u64,
    /// True for the last snapshot, which the worker sends when it ends.
    pub is_final: bool,
    /// The cumulative values of the shard.
    pub snapshot: Snapshot,
}

/// Keeps the newest snapshot of every worker.
///
/// Every session series carries the `side` label of catalog 0.2.0, and the
/// collector still merges client and server workers separately, so a total
/// never adds the two sides of one session together.
#[derive(Debug, Default)]
pub struct Collector {
    latest: BTreeMap<u32, WorkerSnapshot>,
    received: BTreeMap<u32, u64>,
}

impl Collector {
    /// Returns an empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes a snapshot and keeps it when it is newer than the one held.
    pub fn absorb(&mut self, s: WorkerSnapshot) {
        *self.received.entry(s.worker).or_default() += 1;
        match self.latest.get(&s.worker) {
            Some(held) if held.sequence >= s.sequence => {}
            _ => {
                self.latest.insert(s.worker, s);
            }
        }
    }

    /// Returns the newest snapshot of `worker`.
    pub fn latest(&self, worker: u32) -> Option<&WorkerSnapshot> {
        self.latest.get(&worker)
    }

    /// Returns the number of snapshots received from `worker`.
    pub fn received(&self, worker: u32) -> u64 {
        self.received.get(&worker).copied().unwrap_or(0)
    }

    /// Returns true when every worker that reported sent its final snapshot.
    pub fn all_final(&self) -> bool {
        self.latest.values().all(|s| s.is_final)
    }

    /// Merges the newest snapshots of the workers of `side`.
    pub fn total(&self, side: Side) -> Result<Snapshot, StatsError> {
        Snapshot::merge_all(
            self.latest
                .values()
                .filter(|s| s.side == side)
                .map(|s| &s.snapshot),
        )
    }
}

#[cfg(test)]
mod tests {
    use fps_stats::{metric, Shard};

    use super::*;

    fn snap(worker: u32, side: Side, sequence: u64, attempted: u64) -> WorkerSnapshot {
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

    #[test]
    fn keeps_the_newest_snapshot_and_merges_per_side() {
        let mut c = Collector::new();
        c.absorb(snap(0, Side::Client, 2, 20));
        c.absorb(snap(0, Side::Client, 1, 10));
        c.absorb(snap(1, Side::Client, 1, 5));
        c.absorb(snap(2, Side::Server, 1, 7));
        assert_eq!(c.received(0), 2);
        assert_eq!(c.latest(0).unwrap().sequence, 2);
        let client = c.total(Side::Client).unwrap();
        assert_eq!(client.counter_total(metric::SESSIONS_ATTEMPTED), 25);
        let server = c.total(Side::Server).unwrap();
        assert_eq!(server.counter_total(metric::SESSIONS_ATTEMPTED), 7);
        assert!(!c.all_final());
    }
}
