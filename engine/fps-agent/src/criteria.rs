// SPDX-License-Identifier: GPL-3.0-only
//! Evaluation of the pass and fail criteria of a plan.
//!
//! Only the standalone mode evaluates criteria, over the totals of the side
//! that it reports. A criterion names a counter, a gauge, or a histogram
//! statistic such as `sessions.duration.p99`, and it carries no labels, so
//! the value aggregates over every label set. Counters and histograms count
//! from the end of the warmup, which the agent marks with a baseline
//! snapshot. Gauges take their final value. The two derived ratios of the
//! catalog divide their counters as the catalog formula says. A comparison
//! with an undefined value, such as a ratio over zero sessions, fails.

use fps_stats::{metric, MetricId, MetricKind, SeriesValue, Snapshot};

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

/// Evaluates every criterion over `last` minus `base`.
pub fn evaluate(
    criteria: &[Criterion],
    last: &Snapshot,
    base: Option<&Snapshot>,
) -> Result<Vec<CriterionResult>, RunError> {
    criteria
        .iter()
        .map(|c| {
            if c.op == CriterionOp::Unspecified {
                return Err(RunError::rule(format!("criterion {} has no op", c.metric)));
            }
            let value = value_of(&c.metric, last, base)?;
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

fn counter(m: MetricId, last: &Snapshot, base: Option<&Snapshot>) -> u64 {
    last.counter_total(m)
        .saturating_sub(base.map_or(0, |b| b.counter_total(m)))
}

#[allow(clippy::cast_precision_loss)]
fn value_of(name: &str, last: &Snapshot, base: Option<&Snapshot>) -> Result<f64, RunError> {
    if let Some(m) = MetricId::from_name(name) {
        if let Some((_, num, den)) = RATIOS.iter().find(|(r, _, _)| *r == m) {
            let (n, d) = (counter(*num, last, base), counter(*den, last, base));
            return Ok(if d == 0 {
                f64::NAN
            } else {
                n as f64 / d as f64
            });
        }
        return match m.def().kind {
            MetricKind::Counter => Ok(counter(m, last, base) as f64),
            MetricKind::Gauge => {
                let mut found = false;
                let mut sum = 0.0;
                for (k, v) in last.iter() {
                    if let (true, SeriesValue::Gauge(g)) = (k.metric() == m, v) {
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
            if let (true, SeriesValue::Histogram(h)) = (k.metric() == m, v) {
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
