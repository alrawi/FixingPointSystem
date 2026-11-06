// SPDX-License-Identifier: GPL-3.0-only

package runs

import (
	"maps"
	"slices"
	"strconv"
	"strings"

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

// Label names and values of the catalog that the orchestrator reads. The
// catalog gives scenario and transport metrics the side label, and a total
// over both sides counts every session twice, so every total of this package
// and every criterion without a side reports the client side.
const (
	LabelSide     = "side"
	SideClient    = "client"
	SideServer    = "server"
	LabelReason   = "reason"
	ReasonAborted = "aborted"
)

// Criterion is one pass and fail rule of a run: the criterion of the test
// configuration and its label filter.
type Criterion struct {
	compiler.CriterionConfig
	// Where holds label names and the values that a series must carry for
	// the criterion to count it. It selects among the labeled series of the
	// metric, for example {application: dns}. Without a side, the criterion
	// counts the client side only.
	Where map[string]string
}

// criteriaOf converts the criteria of a test configuration.
//
// compiler.CriterionConfig has no where field yet, because the compiler
// belongs to CTL-18. When it gains one, set Where here from the field of c.
func criteriaOf(cs []compiler.CriterionConfig) []Criterion {
	out := make([]Criterion, len(cs))
	for i, c := range cs {
		out[i] = Criterion{CriterionConfig: c}
	}
	return out
}

// Series is the newest value of one counter of the run, summed over the
// agents. A counter without labels has nil Labels.
type Series struct {
	Metric string
	Labels map[string]string
	Value  float64
}

// Outcome is what a run produced when it stops, the input of an Evaluator.
type Outcome struct {
	// RunID names the run. An evaluator that reads further metrics, such as
	// the histograms of the metrics aggregator, looks them up by this id.
	RunID string
	// Criteria are the pass and fail rules of the test configuration.
	Criteria []Criterion
	// Totals holds the newest value of every counter and gauge of each
	// agent without labels, plus the counters with labels of the client
	// side that no agent reports without labels, summed over the agents,
	// plus sessions.failed_ratio. The sessions that a failed agent attempted
	// and never finished count as failed, because a crash ends them.
	Totals map[string]float64
	// Series holds every counter of the agents with its labels, summed over
	// the agents, and with the open sessions of failed agents counted as
	// failed sessions of the reason aborted. An evaluator that filters by
	// labels reads it.
	Series []Series
	// Agents is the number of agents of the run, and FailedAgents the
	// number that failed during the run.
	Agents       int
	FailedAgents int
}

// CriterionResult is the verdict of one criterion.
type CriterionResult struct {
	Criterion
	// Evaluated is false when the outcome lacks the metric. Such a
	// criterion does not decide the verdict.
	Evaluated bool
	// Observed is the value of the metric when Evaluated holds.
	Observed float64
	// Passed is true when the observed value satisfies the rule.
	Passed bool
	// Invalid is true when the criterion cannot be evaluated, for example
	// because it names no catalog metric. It fails the run, and the API
	// shows passed false without an observed value.
	Invalid bool
	// Reason says in words why the criterion passed, failed, or has no
	// result.
	Reason string
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
// the others. It cannot select labels, so it skips a criterion with a where
// filter. A run in which every agent failed never passes.
func EvaluateTotals(o Outcome) Verdict {
	v := Verdict{Passed: o.Agents == 0 || o.FailedAgents < o.Agents}
	for _, c := range o.Criteria {
		res := CriterionResult{Criterion: c}
		obs, ok := o.Totals[c.Metric]
		switch {
		case len(c.Where) > 0:
			res.Reason = "this evaluator cannot select labels"
		case !ok:
			res.Reason = "the run reported no value"
		default:
			res.Evaluated, res.Observed, res.Passed = true, obs, compare(obs, c.Op, c.Value)
			res.Reason = holdReason(obs, c.Op, c.Value, res.Passed)
			if !res.Passed {
				v.Passed = false
			}
		}
		v.Criteria = append(v.Criteria, res)
	}
	return v
}

// holdReason words the comparison of an observed value.
func holdReason(observed float64, op string, value float64, passed bool) string {
	verb := "holds"
	if !passed {
		verb = "does not hold"
	}
	return "observed " + formatNumber(observed) + " " + op + " " + formatNumber(value) + " " + verb
}

// formatNumber renders v in plain decimal notation with the fewest digits
// that parse back to v.
func formatNumber(v float64) string { return strconv.FormatFloat(v, 'f', -1, 64) }

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

// seriesKey identifies a counter by its metric and labels.
func seriesKey(metric string, labels map[string]string) string {
	var b strings.Builder
	b.WriteString(metric)
	for _, k := range slices.Sorted(maps.Keys(labels)) {
		b.WriteByte(0)
		b.WriteString(k)
		b.WriteByte('=')
		b.WriteString(labels[k])
	}
	return b.String()
}

// labelKey identifies a label set, leaving out the label skip.
func labelKey(labels map[string]string, skip string) string {
	rest := maps.Clone(labels)
	delete(rest, skip)
	return seriesKey("", rest)
}

// closeOpen returns the counters of a failed agent with its open sessions
// counted as failed. For every sessions.attempted series, the attempts that
// have neither a success nor a failure of the same labels (the reason aside)
// join a sessions.failed series of the reason aborted.
func closeOpen(agent map[string]Series) map[string]Series {
	succeeded, failed := map[string]float64{}, map[string]float64{}
	for _, s := range agent {
		switch s.Metric {
		case MetricSessionsSucceeded:
			succeeded[labelKey(s.Labels, "")] += s.Value
		case MetricSessionsFailed:
			failed[labelKey(s.Labels, LabelReason)] += s.Value
		}
	}
	out := maps.Clone(agent)
	for _, s := range agent {
		if s.Metric != MetricSessionsAttempted {
			continue
		}
		k := labelKey(s.Labels, "")
		open := s.Value - succeeded[k] - failed[k]
		if open <= 0 {
			continue
		}
		var labels map[string]string
		if len(s.Labels) > 0 {
			labels = maps.Clone(s.Labels)
			labels[LabelReason] = ReasonAborted
		}
		key := seriesKey(MetricSessionsFailed, labels)
		cur := out[key]
		out[key] = Series{Metric: MetricSessionsFailed, Labels: labels, Value: cur.Value + open}
	}
	return out
}

// sumSeries sums the counters of every agent by metric and labels, after it
// counts the open sessions of the agents in failed as failed. It returns the
// series ordered by metric and labels.
func sumSeries(perAgent map[string]map[string]Series, failed map[string]bool) []Series {
	sum := map[string]Series{}
	for id, agent := range perAgent {
		if failed[id] {
			agent = closeOpen(agent)
		}
		for key, s := range agent {
			cur := sum[key]
			sum[key] = Series{Metric: s.Metric, Labels: s.Labels, Value: cur.Value + s.Value}
		}
	}
	out := make([]Series, 0, len(sum))
	for _, key := range slices.Sorted(maps.Keys(sum)) {
		out = append(out, sum[key])
	}
	return out
}

// totals sums the newest counters and gauges without labels of every agent,
// adds the labeled counters of series that are not of the server side, and
// derives sessions.failed_ratio. For an agent in failed, the sessions that it
// attempted without success or failure count as failed.
func totals(counters map[string]map[string]float64, series []Series, failed map[string]bool) map[string]float64 {
	out := map[string]float64{}
	for id, c := range counters {
		agent := maps.Clone(c)
		if agent == nil {
			agent = map[string]float64{}
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
	// A counter that an agent also reports without labels is a run total
	// that repeats its labeled series, so the labeled series add up only for
	// the counters that no agent reports as a run total.
	reported := maps.Clone(out)
	for _, s := range series {
		if _, total := reported[s.Metric]; total || len(s.Labels) == 0 || s.Labels[LabelSide] == SideServer {
			continue
		}
		out[s.Metric] += s.Value
	}
	delete(out, MetricSessionsFailedRatio)
	if att := out[MetricSessionsAttempted]; att > 0 {
		out[MetricSessionsFailedRatio] = out[MetricSessionsFailed] / att
	} else {
		out[MetricSessionsFailedRatio] = 0
	}
	return out
}
