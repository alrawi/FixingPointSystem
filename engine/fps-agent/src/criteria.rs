// SPDX-License-Identifier: GPL-3.0-only
//! Evaluation of the pass and fail criteria of a plan.
//!
//! Only the standalone mode evaluates criteria. A criterion names a counter,
//! a gauge, or a histogram statistic such as `sessions.duration.p99`. Its
//! `labels` select series, as the `where` field of a test criterion does: a
//! series counts when it carries every label of the filter with that value.
//! A key must be a label that the metric catalog lists for the metric, and a
//! filter that names another label is a rule error.
//!
//! Every scenario and transport series carries the `side` label (CON-08), and
//! a total over both sides would count each session twice. The `side` label of
//! a criterion (`client` or `server`) picks the side. Without it a criterion
//! counts the client side, and [`evaluate_for`] counts another default side
//! for a process that runs no client workers, because such a process has no
//! client series. A metric without the `side` label counts as it is. Labels
//! other than `side` aggregate when the criterion does not name them.
//! Counters and histograms count
//! from the end of the warmup, which the agent marks with a baseline
//! snapshot. Gauges take their final value. The two derived ratios of the
//! catalog divide their counters as the catalog formula says. A comparison
//! with an undefined value, such as a ratio over zero sessions, fails.

use fps_stats::{metric, MetricId, MetricKind, SeriesKey, SeriesValue, Side, Snapshot};

use crate::plan::{Criterion, CriterionOp};
use crate::run::RunError;

/// The outcome of one criterion.
#[derive(Debug, Clone, PartialEq)]
pub struct CriterionResult {
    /// The metric reference of the criterion.
    pub metric: String,
    /// The comparison.
    pub op: CriterionOp,
    /// The threshold.
    pub threshold: f64,
    /// The measured value.
    pub value: f64,
    /// True when the comparison holds.
    pub holds: bool,
}

/// The derived gauges of the catalog that have a formula, as numerator and
/// denominator.
const RATIOS: [(MetricId, MetricId, MetricId); 2] = [
    (
        metric::SESSIONS_FAILED_RATIO,
        metric::SESSIONS_FAILED,
        metric::SESSIONS_ATTEMPTED,
    ),
    (
        metric::TLS_RESUMPTION_RATIO,
        metric::TLS_RESUMPTIONS,
        metric::TLS_HANDSHAKES,
    ),
];

const STATS: [&str; 8] = ["p50", "p90", "p99", "p999", "min", "max", "mean", "count"];

/// Evaluates every criterion over `last` minus `base`, counting the client
/// side.
pub fn evaluate(
    criteria: &[Criterion],
    last: &Snapshot,
    base: Option<&Snapshot>,
) -> Result<Vec<CriterionResult>, RunError> {
    evaluate_for(criteria, last, base, Side::Client)
}

/// Evaluates every criterion over `last` minus `base`. A criterion counts the
/// series of the side that its `side` label names, and the series of
/// `default_side` when it names none. Its other labels filter the series.
pub fn evaluate_for(
    criteria: &[Criterion],
    last: &Snapshot,
    base: Option<&Snapshot>,
    default_side: Side,
) -> Result<Vec<CriterionResult>, RunError> {
    criteria
        .iter()
        .map(|c| {
            if c.op == CriterionOp::Unspecified {
                return Err(RunError::rule(format!("criterion {} has no op", c.metric)));
            }
            let filter = Filter::of(c, default_side)?;
            let value = value_of(&c.metric, last, base, &filter)?;
            Ok(CriterionResult {
                metric: c.metric.clone(),
                op: c.op,
                threshold: c.value,
                value,
                holds: c.op.holds(value, c.value),
            })
        })
        .collect()
}

/// Which series a criterion counts.
struct Filter {
    side: Side,
    /// The labels other than `side` that a series must carry.
    labels: Vec<(String, String)>,
}

impl Filter {
    /// Reads the labels of `c` and checks them against the catalog entry of
    /// its metric.
    fn of(c: &Criterion, default_side: Side) -> Result<Self, RunError> {
        let def = MetricId::from_name(&c.metric)
            .or_else(|| {
                c.metric
                    .rsplit_once('.')
                    .and_then(|(base, _)| MetricId::from_name(base))
            })
            .map(|m| m.def());
        let mut side = default_side;
        let mut labels = Vec::new();
        for (k, v) in &c.labels {
            if let Some(def) = def {
                if !def.labels.contains(&k.as_str()) {
                    return Err(RunError::rule(format!(
                        "criterion {}: label {k} is not a label of the metric, which has {}",
                        c.metric,
                        def.labels.join(", ")
                    )));
                }
            }
            if k == "side" {
                side = match v.as_str() {
                    "client" => Side::Client,
                    "server" => Side::Server,
                    other => {
                        return Err(RunError::rule(format!(
                            "criterion {}: the side label is {other:?}, want client or server",
                            c.metric
                        )))
                    }
                };
            } else {
                labels.push((k.clone(), v.clone()));
            }
        }
        Ok(Self { side, labels })
    }

    /// Returns true when the series belongs to the side, or has no side label,
    /// as a metric of the catalog without the label has none, and carries
    /// every label of the filter.
    fn matches(&self, key: &SeriesKey) -> bool {
        let on_side = key
            .labels()
            .iter()
            .find(|(l, _)| *l == "side")
            .is_none_or(|(_, v)| v == self.side.as_str());
        on_side
            && self
                .labels
                .iter()
                .all(|(k, v)| key.labels().iter().any(|(l, lv)| l == k && lv == v))
    }
}

/// Sums a counter over the series that `filter` selects.
fn counter_total(s: &Snapshot, m: MetricId, filter: &Filter) -> u64 {
    s.iter()
        .filter(|(k, _)| k.metric() == m && filter.matches(k))
        .map(|(_, v)| match v {
            SeriesValue::Count(c) => *c,
            _ => 0,
        })
        .fold(0, u64::saturating_add)
}

fn counter(m: MetricId, last: &Snapshot, base: Option<&Snapshot>, filter: &Filter) -> u64 {
    counter_total(last, m, filter).saturating_sub(base.map_or(0, |b| counter_total(b, m, filter)))
}

#[allow(clippy::cast_precision_loss)]
fn value_of(
    name: &str,
    last: &Snapshot,
    base: Option<&Snapshot>,
    filter: &Filter,
) -> Result<f64, RunError> {
    if let Some(m) = MetricId::from_name(name) {
        if let Some((_, num, den)) = RATIOS.iter().find(|(r, _, _)| *r == m) {
            let (n, d) = (
                counter(*num, last, base, filter),
                counter(*den, last, base, filter),
            );
            return Ok(if d == 0 {
                f64::NAN
            } else {
                n as f64 / d as f64
            });
        }
        return match m.def().kind {
            MetricKind::Counter => Ok(counter(m, last, base, filter) as f64),
            MetricKind::Gauge => {
                let mut found = false;
                let mut sum = 0.0;
                for (k, v) in last.iter() {
                    if let (true, SeriesValue::Gauge(g)) = (k.metric() == m && filter.matches(k), v)
                    {
                        found = true;
                        sum += g;
                    }
                }
                if found && m.def().merges_by_sum() {
                    Ok(sum)
                } else {
                    Err(RunError::Unimplemented(format!(
                        "criterion {name}: this agent does not record gauge {name}"
                    )))
                }
            }
            MetricKind::Histogram => Err(RunError::rule(format!(
                "criterion {name}: a histogram reference needs a statistic such as {name}.p99"
            ))),
        };
    }
    let (base_name, stat) = name
        .rsplit_once('.')
        .ok_or_else(|| RunError::rule(format!("criterion {name}: unknown metric")))?;
    let m = MetricId::from_name(base_name)
        .filter(|m| m.def().kind == MetricKind::Histogram && STATS.contains(&stat))
        .ok_or_else(|| RunError::rule(format!("criterion {name}: unknown metric")))?;
    let merged = |s: &Snapshot| {
        let mut out: Option<hdrhistogram::Histogram<u64>> = None;
        for (k, v) in s.iter() {
            if let (true, SeriesValue::Histogram(h)) = (k.metric() == m && filter.matches(k), v) {
                match &mut out {
                    None => out = Some(h.clone()),
                    Some(o) => {
                        // Histograms of one metric share their bounds.
                        let _ = o.add(h);
                    }
                }
            }
        }
        out
    };
    let Some(mut h) = merged(last) else {
        return Err(RunError::Unimplemented(format!(
            "criterion {name}: this agent does not record histogram {base_name}"
        )));
    };
    if let Some(b) = base.and_then(merged) {
        let _ = h.subtract(&b);
    }
    if stat == "count" {
        return Ok(h.len() as f64);
    }
    if h.is_empty() {
        return Ok(f64::NAN);
    }
    Ok(match stat {
        "p50" => h.value_at_quantile(0.5) as f64,
        "p90" => h.value_at_quantile(0.9) as f64,
        "p99" => h.value_at_quantile(0.99) as f64,
        "p999" => h.value_at_quantile(0.999) as f64,
        "min" => h.min() as f64,
        "max" => h.max() as f64,
        _ => h.mean(),
    })
}

#[cfg(test)]
mod tests {
    use fps_stats::Shard;

    use super::*;

    fn snap(attempted: u64, failed: u64, durations: &[u64]) -> Snapshot {
        let mut shard = Shard::new(0);
        let l = [("application", "http1"), ("scenario", "web_get")];
        let a = shard.counter(metric::SESSIONS_ATTEMPTED, &l).unwrap();
        let f = shard
            .counter(
                metric::SESSIONS_FAILED,
                &[
                    ("application", "http1"),
                    ("scenario", "web_get"),
                    ("reason", "expect_timeout"),
                ],
            )
            .unwrap();
        let h = shard.histogram(metric::SESSIONS_DURATION, &l).unwrap();
        shard.add(a, attempted);
        shard.add(f, failed);
        for d in durations {
            shard.record(h, *d);
        }
        Snapshot::merge_all([&shard.snapshot()]).unwrap()
    }

    fn crit(metric: &str, op: CriterionOp, value: f64) -> Criterion {
        Criterion {
            metric: metric.into(),
            op,
            value,
            labels: Default::default(),
        }
    }

    fn labeled(metric: &str, op: CriterionOp, value: f64, labels: &[(&str, &str)]) -> Criterion {
        Criterion {
            labels: labels
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            ..crit(metric, op, value)
        }
    }

    #[test]
    fn ratio_counts_from_the_warmup_baseline() {
        let base = snap(100, 10, &[]);
        let last = snap(1100, 10, &[]);
        let c = [crit("sessions.failed_ratio", CriterionOp::Lt, 0.001)];
        let r = evaluate(&c, &last, Some(&base)).unwrap();
        assert_eq!(r[0].value, 0.0);
        assert!(r[0].holds);
        let r = evaluate(&c, &last, None).unwrap();
        assert!(!r[0].holds, "{r:?}");
    }

    /// One snapshot with both sides of a scenario, registered through
    /// `SessionCounters` as the workers do. The client side attempted 10
    /// sessions and failed 2. The server side attempted 10 and failed 5.
    fn both_sides() -> Snapshot {
        use fps_stats::{FailureReason, SessionCounters};
        let mut shard = Shard::new(0);
        let client = SessionCounters::register(&mut shard, "dns", "lookup", Side::Client).unwrap();
        let server = SessionCounters::register(&mut shard, "dns", "lookup", Side::Server).unwrap();
        for _ in 0..10 {
            client.started(&mut shard);
            server.started(&mut shard);
            client.ended(&mut shard, 100);
            server.ended(&mut shard, 900);
        }
        for _ in 0..2 {
            client.failed(&mut shard, FailureReason::ExpectTimeout);
        }
        for _ in 0..5 {
            server.failed(&mut shard, FailureReason::PatternMismatch);
        }
        Snapshot::merge_all([&shard.snapshot()]).unwrap()
    }

    #[test]
    fn criteria_count_the_client_side_by_default() {
        let both = both_sides();
        // The snapshot holds both sides, and a plain total counts each session twice.
        assert_eq!(both.counter_total(metric::SESSIONS_ATTEMPTED), 20);
        let c = [
            crit("sessions.attempted", CriterionOp::Eq, 10.0),
            crit("sessions.failed", CriterionOp::Eq, 2.0),
            crit("sessions.failed_ratio", CriterionOp::Eq, 0.2),
            crit("sessions.duration.max", CriterionOp::Lt, 200.0),
            crit("sessions.duration.count", CriterionOp::Eq, 10.0),
        ];
        let r = evaluate(&c, &both, None).unwrap();
        assert!(r.iter().all(|r| r.holds), "{r:?}");
    }

    #[test]
    fn another_default_side_counts_the_server_series() {
        let both = both_sides();
        let c = [
            crit("sessions.attempted", CriterionOp::Eq, 10.0),
            crit("sessions.failed", CriterionOp::Eq, 5.0),
            crit("sessions.failed_ratio", CriterionOp::Eq, 0.5),
            crit("sessions.duration.min", CriterionOp::Ge, 800.0),
        ];
        let r = evaluate_for(&c, &both, None, Side::Server).unwrap();
        assert!(r.iter().all(|r| r.holds), "{r:?}");
        // The baseline subtracts per side too.
        let r = evaluate_for(&c[..1], &both, Some(&both), Side::Client).unwrap();
        assert_eq!(r[0].value, 0.0);
    }

    #[test]
    fn the_side_label_picks_the_side_whatever_the_default() {
        let both = both_sides();
        let server = |metric: &str, op, value| labeled(metric, op, value, &[("side", "server")]);
        let client = |metric: &str, op, value| labeled(metric, op, value, &[("side", "client")]);
        let c = [
            server("sessions.failed", CriterionOp::Eq, 5.0),
            server("sessions.failed_ratio", CriterionOp::Eq, 0.5),
            server("sessions.duration.min", CriterionOp::Ge, 800.0),
            client("sessions.failed", CriterionOp::Eq, 2.0),
            client("sessions.duration.max", CriterionOp::Lt, 200.0),
        ];
        for default in [Side::Client, Side::Server] {
            let r = evaluate_for(&c, &both, None, default).unwrap();
            assert!(r.iter().all(|r| r.holds), "{default:?}: {r:?}");
        }
        // Without the label the default side counts.
        let plain = [crit("sessions.failed", CriterionOp::Eq, 5.0)];
        assert!(evaluate_for(&plain, &both, None, Side::Server).unwrap()[0].holds);
        assert!(!evaluate_for(&plain, &both, None, Side::Client).unwrap()[0].holds);
        // The baseline subtracts per side.
        let r = evaluate_for(&c[..1], &both, Some(&both), Side::Client).unwrap();
        assert_eq!(r[0].value, 0.0);
    }

    #[test]
    fn other_labels_select_series_and_must_be_catalog_labels() {
        let both = both_sides();
        let f = |labels: &[(&str, &str)]| {
            let c = [labeled("sessions.failed", CriterionOp::Eq, 0.0, labels)];
            evaluate_for(&c, &both, None, Side::Client).map(|r| r[0].value)
        };
        // A reason filter counts the matching failures of the default side.
        assert_eq!(f(&[("reason", "expect_timeout")]).unwrap(), 2.0);
        assert_eq!(f(&[("reason", "pattern_mismatch")]).unwrap(), 0.0);
        assert_eq!(
            f(&[("reason", "pattern_mismatch"), ("side", "server")]).unwrap(),
            5.0
        );
        assert_eq!(
            f(&[("application", "dns"), ("scenario", "lookup")]).unwrap(),
            2.0
        );
        assert_eq!(f(&[("application", "http1")]).unwrap(), 0.0);
        // A label that the metric lacks, and a side that is none, are rule errors.
        let e = f(&[("port", "p0")]).unwrap_err().to_string();
        assert!(e.contains("port") && e.contains("sessions.failed"), "{e}");
        assert!(f(&[("side", "both")]).is_err());
        // A histogram statistic takes the labels of its histogram.
        let c = [labeled(
            "sessions.duration.max",
            CriterionOp::Ge,
            900.0,
            &[("side", "server"), ("application", "dns")],
        )];
        assert!(evaluate_for(&c, &both, None, Side::Client).unwrap()[0].holds);
    }

    #[test]
    fn a_ratio_over_zero_sessions_fails() {
        let c = [crit("sessions.failed_ratio", CriterionOp::Lt, 0.001)];
        let r = evaluate(&c, &snap(0, 0, &[]), None).unwrap();
        assert!(r[0].value.is_nan());
        assert!(!r[0].holds);
    }

    #[test]
    fn histogram_statistics_and_unknown_metrics() {
        let last = snap(3, 0, &[100, 200, 300]);
        let r = evaluate(
            &[
                crit("sessions.duration.count", CriterionOp::Eq, 3.0),
                crit("sessions.duration.max", CriterionOp::Ge, 300.0),
                crit("sessions.attempted", CriterionOp::Eq, 3.0),
            ],
            &last,
            None,
        )
        .unwrap();
        assert!(r.iter().all(|r| r.holds), "{r:?}");
        assert!(evaluate(&[crit("no.such", CriterionOp::Lt, 1.0)], &last, None).is_err());
        assert!(evaluate(
            &[crit("sessions.duration", CriterionOp::Lt, 1.0)],
            &last,
            None
        )
        .is_err());
    }
}
