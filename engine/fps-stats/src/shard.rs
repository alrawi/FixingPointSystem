// SPDX-License-Identifier: GPL-3.0-only
//! The statistics shard that one worker owns.
//!
//! A worker registers its series once, when it loads the plan or first meets
//! a scenario, and keeps the returned handles. Recording through a handle
//! indexes a vector that only this worker touches, so the hot path takes no
//! lock, uses no atomic instruction, and allocates nothing. Other threads see
//! the values only through [`Shard::snapshot`], which the owning worker calls
//! once per snapshot interval and sends to the collector.

use std::collections::HashMap;

use hdrhistogram::Histogram;

use crate::catalog::{label_values, MetricId, MetricKind};
use crate::error::StatsError;
use crate::snapshot::{SeriesValue, Snapshot};

/// Number of significant decimal digits that every histogram keeps.
pub const HISTOGRAM_SIGFIG: u8 = 3;

/// Largest value that a histogram records, one hour in microseconds. Larger
/// values are recorded as this value, which bounds the memory of a histogram.
pub const HISTOGRAM_HIGHEST: u64 = 3_600_000_000;

/// One series, which is a catalog metric with one set of label values.
///
/// Labels are sorted by name, so two keys built from the same labels in any
/// order compare equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeriesKey {
    metric: MetricId,
    labels: Vec<(&'static str, String)>,
}

impl SeriesKey {
    /// Builds a key and checks the labels against the catalog entry of the metric.
    ///
    /// Each label must be one of the catalog labels of the metric, may appear
    /// once, and must take one of the enumerated values when the catalog
    /// enumerates them. A key may omit labels, which aggregates over them.
    pub fn new(metric: MetricId, labels: &[(&str, &str)]) -> Result<Self, StatsError> {
        let def = metric.def();
        let mut checked: Vec<(&'static str, String)> = Vec::with_capacity(labels.len());
        for &(name, value) in labels {
            let Some(&label) = def.labels.iter().find(|l| **l == name) else {
                return Err(StatsError::UnknownLabel {
                    metric: def.name,
                    label: name.to_owned(),
                });
            };
            if checked.iter().any(|(l, _)| *l == label) {
                return Err(StatsError::DuplicateLabel {
                    metric: def.name,
                    label,
                });
            }
            if let Some(values) = label_values(label) {
                if !values.contains(&value) {
                    return Err(StatsError::LabelValue {
                        metric: def.name,
                        label,
                        value: value.to_owned(),
                    });
                }
            }
            checked.push((label, value.to_owned()));
        }
        checked.sort();
        Ok(Self {
            metric,
            labels: checked,
        })
    }

    /// Returns the metric of the series.
    pub fn metric(&self) -> MetricId {
        self.metric
    }

    /// Returns the label names and values, sorted by name.
    pub fn labels(&self) -> &[(&'static str, String)] {
        &self.labels
    }
}

/// Handle of a counter series within one shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CounterId(u32);

/// Handle of a gauge series within one shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GaugeId(u32);

/// Handle of a histogram series within one shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HistogramId(u32);

#[derive(Debug, Clone, Copy)]
enum Slot {
    Counter(CounterId),
    Gauge(GaugeId),
    Histogram(HistogramId),
}

/// The counters, gauges, and histograms of one worker.
///
/// A handle is valid only for the shard that returned it. A handle from
/// another shard either names another series of this shard or is ignored, so
/// a runtime gives each worker its own handles.
#[derive(Debug)]
pub struct Shard {
    worker_id: u32,
    index: HashMap<SeriesKey, Slot>,
    counter_keys: Vec<SeriesKey>,
    counters: Vec<u64>,
    gauge_keys: Vec<SeriesKey>,
    gauges: Vec<f64>,
    histogram_keys: Vec<SeriesKey>,
    histograms: Vec<Histogram<u64>>,
}

impl Shard {
    /// Creates an empty shard for the worker.
    pub fn new(worker_id: u32) -> Self {
        Self {
            worker_id,
            index: HashMap::new(),
            counter_keys: Vec::new(),
            counters: Vec::new(),
            gauge_keys: Vec::new(),
            gauges: Vec::new(),
            histogram_keys: Vec::new(),
            histograms: Vec::new(),
        }
    }

    /// Returns the worker id of the shard.
    pub fn worker_id(&self) -> u32 {
        self.worker_id
    }

    /// Registers a counter series, or returns the handle of the existing one.
    pub fn counter(
        &mut self,
        metric: MetricId,
        labels: &[(&str, &str)],
    ) -> Result<CounterId, StatsError> {
        check_kind(metric, MetricKind::Counter)?;
        let key = SeriesKey::new(metric, labels)?;
        if let Some(Slot::Counter(id)) = self.index.get(&key) {
            return Ok(*id);
        }
        let id = CounterId(next_index(self.counters.len(), metric, "counter")?);
        self.counters.push(0);
        self.counter_keys.push(key.clone());
        self.index.insert(key, Slot::Counter(id));
        Ok(id)
    }

    /// Registers a gauge series, or returns the handle of the existing one.
    ///
    /// Only gauges that merge by summing have shard values, see
    /// [`MetricDef::merges_by_sum`](crate::MetricDef::merges_by_sum).
    pub fn gauge(
        &mut self,
        metric: MetricId,
        labels: &[(&str, &str)],
    ) -> Result<GaugeId, StatsError> {
        check_kind(metric, MetricKind::Gauge)?;
        let def = metric.def();
        if !def.merges_by_sum() {
            return Err(StatsError::NotAdditive {
                metric: def.name,
                unit: def.unit,
            });
        }
        let key = SeriesKey::new(metric, labels)?;
        if let Some(Slot::Gauge(id)) = self.index.get(&key) {
            return Ok(*id);
        }
        let id = GaugeId(next_index(self.gauges.len(), metric, "gauge")?);
        self.gauges.push(0.0);
        self.gauge_keys.push(key.clone());
        self.index.insert(key, Slot::Gauge(id));
        Ok(id)
    }

    /// Registers a histogram series, or returns the handle of the existing one.
    pub fn histogram(
        &mut self,
        metric: MetricId,
        labels: &[(&str, &str)],
    ) -> Result<HistogramId, StatsError> {
        check_kind(metric, MetricKind::Histogram)?;
        let key = SeriesKey::new(metric, labels)?;
        if let Some(Slot::Histogram(id)) = self.index.get(&key) {
            return Ok(*id);
        }
        let id = HistogramId(next_index(self.histograms.len(), metric, "histogram")?);
        self.histograms.push(new_histogram(metric)?);
        self.histogram_keys.push(key.clone());
        self.index.insert(key, Slot::Histogram(id));
        Ok(id)
    }

    /// Adds `n` to a counter. The counter saturates at `u64::MAX`.
    #[inline]
    pub fn add(&mut self, id: CounterId, n: u64) {
        if let Some(c) = self.counters.get_mut(id.0 as usize) {
            *c = c.saturating_add(n);
        }
    }

    /// Adds one to a counter.
    #[inline]
    pub fn inc(&mut self, id: CounterId) {
        self.add(id, 1);
    }

    /// Sets a gauge.
    #[inline]
    pub fn set(&mut self, id: GaugeId, value: f64) {
        if let Some(g) = self.gauges.get_mut(id.0 as usize) {
            *g = value;
        }
    }

    /// Adds `delta` to a gauge, for example plus one when a session opens and
    /// minus one when it closes.
    #[inline]
    pub fn adjust(&mut self, id: GaugeId, delta: f64) {
        if let Some(g) = self.gauges.get_mut(id.0 as usize) {
            *g += delta;
        }
    }

    /// Records one value, in the catalog unit of the metric.
    ///
    /// Values above [`HISTOGRAM_HIGHEST`] are recorded as that value. The
    /// histogram grows when a value exceeds every earlier value of the
    /// series, which allocates at most a few dozen times over its life.
    #[inline]
    pub fn record(&mut self, id: HistogramId, value: u64) {
        if let Some(h) = self.histograms.get_mut(id.0 as usize) {
            let value = value.min(HISTOGRAM_HIGHEST);
            if h.record(value).is_err() {
                h.saturating_record(value);
            }
        }
    }

    /// Returns the current value of a counter.
    pub fn counter_value(&self, id: CounterId) -> Option<u64> {
        self.counters.get(id.0 as usize).copied()
    }

    /// Returns the current value of a gauge.
    pub fn gauge_value(&self, id: GaugeId) -> Option<f64> {
        self.gauges.get(id.0 as usize).copied()
    }

    /// Returns the histogram of a series.
    pub fn histogram_value(&self, id: HistogramId) -> Option<&Histogram<u64>> {
        self.histograms.get(id.0 as usize)
    }

    /// Copies every series into a per worker snapshot.
    ///
    /// The owning worker calls this once per snapshot interval. Values are
    /// cumulative from the creation of the shard, so a snapshot that the
    /// collector drops loses no data.
    pub fn snapshot(&self) -> Snapshot {
        let mut snap = Snapshot::for_worker(self.worker_id);
        for (key, value) in self.counter_keys.iter().zip(&self.counters) {
            snap.insert(key.clone(), SeriesValue::Count(*value));
        }
        for (key, value) in self.gauge_keys.iter().zip(&self.gauges) {
            snap.insert(key.clone(), SeriesValue::Gauge(*value));
        }
        for (key, value) in self.histogram_keys.iter().zip(&self.histograms) {
            snap.insert(key.clone(), SeriesValue::Histogram(value.clone()));
        }
        snap
    }
}

fn check_kind(metric: MetricId, wanted: MetricKind) -> Result<(), StatsError> {
    let def = metric.def();
    if def.kind == wanted {
        Ok(())
    } else {
        Err(StatsError::wrong_kind(def.name, wanted, def.kind))
    }
}

fn next_index(len: usize, metric: MetricId, kind: &'static str) -> Result<u32, StatsError> {
    u32::try_from(len).map_err(|_| StatsError::TooManySeries {
        metric: metric.name(),
        kind,
    })
}

/// Creates an empty histogram with the precision of the crate.
pub(crate) fn new_histogram(metric: MetricId) -> Result<Histogram<u64>, StatsError> {
    let mut h =
        Histogram::new_with_bounds(1, 2, HISTOGRAM_SIGFIG).map_err(|e| StatsError::Histogram {
            metric: metric.name(),
            operation: "create",
            detail: e.to_string(),
        })?;
    h.auto(true);
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::metric;

    #[test]
    fn registration_is_idempotent_and_label_order_free() {
        let mut s = Shard::new(0);
        let a = s
            .counter(
                metric::SESSIONS_ATTEMPTED,
                &[("application", "http1"), ("scenario", "web_get")],
            )
            .unwrap();
        let b = s
            .counter(
                metric::SESSIONS_ATTEMPTED,
                &[("scenario", "web_get"), ("application", "http1")],
            )
            .unwrap();
        assert_eq!(a, b);
        s.inc(a);
        s.add(b, 4);
        assert_eq!(s.counter_value(a), Some(5));
    }

    #[test]
    fn registration_checks_the_catalog() {
        let mut s = Shard::new(0);
        assert!(matches!(
            s.counter(metric::SESSIONS_RESPONSE_TIME, &[]),
            Err(StatsError::WrongKind { .. })
        ));
        assert!(matches!(
            s.counter(metric::SESSIONS_ATTEMPTED, &[("port", "p1")]),
            Err(StatsError::UnknownLabel { .. })
        ));
        assert!(matches!(
            s.counter(
                metric::SESSIONS_FAILED,
                &[("reason", "aborted"), ("reason", "internal")]
            ),
            Err(StatsError::DuplicateLabel { .. })
        ));
        assert!(matches!(
            s.counter(metric::SESSIONS_FAILED, &[("reason", "bored")]),
            Err(StatsError::LabelValue { .. })
        ));
        assert!(matches!(
            s.gauge(metric::SESSIONS_FAILED_RATIO, &[]),
            Err(StatsError::NotAdditive { .. })
        ));
    }

    #[test]
    fn counters_saturate() {
        let mut s = Shard::new(0);
        let c = s.counter(metric::BYTES_TX, &[]).unwrap();
        s.add(c, u64::MAX);
        s.add(c, 10);
        assert_eq!(s.counter_value(c), Some(u64::MAX));
    }

    #[test]
    fn gauges_set_and_adjust() {
        let mut s = Shard::new(0);
        let g = s.gauge(metric::SESSIONS_CONCURRENT, &[]).unwrap();
        s.adjust(g, 1.0);
        s.adjust(g, 1.0);
        s.adjust(g, -1.0);
        assert_eq!(s.gauge_value(g), Some(1.0));
        s.set(g, 7.0);
        assert_eq!(s.gauge_value(g), Some(7.0));
    }

    #[test]
    fn histograms_grow_and_clamp() {
        let mut s = Shard::new(0);
        let h = s.histogram(metric::ACTIONS_LATENCY, &[]).unwrap();
        s.record(h, 0);
        s.record(h, 150);
        s.record(h, 2_000_000);
        s.record(h, u64::MAX);
        let hist = s.histogram_value(h).unwrap();
        assert_eq!(hist.len(), 4);
        assert!(hist.max() >= HISTOGRAM_HIGHEST);
        assert!(hist.max() <= HISTOGRAM_HIGHEST + HISTOGRAM_HIGHEST / 1000);
        assert!(hist.equivalent(hist.value_at_quantile(0.5), 150));
    }

    #[test]
    fn foreign_handles_never_panic() {
        let mut big = Shard::new(0);
        let mut small = Shard::new(1);
        let mut last = None;
        for scenario in ["a", "b", "c"] {
            last = Some(
                big.counter(metric::BYTES_RX, &[("scenario", scenario)])
                    .unwrap(),
            );
        }
        small.inc(last.unwrap());
        assert_eq!(small.counter_value(last.unwrap()), None);
    }
}
