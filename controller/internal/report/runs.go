// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// RunEvaluator returns the runs.Evaluator that the orchestrator calls when
// a run stops. It evaluates every criterion with the catalog formulas, so
// ratio gauges such as sessions.failed_ratio come from the merged counters,
// and it fails a run in which every agent failed, as runs.EvaluateTotals
// does. A criterion whose metric the run lacks does not decide the verdict,
// and an invalid criterion fails it.
//
// The evaluator decides as the report does. It reads the series of the run
// from the source of UseSeries by runs.Outcome.RunID, merges the counters
// that the orchestrator holds, and so applies the where filter, the side
// default, and the histogram statistics. Without such data it falls back to
// runs.Outcome.Totals, which hold no histograms. NewAPI sets the source from
// its totals source, so the binary needs only the call that it already makes.
//
// The binary passes it as runs.Config.Evaluator.
func (e *Evaluator) RunEvaluator() runs.Evaluator {
	return func(o runs.Outcome) runs.Verdict {
		ev := e.EvaluateData(o.Criteria, e.outcomeData(o))
		return ToVerdict(ev, o.Agents == 0 || o.FailedAgents < o.Agents)
	}
}

// outcomeData returns the data that the criteria of a stopped run are
// evaluated over.
func (e *Evaluator) outcomeData(o runs.Outcome) Data {
	var agg []*metrics.Series
	found := false
	if src := e.source(); src != nil && o.RunID != "" {
		// An error means that the source holds no metrics of the run, and
		// the counters of the orchestrator then stand alone.
		if ss, err := src.Series(o.RunID); err == nil {
			agg, found = ss, true
		}
	}
	if !found && len(o.Series) == 0 {
		return Data{Totals: o.Totals}
	}
	return Data{Series: e.mergeSeries(agg, o.Series, o.Totals), Labeled: true}
}

// ToVerdict converts an Evaluation to the verdict type of internal/runs. A
// false alive fails the verdict whatever the criteria say.
func ToVerdict(ev Evaluation, alive bool) runs.Verdict {
	v := runs.Verdict{Passed: ev.Passed && alive, Criteria: make([]runs.CriterionResult, 0, len(ev.Results))}
	for _, r := range ev.Results {
		cr := runs.CriterionResult{Criterion: r.Criterion, Reason: r.Reason}
		switch r.Status {
		case Pass, Fail:
			cr.Evaluated, cr.Observed, cr.Passed = true, r.Observed, r.Status == Pass
		case Invalid:
			cr.Invalid = true
		case Missing:
		}
		v.Criteria = append(v.Criteria, cr)
	}
	return v
}
