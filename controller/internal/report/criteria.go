// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"fmt"
	"maps"
	"math"
	"slices"
	"strconv"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
)

// Status is the result of one criterion.
type Status string

// The four results of a criterion. Only Fail and Invalid fail the run.
const (
	// Pass means that the observed value satisfies the rule.
	Pass Status = "pass"
	// Fail means that the observed value breaks the rule.
	Fail Status = "fail"
	// Missing means that the run has no value for a catalog metric, for
	// example a ratio whose divisor is zero or a gauge that no component
	// computed. Such a criterion does not decide the verdict.
	Missing Status = "missing"
	// Invalid means that the criterion names no catalog metric or statistic,
	// or an unknown operator. Such a criterion fails the run, because the
	// rule cannot hold.
	Invalid Status = "invalid"
)

// Operators lists the criterion operators of
// contracts/schemas/test-config.schema.json in schema order.
var Operators = []string{"<", "<=", ">", ">=", "==", "!="}

// Result is the result of one criterion.
type Result struct {
	compiler.CriterionConfig
	// Status is the result.
	Status Status
	// Observed is the value of the metric. It is meaningful when Status is
	// Pass or Fail.
	Observed float64
	// Reason explains the status in one lowercase sentence.
	Reason string
}

// Evaluated reports whether the criterion decided by comparing a value.
func (r Result) Evaluated() bool { return r.Status == Pass || r.Status == Fail }

// Evaluation is the result of the criteria of a run.
type Evaluation struct {
	// Passed is true when no criterion failed or was invalid.
	Passed bool
	// Results holds one result per criterion, in configuration order.
	Results []Result
}

// Evaluator evaluates criteria with the metric names of a catalog.
type Evaluator struct {
	cat *metrics.Catalog
	// formulas holds the parsed formulas of the derived gauges, in catalog
	// order, so a formula over another derived gauge sees its new value.
	formulas []derived
	stats    map[string]bool
}

// derived is one derived gauge of the catalog.
type derived struct {
	name, left, right string
	op                byte
}

// NewEvaluator returns an Evaluator over cat. A nil cat means
// metrics.DefaultCatalog.
func NewEvaluator(cat *metrics.Catalog) (*Evaluator, error) {
	if cat == nil {
		c, err := metrics.DefaultCatalog()
		if err != nil {
			return nil, fmt.Errorf("criteria evaluator: %w", err)
		}
		cat = c
	}
	e := &Evaluator{cat: cat, stats: map[string]bool{}}
	for _, s := range metrics.Stats {
		e.stats[s.Name] = true
	}
	for _, m := range cat.Metrics() {
		if m.Formula == "" {
			continue
		}
		f := strings.Fields(m.Formula)
		if len(f) != 3 || len(f[1]) != 1 || !strings.Contains("+-*/", f[1]) {
			return nil, fmt.Errorf("criteria evaluator: metric %s: formula %q is not of the form <metric> <op> <metric>", m.Name, m.Formula)
		}
		e.formulas = append(e.formulas, derived{name: m.Name, left: f[0], op: f[1][0], right: f[2]})
	}
	return e, nil
}

// Catalog returns the metric catalog of e.
func (e *Evaluator) Catalog() *metrics.Catalog { return e.cat }

// Derive returns a copy of totals in which every derived gauge of the
// catalog, such as sessions.failed_ratio, is recomputed from the merged
// counters with its catalog formula. A derived gauge whose divisor is zero or
// whose divisor is absent leaves the result, because its value is undefined.
// A missing dividend counts as zero, as in the metrics aggregator.
func (e *Evaluator) Derive(totals map[string]float64) map[string]float64 {
	out := maps.Clone(totals)
	if out == nil {
		out = map[string]float64{}
	}
	for _, f := range e.formulas {
		r, ok := out[f.right]
		if !ok {
			delete(out, f.name)
			continue
		}
		l := out[f.left]
		v, ok := apply(f.op, l, r)
		if !ok {
			delete(out, f.name)
			continue
		}
		out[f.name] = v
	}
	return out
}

func apply(op byte, l, r float64) (float64, bool) {
	switch op {
	case '+':
		return l + r, true
	case '-':
		return l - r, true
	case '*':
		return l * r, true
	case '/':
		if r == 0 {
			return 0, false
		}
		return l / r, true
	}
	return 0, false
}

// Evaluate evaluates criteria over totals, which hold run metrics keyed by
// catalog name with histogram statistics under <metric>.<stat>. It derives
// the catalog formulas first, so a criterion on a ratio sees the ratio of the
// merged counters and never a mean of per agent ratios.
func (e *Evaluator) Evaluate(criteria []compiler.CriterionConfig, totals map[string]float64) Evaluation {
	values := e.Derive(totals)
	ev := Evaluation{Passed: true, Results: make([]Result, 0, len(criteria))}
	for _, c := range criteria {
		res := e.one(c, values)
		if res.Status == Fail || res.Status == Invalid {
			ev.Passed = false
		}
		ev.Results = append(ev.Results, res)
	}
	return ev
}

// one evaluates one criterion over the derived values.
func (e *Evaluator) one(c compiler.CriterionConfig, values map[string]float64) Result {
	res := Result{CriterionConfig: c}
	if !slices.Contains(Operators, c.Op) {
		res.Status, res.Reason = Invalid, fmt.Sprintf("unknown operator %q", c.Op)
		return res
	}
	if reason := e.checkName(c.Metric); reason != "" {
		res.Status, res.Reason = Invalid, reason
		return res
	}
	obs, ok := values[c.Metric]
	if !ok || math.IsNaN(obs) {
		res.Status, res.Reason = Missing, e.missingReason(c.Metric, values)
		return res
	}
	res.Observed = obs
	if compare(obs, c.Op, c.Value) {
		res.Status = Pass
		res.Reason = fmt.Sprintf("observed %s %s %s holds", formatNumber(obs), c.Op, formatNumber(c.Value))
	} else {
		res.Status = Fail
		res.Reason = fmt.Sprintf("observed %s %s %s does not hold", formatNumber(obs), c.Op, formatNumber(c.Value))
	}
	return res
}

// checkName returns why name is no metric reference of the catalog, or "".
// A histogram needs a statistic suffix, and only a histogram takes one.
func (e *Evaluator) checkName(name string) string {
	if m := e.cat.Lookup(name); m != nil {
		if m.Kind == metrics.Histogram {
			return fmt.Sprintf("histogram %s needs a statistic suffix such as .p99", name)
		}
		return ""
	}
	base, stat, ok := cutLast(name)
	if ok && e.stats[stat] {
		m := e.cat.Lookup(base)
		switch {
		case m == nil:
		case m.Kind == metrics.Histogram:
			return ""
		default:
			return fmt.Sprintf("statistic %s applies only to histograms, and %s is a %s", stat, base, m.Kind)
		}
	}
	return fmt.Sprintf("metric %s is not in the metric catalog", name)
}

// missingReason explains why a valid metric has no value.
func (e *Evaluator) missingReason(name string, values map[string]float64) string {
	for _, f := range e.formulas {
		if f.name != name {
			continue
		}
		if r, ok := values[f.right]; ok && r == 0 && f.op == '/' {
			return fmt.Sprintf("undefined because %s is 0", f.right)
		}
		return fmt.Sprintf("undefined because the run reported no %s", f.right)
	}
	return "the run reported no value"
}

func cutLast(name string) (base, last string, ok bool) {
	i := strings.LastIndexByte(name, '.')
	if i < 0 {
		return "", "", false
	}
	return name[:i], name[i+1:], true
}

// compare applies the criterion operator op.
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

// formatNumber renders v in the shortest form that parses back to v.
func formatNumber(v float64) string { return strconv.FormatFloat(v, 'g', -1, 64) }
