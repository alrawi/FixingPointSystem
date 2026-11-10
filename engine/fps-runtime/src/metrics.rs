// SPDX-License-Identifier: GPL-3.0-only
//! The session series of one worker shard.

use fps_interp::{Failure, Reason, Side, SideStats};
use fps_stats::{metric, FailureReason, SessionCounters, Shard, StatsError};

use crate::tls::TlsEvent;

/// Handles of the series that a worker records for every session.
///
/// The worker registers them once at start through
/// [`fps_stats::SessionCounters`], so every series carries the `side` label of
/// the worker, and recording a session costs a few vector writes and no
/// lookup.
#[derive(Debug, Clone)]
pub(crate) struct SessionMetrics {
    counters: SessionCounters,
}

/// Maps the side of the interpreter to the `side` label value.
pub(crate) fn stats_side(side: Side) -> fps_stats::Side {
    match side {
        Side::Client => fps_stats::Side::Client,
        Side::Server => fps_stats::Side::Server,
    }
}

/// Maps a failure reason of the interpreter to its `reason` label value.
pub(crate) fn stats_reason(reason: Reason) -> FailureReason {
    match reason {
        Reason::ConnectTimeout => FailureReason::ConnectTimeout,
        Reason::ConnectRefused => FailureReason::ConnectRefused,
        Reason::ExpectTimeout => FailureReason::ExpectTimeout,
        Reason::PatternMismatch => FailureReason::PatternMismatch,
        Reason::UnexpectedReset => FailureReason::UnexpectedReset,
        Reason::PeerClosed => FailureReason::PeerClosed,
        Reason::TlsFailure => FailureReason::TlsFailure,
        Reason::Aborted => FailureReason::Aborted,
        Reason::Internal => FailureReason::Internal,
        Reason::DatagramRejected => FailureReason::DatagramRejected,
        Reason::TemplateError => FailureReason::TemplateError,
        Reason::ExtensionError => FailureReason::ExtensionError,
    }
}

impl SessionMetrics {
    /// Registers the series with the `application`, `scenario`, and `side`
    /// labels.
    pub(crate) fn register(
        shard: &mut Shard,
        application: &str,
        scenario: &str,
        side: Side,
    ) -> Result<Self, StatsError> {
        Ok(Self {
            counters: SessionCounters::register(shard, application, scenario, stats_side(side))?,
        })
    }

    /// Records the start of a session.
    pub(crate) fn started(&self, shard: &mut Shard) {
        self.counters.started(shard);
    }

    /// Records the end of a session that ran for `duration_us`.
    pub(crate) fn ended(
        &self,
        shard: &mut Shard,
        stats: SideStats,
        outcome: &Result<(), Failure>,
        duration_us: u64,
    ) {
        self.ended_with(
            shard,
            stats,
            outcome.as_ref().err().map(|f| f.reason),
            duration_us,
        );
    }

    /// Records the end of a session that failed with `failed`, or succeeded
    /// when that is `None`. A scenario instance reports its sides this way,
    /// because it keeps the reason of a side and not the whole failure.
    pub(crate) fn ended_with(
        &self,
        shard: &mut Shard,
        stats: SideStats,
        failed: Option<Reason>,
        duration_us: u64,
    ) {
        if stats.established {
            self.counters.established(shard);
        }
        match failed {
            None => self.counters.succeeded(shard),
            Some(reason) => self.counters.failed(shard, stats_reason(reason)),
        }
        self.counters
            .add_bytes(shard, stats.bytes_tx, stats.bytes_rx);
        self.counters.ended(shard, duration_us);
    }
}

/// The `port` label of the `tls.*` series of a flow that names no port.
pub(crate) const DEFAULT_PORT: &str = "default";

/// Records the TLS handshakes of a worker as the `tls.*` metrics.
///
/// The series carry the `port` label of the flow and the `side` of the
/// handshake, and register when the first event of a kind arrives, so a run
/// without `tls_upgrade` has no `tls.*` series. A handshake is a few
/// counter writes and one histogram write, next to a key exchange.
#[derive(Debug, Clone)]
pub(crate) struct TlsMetrics {
    port: String,
}

impl TlsMetrics {
    pub(crate) fn new(port: Option<&str>) -> Self {
        Self {
            port: port.unwrap_or(DEFAULT_PORT).to_owned(),
        }
    }

    /// Records `events` in `shard`.
    pub(crate) fn record(&self, shard: &mut Shard, events: Vec<TlsEvent>) {
        for event in events {
            if let Err(e) = self.record_one(shard, &event) {
                tracing::warn!(error = %e, "a tls metric could not be recorded");
            }
        }
    }

    fn record_one(&self, shard: &mut Shard, event: &TlsEvent) -> Result<(), StatsError> {
        match event {
            TlsEvent::Completed {
                side,
                elapsed_us,
                resumed,
            } => {
                let l = [("port", self.port.as_str()), ("side", side.as_str())];
                let n = shard.counter(metric::TLS_HANDSHAKES, &l)?;
                shard.inc(n);
                if *resumed {
                    let n = shard.counter(metric::TLS_RESUMPTIONS, &l)?;
                    shard.inc(n);
                }
                let t = shard.histogram(metric::TLS_HANDSHAKE_TIME, &l)?;
                shard.record(t, *elapsed_us);
            }
            TlsEvent::Failed {
                side,
                alert_rx,
                alert_tx,
            } => {
                let l = [("port", self.port.as_str()), ("side", side.as_str())];
                let n = shard.counter(metric::TLS_HANDSHAKE_FAILURES, &l)?;
                shard.inc(n);
                for (name, alert) in [
                    (metric::TLS_ALERTS_TX, alert_tx),
                    (metric::TLS_ALERTS_RX, alert_rx),
                ] {
                    if let Some(alert) = alert {
                        let l = [
                            ("port", self.port.as_str()),
                            ("side", side.as_str()),
                            ("alert", alert.as_str()),
                        ];
                        let n = shard.counter(name, &l)?;
                        shard.inc(n);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_interpreter_reason_maps_to_the_same_label() {
        for reason in Reason::ALL {
            assert_eq!(stats_reason(reason).as_str(), reason.as_str());
        }
        assert_eq!(stats_side(Side::Client).as_str(), "client");
        assert_eq!(stats_side(Side::Server).as_str(), "server");
    }
}
