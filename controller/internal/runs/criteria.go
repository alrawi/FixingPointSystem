// SPDX-License-Identifier: GPL-3.0-only

package runs

import (
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
)

// Metric names from contracts/metrics/catalog.yaml that the orchestrator
// reads from agent snapshots.
const (
	MetricSessionsAttempted   = "sessions.attempted"
	MetricSessionsSucceeded   = "sessions.succeeded"
	MetricSessionsFailed      = "sessions.failed"
	MetricSessionsFailedRatio = "sessions.failed_ratio"
	MetricBytesTx             = "bytes.tx"
	MetricBytesRx             = "bytes.rx"
)

// Outcome is what a run produced when it stops, the input of an Evaluator.
type Outcome struct {
	// Criteria are the pass and fail rules of the test configuration.
	Criteria []compiler.CriterionConfig
	// Totals holds every unlabeled counter of the newest snapshot of each
	// agent, summed over the agents, plus sessions.failed_ratio. The
	// sessions that a failed agent attempted and never finished count as
	// failed, because a crash ends them.
	Totals map[string]float64
	// Agents is the number of agents of the run, and FailedAgents the
	// number that failed during the run.
	Agents       int
	FailedAgents int
}

// CriterionResult is the verdict of one criterion.
type CriterionResult struct {
	compiler.CriterionConfig
	// Evaluated is false when the outcome lacks the metric. Such a
	// criterion does not decide the verdict.
	Evaluated bool
	// Observed is the value of the metric when Evaluated holds.
	Observed float64
	// Passed is true when the observed value satisfies the rule.
	Passed bool
}

// Verdict is the result of the pass and fail criteria.
type Verdict struct {
	// Passed is true when every evaluated criterion passed.
	Passed bool
	// Criteria holds one result per criterion, in configuration order.
	Criteria []CriterionResult
}

// Evaluator decides the verdict of a stopped run. A run whose verdict fails
// ends as failed, and every other stopped run ends as completed. CTL-09
// supplies the evaluator for the whole metric catalog.
type Evaluator func(Outcome) Verdict

// EvaluateTotals is the default Evaluator. It compares every criterion whose
// metric appears in Outcome.Totals, such as sessions.failed_ratio, and skips
// the others. A run in which every agent failed never passes.
func EvaluateTotals(o Outcome) Verdict {
	v := Verdict{Passed: o.Agents == 0 || o.FailedAgents < o.Agents}
	for _, c := range o.Criteria {
		res := CriterionResult{CriterionConfig: c}
		if obs, ok := o.Totals[c.Metric]; ok {
			res.Evaluated, res.Observed, res.Passed = true, obs, compare(obs, c.Op, c.Value)
			if !res.Passed {
				v.Passed = false
			}
		}
		v.Criteria = append(v.Criteria, res)
	}
	return v
}

// compare applies the criterion operator op. An unknown operator fails.
func compare(observed float64, op string, value float64) bool {
	switch op {
	case "<":
		return observed < value
	case "<=":
		return observed <= value
	case ">":
		return observed > value
	case ">=":
		return observed >= value
	case "==":
		return observed == value
	case "!=":
		return observed != value
	}
	return false
}

// totals sums the unlabeled counters and gauges of the newest snapshot of
// every agent and derives sessions.failed_ratio. For an agent in failed, the
// sessions that it attempted without success or failure count as failed.
func totals(snaps map[string]*agentpb.CounterSnapshot, failed map[string]bool) map[string]float64 {
	out := map[string]float64{}
	for id, snap := range snaps {
		agent := map[string]float64{}
		for _, c := range snap.GetCounters() {
			if len(c.GetLabels()) > 0 {
				continue
			}
			switch c.GetValue().(type) {
			case *agentpb.CounterValue_Count:
				agent[c.GetMetric()] += float64(c.GetCount())
			case *agentpb.CounterValue_Gauge:
				agent[c.GetMetric()] += c.GetGauge()
			}
		}
		if failed[id] {
			open := agent[MetricSessionsAttempted] - agent[MetricSessionsSucceeded] - agent[MetricSessionsFailed]
			if open > 0 {
				agent[MetricSessionsFailed] += open
			}
		}
		for k, v := range agent {
			out[k] += v
		}
	}
	delete(out, MetricSessionsFailedRatio)
	if att := out[MetricSessionsAttempted]; att > 0 {
		out[MetricSessionsFailedRatio] = out[MetricSessionsFailed] / att
	} else {
		out[MetricSessionsFailedRatio] = 0
	}
	return out
}
