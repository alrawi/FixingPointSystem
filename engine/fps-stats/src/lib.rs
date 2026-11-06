// SPDX-License-Identifier: GPL-3.0-only
//! Counters and histograms.
//!
//! Each worker records into its own [`Shard`], keyed by the metric names of
//! `contracts/metrics/catalog.yaml`. A shard belongs to one worker thread and
//! needs no lock or atomic instruction. Once per snapshot interval the worker
//! calls [`Shard::snapshot`] and sends the cumulative [`Snapshot`] to the
//! collector, which keeps the newest snapshot of every worker, merges them with
//! [`Snapshot::merge_all`], and serializes the result with
//! [`Snapshot::to_proto_json`] to the proto3 JSON mapping of `CounterSnapshot`.
//!
//! ```
//! use fps_stats::{metric, Shard, Snapshot, SnapshotMeta, Timestamp};
//!
//! let mut shard = Shard::new(0);
//! let labels = [("application", "http1"), ("scenario", "web_get")];
//! let attempted = shard.counter(metric::SESSIONS_ATTEMPTED, &labels)?;
//! let response = shard.histogram(metric::SESSIONS_RESPONSE_TIME, &labels)?;
//! shard.inc(attempted);
//! shard.record(response, 1250);
//!
//! let total = Snapshot::merge_all([&shard.snapshot()])?;
//! let meta = SnapshotMeta {
//!     plan_id: "run-1/agent-a".into(),
//!     sequence: 1,
//!     time: Timestamp { seconds: 1_793_675_400, nanos: 0 },
//!     ..SnapshotMeta::default()
//! };
//! let json = total.to_proto_json(&meta)?;
//! assert_eq!(json["counters"][0]["metric"], "sessions.attempted");
//! assert_eq!(json["counters"][0]["count"], "1");
//! # Ok::<(), fps_stats::StatsError>(())
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod catalog;
mod error;
mod series;
mod shard;
mod snapshot;

pub use catalog::{metric, MetricDef, MetricId, MetricKind, CATALOG};
pub use error::StatsError;
pub use series::{FailureReason, SessionCounters, Side, TransportCounters};
pub use shard::{
    CounterId, GaugeId, HistogramId, SeriesKey, Shard, HISTOGRAM_HIGHEST, HISTOGRAM_SIGFIG,
};
pub use snapshot::{SeriesValue, Snapshot, SnapshotMeta, Timestamp};
