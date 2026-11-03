// SPDX-License-Identifier: GPL-3.0-only
//! Snapshots of shards, their merge, and the proto3 JSON mapping of
//! `CounterSnapshot` from `contracts/proto/fps/agent/v1/agent.proto`.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use hdrhistogram::serialization::{Serializer as _, V2DeflateSerializer};
use hdrhistogram::Histogram;
use serde_json::{Map, Number, Value};

use crate::catalog::MetricId;
use crate::error::StatsError;
use crate::shard::{SeriesKey, HISTOGRAM_SIGFIG};

/// The value of one series in a snapshot.
#[derive(Debug, Clone, PartialEq)]
pub enum SeriesValue {
    /// A counter value.
    Count(u64),
    /// A gauge value.
    Gauge(f64),
    /// A histogram in the catalog unit of the metric.
    Histogram(Histogram<u64>),
}

/// The cumulative values of one worker, or of every worker after a merge.
///
/// Series are ordered by catalog position and then by labels, so the same
/// values always serialize to the same bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    worker_id: Option<u32>,
    series: BTreeMap<SeriesKey, SeriesValue>,
}

/// The fields of `CounterSnapshot` that do not come from the shards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// The plan id, `CounterSnapshot.plan_id`.
    pub plan_id: String,
    /// The stream sequence number, starting at 1.
    pub sequence: u64,
    /// The agent clock time of the snapshot.
    pub time: Timestamp,
    /// The name of the load phase at the time of the snapshot.
    pub phase: String,
    /// True for the last snapshot of the run.
    pub is_final: bool,
}

/// A point in time as `google.protobuf.Timestamp` represents it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// Seconds since 1970-01-01T00:00:00Z.
    pub seconds: i64,
    /// Nanoseconds within the second, below 1000000000.
    pub nanos: u32,
}

impl Timestamp {
    /// Converts a system time. The agent calls this at its process edge with
    /// the time of its injected clock.
    pub fn from_system_time(t: SystemTime) -> Self {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => Self {
                seconds: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
                nanos: d.subsec_nanos(),
            },
            Err(e) => {
                let d = e.duration();
                let mut seconds = -i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
                let mut nanos = d.subsec_nanos();
                if nanos > 0 {
                    seconds -= 1;
                    nanos = 1_000_000_000 - nanos;
                }
                Self { seconds, nanos }
            }
        }
    }

    /// Formats the time in RFC 3339 with a `Z` suffix and 0, 3, 6, or 9
    /// fractional digits, as the proto3 JSON mapping requires.
    pub fn to_rfc3339(self) -> Result<String, StatsError> {
        // 0001-01-01T00:00:00Z and 9999-12-31T23:59:59Z.
        const MIN: i64 = -62_135_596_800;
        const MAX: i64 = 253_402_300_799;
        if !(MIN..=MAX).contains(&self.seconds) || self.nanos >= 1_000_000_000 {
            return Err(StatsError::TimeRange {
                seconds: self.seconds,
                nanos: self.nanos,
            });
        }
        let days = self.seconds.div_euclid(86_400);
        let secs = self.seconds.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        let (hh, mm, ss) = (secs / 3600, secs / 60 % 60, secs % 60);
        let frac = match self.nanos {
            0 => String::new(),
            n if n % 1_000_000 == 0 => format!(".{:03}", n / 1_000_000),
            n if n % 1_000 == 0 => format!(".{:06}", n / 1_000),
            n => format!(".{n:09}"),
        };
        Ok(format!(
            "{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}{frac}Z"
        ))
    }
}

/// Converts days since 1970-01-01 to a proleptic Gregorian date, after
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

impl Snapshot {
    /// Creates an empty snapshot of one worker.
    pub fn for_worker(worker_id: u32) -> Self {
        Self {
            worker_id: Some(worker_id),
            series: BTreeMap::new(),
        }
    }

    /// Creates an empty aggregate snapshot.
    pub fn aggregate() -> Self {
        Self::default()
    }

    /// Merges per worker snapshots into one aggregate snapshot.
    ///
    /// Counters, additive gauges, and histograms of the same series add up.
    /// The collector keeps the newest snapshot of every worker and merges
    /// them, because shard values are cumulative.
    pub fn merge_all<'a, I>(snapshots: I) -> Result<Self, StatsError>
    where
        I: IntoIterator<Item = &'a Snapshot>,
    {
        let mut out = Self::aggregate();
        for s in snapshots {
            out.merge(s)?;
        }
        Ok(out)
    }

    /// Adds the values of `other` to this snapshot.
    pub fn merge(&mut self, other: &Snapshot) -> Result<(), StatsError> {
        for (key, value) in &other.series {
            match self.series.get_mut(key) {
                None => {
                    self.series.insert(key.clone(), value.clone());
                }
                Some(mine) => add_value(key.metric(), mine, value)?,
            }
        }
        Ok(())
    }

    pub(crate) fn insert(&mut self, key: SeriesKey, value: SeriesValue) {
        self.series.insert(key, value);
    }

    /// Returns the worker id, or `None` for an aggregate snapshot.
    pub fn worker_id(&self) -> Option<u32> {
        self.worker_id
    }

    /// Returns the value of one series.
    pub fn get(&self, key: &SeriesKey) -> Option<&SeriesValue> {
        self.series.get(key)
    }

    /// Iterates over every series in serialization order.
    pub fn iter(&self) -> impl Iterator<Item = (&SeriesKey, &SeriesValue)> {
        self.series.iter()
    }

    /// Returns the number of series.
    pub fn len(&self) -> usize {
        self.series.len()
    }

    /// Reports whether the snapshot holds no series.
    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// Sums a counter over every label set, which is the run total of the metric.
    pub fn counter_total(&self, metric: MetricId) -> u64 {
        self.series
            .iter()
            .filter(|(k, _)| k.metric() == metric)
            .map(|(_, v)| match v {
                SeriesValue::Count(c) => *c,
                _ => 0,
            })
            .fold(0, u64::saturating_add)
    }

    /// Serializes the snapshot to the proto3 JSON mapping of `CounterSnapshot`.
    ///
    /// Field names are lower camel case, 64 bit integers are strings, bytes
    /// are base64, and fields with default values are omitted, as the
    /// canonical proto3 JSON printer does.
    pub fn to_proto_json(&self, meta: &SnapshotMeta) -> Result<Value, StatsError> {
        let mut root = Map::new();
        if !meta.plan_id.is_empty() {
            root.insert("planId".into(), meta.plan_id.clone().into());
        }
        if meta.sequence != 0 {
            root.insert("sequence".into(), meta.sequence.to_string().into());
        }
        root.insert("time".into(), meta.time.to_rfc3339()?.into());
        if !meta.phase.is_empty() {
            root.insert("phase".into(), meta.phase.clone().into());
        }
        match self.worker_id {
            Some(0) => {}
            Some(id) => {
                root.insert("workerId".into(), id.into());
            }
            None => {
                root.insert("aggregate".into(), true.into());
            }
        }
        if meta.is_final {
            root.insert("isFinal".into(), true.into());
        }

        let mut counters = Vec::new();
        let mut histograms = Vec::new();
        let mut serializer = V2DeflateSerializer::new();
        let mut buf = Vec::new();
        for (key, value) in &self.series {
            let mut entry = Map::new();
            entry.insert("metric".into(), key.metric().name().into());
            if !key.labels().is_empty() {
                let labels: Map<String, Value> = key
                    .labels()
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), Value::from(v.as_str())))
                    .collect();
                entry.insert("labels".into(), labels.into());
            }
            match value {
                SeriesValue::Count(c) => {
                    entry.insert("count".into(), c.to_string().into());
                    counters.push(Value::Object(entry));
                }
                SeriesValue::Gauge(g) => {
                    entry.insert("gauge".into(), double_json(*g));
                    counters.push(Value::Object(entry));
                }
                SeriesValue::Histogram(h) => {
                    if !h.is_empty() {
                        entry.insert("count".into(), h.len().to_string().into());
                    }
                    let canonical = canonical_histogram(key.metric(), h)?;
                    buf.clear();
                    serializer.serialize(&canonical, &mut buf).map_err(|e| {
                        StatsError::Histogram {
                            metric: key.metric().name(),
                            operation: "encode",
                            detail: format!("{e:?}"),
                        }
                    })?;
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
                    entry.insert("hdrV2".into(), encoded.into());
                    histograms.push(Value::Object(entry));
                }
            }
        }
        if !counters.is_empty() {
            root.insert("counters".into(), counters.into());
        }
        if !histograms.is_empty() {
            root.insert("histograms".into(), histograms.into());
        }
        Ok(Value::Object(root))
    }

    /// Serializes the snapshot to a compact JSON string, see [`Snapshot::to_proto_json`].
    pub fn to_proto_json_string(&self, meta: &SnapshotMeta) -> Result<String, StatsError> {
        Ok(self.to_proto_json(meta)?.to_string())
    }
}

fn add_value(
    metric: MetricId,
    mine: &mut SeriesValue,
    other: &SeriesValue,
) -> Result<(), StatsError> {
    match (mine, other) {
        (SeriesValue::Count(a), SeriesValue::Count(b)) => *a = a.saturating_add(*b),
        (SeriesValue::Gauge(a), SeriesValue::Gauge(b)) => *a += *b,
        (SeriesValue::Histogram(a), SeriesValue::Histogram(b)) => {
            a.add(b).map_err(|e| StatsError::Histogram {
                metric: metric.name(),
                operation: "merge",
                detail: format!("{e:?}"),
            })?;
        }
        // The catalog fixes the kind of every metric, so both sides of a
        // series always hold the same variant and this arm never runs.
        _ => {}
    }
    Ok(())
}

/// Copies a histogram into one whose bounds depend only on its largest
/// value. The V2 header records the highest trackable value, which an auto
/// resizing histogram reaches along a path that depends on the recording
/// order, so the copy makes equal distributions encode to equal bytes.
fn canonical_histogram(metric: MetricId, h: &Histogram<u64>) -> Result<Histogram<u64>, StatsError> {
    let err = |operation, detail: String| StatsError::Histogram {
        metric: metric.name(),
        operation,
        detail,
    };
    let mut out = Histogram::new_with_bounds(1, h.max().max(2), HISTOGRAM_SIGFIG)
        .map_err(|e| err("create", e.to_string()))?;
    out.add(h).map_err(|e| err("copy", format!("{e:?}")))?;
    Ok(out)
}

/// Encodes a double as the proto3 JSON mapping does, with the strings
/// `NaN`, `Infinity`, and `-Infinity` for values that JSON numbers cannot hold.
fn double_json(v: f64) -> Value {
    match Number::from_f64(v) {
        Some(n) => Value::Number(n),
        None if v.is_nan() => "NaN".into(),
        None if v > 0.0 => "Infinity".into(),
        None => "-Infinity".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_format_as_rfc3339() {
        let t = |seconds, nanos| Timestamp { seconds, nanos }.to_rfc3339().unwrap();
        assert_eq!(t(0, 0), "1970-01-01T00:00:00Z");
        assert_eq!(t(1_793_675_400, 0), "2026-11-03T03:10:00Z");
        assert_eq!(t(951_782_400, 5_000_000), "2000-02-29T00:00:00.005Z");
        assert_eq!(t(-1, 1_000), "1969-12-31T23:59:59.000001Z");
        assert_eq!(t(1, 123_456_789), "1970-01-01T00:00:01.123456789Z");
        assert_eq!(t(-62_135_596_800, 0), "0001-01-01T00:00:00Z");
        assert_eq!(t(253_402_300_799, 0), "9999-12-31T23:59:59Z");
        assert!(Timestamp {
            seconds: 253_402_300_800,
            nanos: 0
        }
        .to_rfc3339()
        .is_err());
        assert!(Timestamp {
            seconds: 0,
            nanos: 1_000_000_000
        }
        .to_rfc3339()
        .is_err());
    }

    #[test]
    fn system_time_before_the_epoch() {
        let t = UNIX_EPOCH - std::time::Duration::from_millis(1500);
        let ts = Timestamp::from_system_time(t);
        assert_eq!(
            ts,
            Timestamp {
                seconds: -2,
                nanos: 500_000_000
            }
        );
    }

    #[test]
    fn doubles_follow_the_json_mapping() {
        assert_eq!(double_json(1.5), Value::from(1.5));
        assert_eq!(double_json(f64::NAN), Value::from("NaN"));
        assert_eq!(double_json(f64::INFINITY), Value::from("Infinity"));
        assert_eq!(double_json(f64::NEG_INFINITY), Value::from("-Infinity"));
    }
}
