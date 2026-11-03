// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"encoding/json"
	"math"
	"os"
	"slices"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

const testConfigSchema = "../../../contracts/schemas/test-config.schema.json"

func newEvaluator(t *testing.T) *Evaluator {
	t.Helper()
	e, err := NewEvaluator(nil)
	if err != nil {
		t.Fatal(err)
	}
	return e
}

func crit(metric, op string, value float64) compiler.CriterionConfig {
	return compiler.CriterionConfig{Metric: metric, Op: op, Value: value}
}

func TestOperatorsMatchTheSchema(t *testing.T) {
	data, err := os.ReadFile(testConfigSchema)
	if err != nil {
		t.Fatal(err)
	}
	var doc struct {
		Properties struct {
			Criteria struct {
				Items struct {
					Properties struct {
						Op struct {
							Enum []string `json:"enum"`
						} `json:"op"`
					} `json:"properties"`
				} `json:"items"`
			} `json:"criteria"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(data, &doc); err != nil {
		t.Fatal(err)
	}
	if got := doc.Properties.Criteria.Items.Properties.Op.Enum; !slices.Equal(got, Operators) {
		t.Fatalf("schema operators %v, package operators %v", got, Operators)
	}
}

func TestEveryOperator(t *testing.T) {
	e := newEvaluator(t)
	totals := map[string]float64{"bytes.tx": 10}
	cases := []struct {
		op    string
		value float64
		want  Status
	}{
		{"<", 11, Pass}, {"<", 10, Fail},
		{"<=", 10, Pass}, {"<=", 9, Fail},
		{">", 9, Pass}, {">", 10, Fail},
		{">=", 10, Pass}, {">=", 11, Fail},
		{"==", 10, Pass}, {"==", 10.5, Fail},
		{"!=", 11, Pass}, {"!=", 10, Fail},
		{"=~", 10, Invalid}, {"", 10, Invalid},
	}
	for _, c := range cases {
		ev := e.Evaluate([]compiler.CriterionConfig{crit("bytes.tx", c.op, c.value)}, totals)
		res := ev.Results[0]
		if res.Status != c.want {
			t.Errorf("bytes.tx %s %v: status %s, want %s (%s)", c.op, c.value, res.Status, c.want, res.Reason)
		}
		if ev.Passed != (c.want == Pass) {
			t.Errorf("bytes.tx %s %v: passed %v", c.op, c.value, ev.Passed)
		}
		if res.Evaluated() && res.Observed != 10 {
			t.Errorf("bytes.tx %s %v: observed %v", c.op, c.value, res.Observed)
		}
	}
}

func TestRatioComesFromMergedCounters(t *testing.T) {
	e := newEvaluator(t)
	// Two agents with ratios 0.5 and 0.0 average 0.25, but the run failed
	// 10 of 1010 sessions. The catalog formula wins over any stored ratio.
	totals := map[string]float64{
		"sessions.attempted": 1010, "sessions.failed": 10, "sessions.failed_ratio": 0.25,
		"tls.handshakes": 200, "tls.resumptions": 50,
	}
	ev := e.Evaluate([]compiler.CriterionConfig{
		crit("sessions.failed_ratio", "<", 0.01),
		crit("tls.resumption_ratio", "==", 0.25),
	}, totals)
	if !ev.Passed {
		t.Fatalf("evaluation failed: %+v", ev.Results)
	}
	if got := ev.Results[0].Observed; math.Abs(got-10.0/1010) > 1e-15 {
		t.Fatalf("sessions.failed_ratio observed %v", got)
	}
	if totals["sessions.failed_ratio"] != 0.25 {
		t.Fatal("Evaluate changed its input")
	}
}

func TestEveryCatalogFormulaIsDerived(t *testing.T) {
	e := newEvaluator(t)
	n := 0
	for _, m := range e.Catalog().Metrics() {
		if m.Formula == "" {
			continue
		}
		n++
		f := strings.Fields(m.Formula)
		got := e.Derive(map[string]float64{f[0]: 3, f[2]: 4})
		want, _ := apply(f[1][0], 3, 4)
		if got[m.Name] != want {
			t.Errorf("%s = %s: derived %v, want %v", m.Name, m.Formula, got[m.Name], want)
		}
	}
	if n < 2 {
		t.Fatalf("the catalog has %d formulas, want at least 2", n)
	}
}

func TestUndefinedRatioIsMissing(t *testing.T) {
	e := newEvaluator(t)
	ev := e.Evaluate([]compiler.CriterionConfig{
		crit("sessions.failed_ratio", "==", 0),
		crit("tls.resumption_ratio", ">", 0.1),
		crit("mix.max_abs_error_pp", "<=", 2),
	}, map[string]float64{"sessions.attempted": 0, "sessions.failed": 0, "sessions.failed_ratio": 0})
	if !ev.Passed {
		t.Fatalf("missing criteria decided the verdict: %+v", ev.Results)
	}
	want := []string{
		"undefined because sessions.attempted is 0",
		"undefined because the run reported no tls.handshakes",
		"the run reported no value",
	}
	for i, res := range ev.Results {
		if res.Status != Missing || res.Reason != want[i] {
			t.Errorf("criterion %d: %s %q, want missing %q", i, res.Status, res.Reason, want[i])
		}
	}
}

func TestMetricReferences(t *testing.T) {
	e := newEvaluator(t)
	totals := map[string]float64{"sessions.response_time.p99": 900, "sessions.response_time.count": 5, "bytes.tx": 1}
	cases := []struct {
		metric string
		want   Status
		reason string
	}{
		{"sessions.response_time.p99", Pass, ""},
		{"sessions.response_time.count", Pass, ""},
		{"sessions.response_time.p50", Missing, "the run reported no value"},
		{"sessions.response_time", Invalid, "histogram sessions.response_time needs a statistic suffix such as .p99"},
		{"sessions.response_time.p42", Invalid, "metric sessions.response_time.p42 is not in the metric catalog"},
		{"bytes.tx.p99", Invalid, "statistic p99 applies only to histograms, and bytes.tx is a counter"},
		{"bogus.metric", Invalid, "metric bogus.metric is not in the metric catalog"},
		{"nodot", Invalid, "metric nodot is not in the metric catalog"},
	}
	for _, c := range cases {
		res := e.Evaluate([]compiler.CriterionConfig{crit(c.metric, ">=", 1)}, totals).Results[0]
		if res.Status != c.want || (c.reason != "" && res.Reason != c.reason) {
			t.Errorf("%s: %s %q, want %s %q", c.metric, res.Status, res.Reason, c.want, c.reason)
		}
	}
}

func TestNaNIsMissing(t *testing.T) {
	e := newEvaluator(t)
	res := e.Evaluate([]compiler.CriterionConfig{crit("throughput.bps", "!=", 0)}, map[string]float64{"throughput.bps": math.NaN()}).Results[0]
	if res.Status != Missing {
		t.Fatalf("status %s", res.Status)
	}
}

func TestNewEvaluatorRejectsBadFormula(t *testing.T) {
	cat, err := metrics.ParseCatalog([]byte(`version: 0.1.0
metrics:
  - { name: a.b, type: counter }
  - { name: a.c, type: gauge, formula: a.b }
`))
	if err == nil {
		_, err = NewEvaluator(cat)
	}
	if err == nil {
		t.Fatal("a formula without an operator was accepted")
	}
}

func TestRunEvaluator(t *testing.T) {
	e := newEvaluator(t)
	eval := e.RunEvaluator()
	criteria := []compiler.CriterionConfig{
		crit("sessions.failed_ratio", "<", 0.01),
		crit("mix.max_abs_error_pp", "<=", 2),
		crit("bytes.tx", ">", 0),
	}
	totals := map[string]float64{"sessions.attempted": 100, "sessions.failed": 0, "bytes.tx": 5}
	v := eval(runs.Outcome{Criteria: criteria, Totals: totals, Agents: 2, FailedAgents: 1})
	if !v.Passed || len(v.Criteria) != 3 {
		t.Fatalf("verdict %+v", v)
	}
	if !v.Criteria[0].Evaluated || !v.Criteria[0].Passed || v.Criteria[0].Observed != 0 {
		t.Errorf("ratio criterion %+v", v.Criteria[0])
	}
	if v.Criteria[1].Evaluated {
		t.Errorf("missing criterion was evaluated: %+v", v.Criteria[1])
	}
	if v := eval(runs.Outcome{Criteria: criteria, Totals: totals, Agents: 2, FailedAgents: 2}); v.Passed {
		t.Error("a run in which every agent failed passed")
	}
	if v := eval(runs.Outcome{Criteria: []compiler.CriterionConfig{crit("bogus.metric", "<", 1)}, Totals: totals, Agents: 1}); v.Passed || !v.Criteria[0].Evaluated || v.Criteria[0].Passed {
		t.Errorf("an invalid criterion did not fail the run: %+v", v)
	}
	if v := eval(runs.Outcome{Totals: totals}); !v.Passed {
		t.Error("a run without criteria and agents failed")
	}
}

// TestRunEvaluatorAgreesWithEvaluateTotals checks that the evaluator decides
// like the default of internal/runs wherever both evaluate a criterion.
func TestRunEvaluatorAgreesWithEvaluateTotals(t *testing.T) {
	e := newEvaluator(t)
	totals := map[string]float64{"sessions.attempted": 1000, "sessions.failed": 3, "bytes.tx": 7, "bytes.rx": 9}
	totals["sessions.failed_ratio"] = totals["sessions.failed"] / totals["sessions.attempted"]
	for _, op := range Operators {
		for _, value := range []float64{0, 0.003, 0.01, 7, 9} {
			for _, metric := range []string{"sessions.failed_ratio", "bytes.tx", "bytes.rx"} {
				o := runs.Outcome{Criteria: []compiler.CriterionConfig{crit(metric, op, value)}, Totals: totals, Agents: 1}
				got, want := e.RunEvaluator()(o), runs.EvaluateTotals(o)
				if got.Passed != want.Passed || got.Criteria[0] != want.Criteria[0] {
					t.Errorf("%s %s %v: got %+v, want %+v", metric, op, value, got, want)
				}
			}
		}
	}
}
