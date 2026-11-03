// SPDX-License-Identifier: GPL-3.0-only
//! The session series of one worker shard.

use fps_interp::{Failure, Reason, SideStats};
use fps_stats::{metric, CounterId, GaugeId, HistogramId, Shard, StatsError};

/// Handles of the series that a worker records for every session.
///
/// The worker registers them once at start, so recording a session costs a
/// few vector writes and no lookup.
#[derive(Debug, Clone)]
pub(crate) struct SessionMetrics {
    attempted: CounterId,
    established: CounterId,
    succeeded: CounterId,
    /// One counter per reason, in the order of [`Reason::ALL`].
    failed: Vec<CounterId>,
    bytes_tx: CounterId,
    bytes_rx: CounterId,
    concurrent: GaugeId,
    duration: HistogramId,
}

impl SessionMetrics {
    /// Registers the series with the `application` and `scenario` labels.
    pub(crate) fn register(
        shard: &mut Shard,
        application: &str,
        scenario: &str,
    ) -> Result<Self, StatsError> {
        let labels = [("application", application), ("scenario", scenario)];
        let mut failed = Vec::with_capacity(Reason::ALL.len());
        for reason in Reason::ALL {
            let l = [
                ("application", application),
                ("scenario", scenario),
                ("reason", reason.as_str()),
            ];
            failed.push(shard.counter(metric::SESSIONS_FAILED, &l)?);
        }
        Ok(Self {
            attempted: shard.counter(metric::SESSIONS_ATTEMPTED, &labels)?,
            established: shard.counter(metric::SESSIONS_ESTABLISHED, &labels)?,
            succeeded: shard.counter(metric::SESSIONS_SUCCEEDED, &labels)?,
            failed,
            bytes_tx: shard.counter(metric::BYTES_TX, &labels)?,
            bytes_rx: shard.counter(metric::BYTES_RX, &labels)?,
            concurrent: shard.gauge(metric::SESSIONS_CONCURRENT, &labels)?,
            duration: shard.histogram(metric::SESSIONS_DURATION, &labels)?,
        })
    }

    /// Records the start of a session.
    pub(crate) fn started(&self, shard: &mut Shard) {
        shard.inc(self.attempted);
        shard.adjust(self.concurrent, 1.0);
    }

    /// Records the end of a session that ran for `duration_us`.
    pub(crate) fn ended(
        &self,
        shard: &mut Shard,
        stats: SideStats,
        outcome: &Result<(), Failure>,
        duration_us: u64,
    ) {
        if stats.established {
            shard.inc(self.established);
        }
        match outcome {
            Ok(()) => shard.inc(self.succeeded),
            Err(f) => {
                let i = Reason::ALL.iter().position(|r| *r == f.reason).unwrap_or(0);
                if let Some(id) = self.failed.get(i) {
                    shard.inc(*id);
                }
            }
        }
        shard.add(self.bytes_tx, stats.bytes_tx);
        shard.add(self.bytes_rx, stats.bytes_rx);
        shard.adjust(self.concurrent, -1.0);
        shard.record(self.duration, duration_us);
    }
}
