// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"encoding/json"
	"maps"
	"os"
	"reflect"
	"testing"

	"go.yaml.in/yaml/v3"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

const checkMetricsPath = "../../../contracts/examples/realism/check-metrics.yaml"

type exampleMetric struct {
	Name   string            `yaml:"name" json:"name"`
	Labels map[string]string `yaml:"labels" json:"labels"`
	Value  float64           `yaml:"value" json:"value"`
}

type exampleResult struct {
	Metric   string            `yaml:"metric"`
	Labels   map[string]string `yaml:"labels"`
	Observed float64           `yaml:"observed"`
	Property string            `yaml:"property"`
	Target   string            `yaml:"target"`
	Passed   bool              `yaml:"passed"`
}

type exampleCase struct {
	Name        string          `yaml:"name"`
	CheckPassed bool            `yaml:"check_passed"`
	Metrics     []exampleMetric `yaml:"metrics"`
	Results     []exampleResult `yaml:"results"`
}

type exampleFile struct {
	Examples []struct {
		Check  string         `yaml:"check"`
		Report map[string]any `yaml:"report"`
		Cases  []exampleCase  `yaml:"cases"`
	} `yaml:"examples"`
}

// number reads a number of an example report, which YAML gives as an int or a
// float.
func number(t *testing.T, v any) float64 {
	t.Helper()
	switch n := v.(type) {
	case int:
		return float64(n)
	case float64:
		return n
	}
	t.Fatalf("%v is not a number", v)
	return 0
}

// exampleCheck builds the check of the realism report that an example case
// describes. The example gives the fields that the target text uses and the
// metrics. The entries that the mapping reads for the outcome (the applications
// of a conformance or content check, the comparison of a temporal check, the
// rows of a mix check) follow from the metrics and the verdict of the case.
func exampleCheck(t *testing.T, kind string, report map[string]any, c exampleCase) map[string]any {
	t.Helper()
	check := map[string]any{"check": kind, "passed": c.CheckPassed, "metrics": c.Metrics}
	maps.Copy(check, report)
	type entry = map[string]any
	switch kind {
	case checkConformance:
		var apps []entry
		for _, m := range c.Metrics {
			if m.Name == metricConformanceRatio {
				apps = append(apps, entry{"application": m.Labels["application"], "passed": m.Value >= number(t, report["threshold"])})
			}
		}
		check["applications"] = apps
	case checkTemporal:
		status := "fail"
		if c.CheckPassed {
			status = "pass"
		}
		check["comparisons"] = []entry{{"property": "inter_arrival", "application": "http1", "alpha": report["alpha"], "status": status}}
	case checkContent:
		var apps []entry
		for _, m := range c.Metrics {
			if m.Name == metricDuplicateRatio {
				apps = append(apps, entry{"application": m.Labels["application"], "passed": m.Value <= number(t, report["max_duplicate_ratio"])})
			}
		}
		check["applications"] = apps
	case checkMixAccuracy:
		// The example says that the configured share of http1 is 60 percent.
		var apps []entry
		for _, m := range c.Metrics {
			if m.Name == metricMixAbsError {
				apps = append(apps, entry{"application": m.Labels["application"], "configured_pct": 60, "ndpi_passed": m.Value <= number(t, report["tolerance_points"])})
			}
		}
		check["bases"] = []entry{{"basis": report["metrics_basis"], "applications": apps}}
	}
	return check
}

func marshalChecks(t *testing.T, checks ...any) []byte {
	t.Helper()
	doc, err := json.Marshal(map[string]any{"checks": checks})
	if err != nil {
		t.Fatal(err)
	}
	return doc
}

// literal is the part of a RealismResult that CON-23 fixes, and the wording in
// property of the metrics that wordedProperty names. The wording of the other
// metrics is for an operator and takes no part in the comparison.
type literal struct {
	Metric   string
	Labels   map[string]string
	Observed float32
	Target   string
	Passed   bool
	Property string
}

// wordedProperty reports whether the controller words the property of metric
// as the examples do. The KS metrics (CTL-46) and the conformance ratio do. The
// other metrics keep the wording of their name and labels, which the examples
// do not follow yet, so the comparison leaves their property out.
func wordedProperty(metric string) bool {
	return metric == metricKSStatistic || metric == metricKSPValue || metric == metricConformanceRatio
}

func propertyOf(metric, property string) string {
	if wordedProperty(metric) {
		return property
	}
	return ""
}

func literals(rs []api.RealismResult) []literal {
	out := make([]literal, 0, len(rs))
	for _, r := range rs {
		l := literal{Metric: *r.Metric, Observed: r.Observed, Target: r.Target, Passed: r.Passed, Property: propertyOf(*r.Metric, r.Property)}
		if r.Labels != nil && len(*r.Labels) > 0 {
			l.Labels = *r.Labels
		}
		out = append(out, l)
	}
	return out
}

// TestMapRealismCON23Examples maps every case of
// contracts/examples/realism/check-metrics.yaml to the literal target and
// passed that the example gives.
func TestMapRealismCON23Examples(t *testing.T) {
	data, err := os.ReadFile(checkMetricsPath)
	if err != nil {
		t.Fatal(err)
	}
	var f exampleFile
	if err := yaml.Unmarshal(data, &f); err != nil {
		t.Fatal(err)
	}
	if len(f.Examples) != 9 {
		t.Fatalf("the examples hold %d check kinds, want 9", len(f.Examples))
	}
	names := map[string]bool{}
	cases := 0
	for _, ex := range f.Examples {
		for _, c := range ex.Cases {
			cases++
			got, findings, err := MapRealism(marshalChecks(t, exampleCheck(t, ex.Check, ex.Report, c)))
			if err != nil || len(findings) != 0 {
				t.Errorf("%s %s: MapRealism = %v, %+v", ex.Check, c.Name, err, findings)
				continue
			}
			var want []literal
			for _, r := range c.Results {
				l := literal{Metric: r.Metric, Observed: float32(r.Observed), Target: r.Target, Passed: r.Passed, Property: propertyOf(r.Metric, r.Property)}
				if len(r.Labels) > 0 {
					l.Labels = r.Labels
				}
				want = append(want, l)
				names[r.Metric] = true
			}
			if have := literals(got); !reflect.DeepEqual(have, want) {
				t.Errorf("%s %s:\n got %+v\nwant %+v", ex.Check, c.Name, have, want)
			}
		}
	}
	if cases != 19 {
		t.Errorf("the examples hold %d cases, want 19", cases)
	}
	// Every row of the table of CON-23 that names a metric has an example.
	for _, name := range []string{
		metricConformanceRatio, metricKSPValue, metricKSStatistic, metricCheckFailures, metricParserErrors,
		metricDecryptedRatio, metricJA4MatchRatio, metricDuplicateRatio, metricStructureFailures,
		metricTransportDeviations, metricMixShare, metricMixAbsError, metricMixMaxAbsError,
	} {
		if !names[name] {
			t.Errorf("no example maps %s", name)
		}
	}
}

// TestMapRealismCheckFailuresTakesTheCheckOfItsLabel checks that the passed of
// realism.check_failures is the outcome of the check that the label check
// names, and that a label which names no check of the document leaves it to the
// check that carries the series.
func TestMapRealismCheckFailuresTakesTheCheckOfItsLabel(t *testing.T) {
	failures := func(check string, v float64) []exampleMetric {
		return []exampleMetric{{Name: metricCheckFailures, Labels: map[string]string{"check": check}, Value: v}}
	}
	doc := marshalChecks(t,
		map[string]any{"check": checkContent, "passed": true, "metrics": failures(checkTemporal, 1)},
		map[string]any{"check": checkTemporal, "passed": false, "metrics": []exampleMetric{}},
		map[string]any{"check": checkEncryption, "passed": false, "metrics": failures("messages", 1)},
	)
	got, findings, err := MapRealism(doc)
	if err != nil || len(findings) != 0 {
		t.Fatalf("MapRealism = %v, %+v", err, findings)
	}
	want := []literal{
		{metricCheckFailures, map[string]string{"check": checkTemporal}, 1, "== 0", false, ""},
		{metricCheckFailures, map[string]string{"check": "messages"}, 1, "== 0", false, ""},
	}
	if have := literals(got); !reflect.DeepEqual(have, want) {
		t.Fatalf("results %+v, want %+v", have, want)
	}
}

// TestMapRealismCON23Fallbacks checks that a metric which the table of CON-23
// does not name for its check, or which finds no entry to take its outcome
// from, keeps the last row: the target `check passed` and the outcome of its
// check.
func TestMapRealismCON23Fallbacks(t *testing.T) {
	classifier := func(app string) map[string]string {
		return map[string]string{"application": app, "source": "classifier", "classifier": "ndpi"}
	}
	row := func(app string, passed bool) map[string]any {
		return map[string]any{"application": app, "configured_pct": 60, "ndpi_passed": passed}
	}
	mix := func(basis any, metrics ...exampleMetric) map[string]any {
		return map[string]any{
			"check": checkMixAccuracy, "passed": false, "metrics": metrics, "tolerance_points": 2, "metrics_basis": basis,
			"bases": []any{map[string]any{"basis": "sessions", "applications": []any{row("http1", true), row("dns", false)}}},
		}
	}
	generic := func(name string, labels map[string]string, v float64, passed bool) literal {
		return literal{name, labels, float32(v), "check passed", passed, ""}
	}
	metricsSource := map[string]string{"application": "http1", "source": "metrics"}
	doc := marshalChecks(t,
		// Another source, a basis that no row carries, and a label without a row.
		mix("sessions",
			exampleMetric{Name: metricMixShare, Labels: metricsSource, Value: 59},
			exampleMetric{Name: metricMixAbsError, Labels: classifier("other"), Value: 1},
			exampleMetric{Name: metricMixMaxAbsError, Labels: map[string]string{"source": "classifier", "classifier": "ndpi"}, Value: 9}),
		mix(nil, exampleMetric{Name: metricMixShare, Labels: classifier("http1"), Value: 59}),
		mix("bytes", exampleMetric{Name: metricMixShare, Labels: classifier("dns"), Value: 59}),
		// A metric of another check kind, and an unknown metric of the kind.
		map[string]any{"check": checkContent, "passed": true, "max_duplicate_ratio": 0.1,
			"applications": []any{},
			"metrics": []exampleMetric{
				{Name: metricDecryptedRatio, Labels: map[string]string{}, Value: 0.1},
				{Name: metricDuplicateRatio, Labels: map[string]string{"application": "http1"}, Value: 0.5},
			}},
		map[string]any{"check": checkParserErrors, "passed": true,
			"metrics": []exampleMetric{{Name: "realism.parser_notes", Labels: map[string]string{}, Value: 3}}},
	)
	got, findings, err := MapRealism(doc)
	if err != nil || len(findings) != 0 {
		t.Fatalf("MapRealism = %v, %+v", err, findings)
	}
	want := []literal{
		generic(metricMixShare, metricsSource, 59, false),
		// The label other has no row in the basis, and the largest error is
		// judged by the rows, of which dns fails.
		generic(metricMixAbsError, classifier("other"), 1, false),
		{metricMixMaxAbsError, map[string]string{"source": "classifier", "classifier": "ndpi"}, 9, "<= 2", false, ""},
		generic(metricMixShare, classifier("http1"), 59, false),
		generic(metricMixShare, classifier("dns"), 59, false),
		generic(metricDecryptedRatio, nil, 0.1, true),
		generic(metricDuplicateRatio, map[string]string{"application": "http1"}, 0.5, true),
		generic("realism.parser_notes", nil, 3, true),
	}
	if have := literals(got); !reflect.DeepEqual(have, want) {
		t.Fatalf("results:\n %+v\nwant:\n %+v", have, want)
	}
}
