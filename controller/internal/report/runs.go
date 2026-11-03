// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// RunEvaluator returns the runs.Evaluator that the orchestrator calls when
// a run stops. It evaluates every criterion over runs.Outcome.Totals with the
// catalog formulas, so ratio gauges such as sessions.failed_ratio come from
// the merged counters, and it fails a run in which every agent failed, as
// runs.EvaluateTotals does. A criterion whose metric the totals lack does not
// decide the verdict, and an invalid criterion fails it.
//
// The binary passes it as runs.Config.Evaluator.
func (e *Evaluator) RunEvaluator() runs.Evaluator {
	return func(o runs.Outcome) runs.Verdict {
		ev := e.Evaluate(o.Criteria, o.Totals)
		return ToVerdict(ev, o.Agents == 0 || o.FailedAgents < o.Agents)
	}
}

// ToVerdict converts an Evaluation to the verdict type of internal/runs. A
// false alive fails the verdict whatever the criteria say.
func ToVerdict(ev Evaluation, alive bool) runs.Verdict {
	v := runs.Verdict{Passed: ev.Passed && alive, Criteria: make([]runs.CriterionResult, 0, len(ev.Results))}
	for _, r := range ev.Results {
		cr := runs.CriterionResult{CriterionConfig: r.CriterionConfig}
		switch r.Status {
		case Pass, Fail:
			cr.Evaluated, cr.Observed, cr.Passed = true, r.Observed, r.Status == Pass
		case Invalid:
			cr.Evaluated = true
		case Missing:
		}
		v.Criteria = append(v.Criteria, cr)
	}
	return v
}
