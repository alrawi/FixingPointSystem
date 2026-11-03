// SPDX-License-Identifier: GPL-3.0-only
//! Errors of statistics collection.

use crate::catalog::MetricKind;

/// Errors that series registration, merging, and serialization report.
#[derive(Debug, thiserror::Error)]
pub enum StatsError {
    /// The metric has another type than the registration asked for.
    #[error("metric {metric}: registered as {wanted} but the catalog type is {actual}")]
    WrongKind {
        /// The catalog name of the metric.
        metric: &'static str,
        /// The type that the caller asked for.
        wanted: &'static str,
        /// The type in the catalog.
        actual: &'static str,
    },
    /// The catalog does not allow the label on the metric.
    #[error("metric {metric}: label {label:?} is not in the catalog labels of the metric")]
    UnknownLabel {
        /// The catalog name of the metric.
        metric: &'static str,
        /// The label name that the caller gave.
        label: String,
    },
    /// The label appears twice.
    #[error("metric {metric}: label {label} appears twice")]
    DuplicateLabel {
        /// The catalog name of the metric.
        metric: &'static str,
        /// The repeated label name.
        label: &'static str,
    },
    /// The label value is not one of the values that the catalog enumerates.
    #[error("metric {metric}: label {label} does not allow the value {value:?}")]
    LabelValue {
        /// The catalog name of the metric.
        metric: &'static str,
        /// The label name.
        label: &'static str,
        /// The value that the caller gave.
        value: String,
    },
    /// The gauge is a ratio or a percentage, which workers cannot sum.
    #[error("metric {metric}: gauge in {unit} is derived after the merge and has no shard value")]
    NotAdditive {
        /// The catalog name of the metric.
        metric: &'static str,
        /// The catalog unit of the metric.
        unit: &'static str,
    },
    /// The shard already holds the largest number of series that a handle can address.
    #[error("metric {metric}: shard holds too many {kind} series")]
    TooManySeries {
        /// The catalog name of the metric.
        metric: &'static str,
        /// The series type.
        kind: &'static str,
    },
    /// A histogram could not be created, merged, or encoded.
    #[error("metric {metric}: histogram {operation} failed: {detail}")]
    Histogram {
        /// The catalog name of the metric.
        metric: &'static str,
        /// The failed operation, such as `merge` or `encode`.
        operation: &'static str,
        /// The cause as reported by the histogram library.
        detail: String,
    },
    /// The snapshot time lies outside the range of `google.protobuf.Timestamp`.
    #[error("snapshot time {seconds}s {nanos}ns is outside 0001-01-01 to 9999-12-31")]
    TimeRange {
        /// Seconds since the Unix epoch.
        seconds: i64,
        /// Nanoseconds within the second.
        nanos: u32,
    },
}

impl StatsError {
    pub(crate) fn wrong_kind(metric: &'static str, wanted: MetricKind, actual: MetricKind) -> Self {
        Self::WrongKind {
            metric,
            wanted: wanted.as_str(),
            actual: actual.as_str(),
        }
    }
}
