// SPDX-License-Identifier: GPL-3.0-only

package html

import (
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/report"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

func criterion(metric, op string, value float64, where map[string]string) runs.Criterion {
	return runs.Criterion{
		CriterionConfig: compiler.CriterionConfig{Metric: metric, Op: op, Value: value},
		Where:           where,
	}
}

func testReport() report.Report {
	t0 := time.Date(2026, 11, 3, 12, 0, 2, 0, time.UTC)
	return report.Report{
		Run: runs.Run{
			ID: "run-1", ConfigID: "cfg-1", State: runs.Failed, Degraded: true, Phase: "done",
			Reason:    "criteria failed: <b>x</b>",
			CreatedAt: t0.Add(-2 * time.Second), StartedAt: t0, EndedAt: t0.Add(60 * time.Second),
			Seed: 42, EngineVersion: "0.1.0",
			ModuleVersions: map[string]string{"http1": "0.1.0", "dns": "0.2.0"},
			Agents:         []string{"a", "b"}, FailedAgents: []string{"b"},
		},
		Totals: map[string]float64{
			"sessions.attempted": 1000, "sessions.failed": 5, "sessions.failed_ratio": 0.005,
			"throughput.bps": 610e6, "unknown.metric": 1,
		},
		Evaluation: &report.Evaluation{Results: []report.Result{
			{Criterion: criterion("sessions.failed_ratio", "<", 0.001, map[string]string{"side": "client", "application": "dns"}),
				Status: report.Fail, Observed: 0.005, Reason: "observed 0.005 < 0.001 does not hold"},
			{Criterion: criterion("sessions.attempted", ">=", 100, nil), Status: report.Pass, Observed: 1000, Reason: "holds"},
			{Criterion: criterion("mix.max_abs_error_pp", "<=", 2, nil), Status: report.Missing, Observed: 99, Reason: "the run reported no value"},
			{Criterion: criterion("nope", "<", 1, nil), Status: report.Invalid, Reason: "unknown metric"},
		}},
	}
}

func TestFromReport(t *testing.T) {
	p := FromReport(testReport(), Options{Objective: FormatObjective("sessions_per_second", 1000)})
	if p.RunID != "run-1" || p.State != "failed" || !p.Degraded || p.Phase != "done" {
		t.Errorf("run fields = %+v", p)
	}
	if p.Duration != "1m0s" || p.Started != "2026-11-03T12:00:02Z" {
		t.Errorf("duration %q, started %q", p.Duration, p.Started)
	}
	if p.Objective != "sessions per second 1000" {
		t.Errorf("objective = %q", p.Objective)
	}
	if p.Verdict != VerdictFail {
		t.Errorf("verdict = %q", p.Verdict)
	}
	if len(p.Modules) != 2 || p.Modules[0].Name != "dns" {
		t.Errorf("modules = %v", p.Modules)
	}
	for _, kv := range p.Summary {
		if kv.Name == "unknown.metric" {
			t.Error("summary holds a metric outside its list")
		}
	}
	if len(p.Summary) != 4 || p.Summary[3].Value != "610M bit/s" {
		t.Errorf("summary = %v", p.Summary)
	}
	want := []struct{ status, observed, where string }{
		{"fail", "0.005", "application=dns,side=client"},
		{"pass", "1000", ""},
		{"missing", "", ""},
		{"invalid", "", ""},
	}
	if len(p.Criteria) != len(want) {
		t.Fatalf("criteria = %v", p.Criteria)
	}
	for i, w := range want {
		c := p.Criteria[i]
		if c.Status != w.status || c.Observed != w.observed || c.Where != w.where {
			t.Errorf("criterion %d = %+v, want %+v", i, c, w)
		}
	}
	if p.Criteria[0].Threshold != "0.001" || p.Criteria[0].Op != "<" {
		t.Errorf("rule = %+v", p.Criteria[0])
	}
}

func TestFromReportVerdicts(t *testing.T) {
	r := testReport()
	r.Passed = true
	if v := FromReport(r, Options{}).Verdict; v != VerdictPass {
		t.Errorf("passed verdict = %q", v)
	}
	r.Evaluation = nil
	p := FromReport(r, Options{})
	if p.Verdict != VerdictNotEvaluated || len(p.Criteria) != 0 {
		t.Errorf("report without evaluation = %q, %d criteria", p.Verdict, len(p.Criteria))
	}
	if d := duration(time.Time{}, time.Now()); d != "" {
		t.Errorf("duration without start = %q", d)
	}
	if d := duration(time.Now(), time.Now().Add(-time.Hour)); d != "" {
		t.Errorf("duration of a negative span = %q", d)
	}
}

func TestChartsFromSamples(t *testing.T) {
	t0 := time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)
	sample := func(sec int, key string, v map[string]float64) metrics.Sample {
		return metrics.Sample{Ts: t0.Add(time.Duration(sec) * time.Second), View: "application", Key: key, Values: v}
	}
	samples := []metrics.Sample{
		sample(2, metrics.RunKey, map[string]float64{"throughput.bps": 30, "sessions.concurrent": 3}),
		sample(0, metrics.RunKey, map[string]float64{"throughput.bps": 10}),
		sample(0, "dns", map[string]float64{"throughput.bps": 999}),
		sample(1, metrics.RunKey, map[string]float64{"throughput.bps": 20}),
	}
	cs := charts(samples)
	if len(cs) != 2 {
		t.Fatalf("charts = %v", cs)
	}
	var thr Chart
	for _, c := range cs {
		if c.Title == "Throughput" {
			thr = c
		}
	}
	pts := thr.Series[0].Points
	if len(pts) != 3 || pts[0] != (Point{0, 10}) || pts[1] != (Point{1, 20}) || pts[2] != (Point{2, 30}) {
		t.Errorf("throughput points = %v (only the whole run key, in time order)", pts)
	}
	if charts(nil) != nil {
		t.Error("no samples gave charts")
	}
}

func TestRenderFromReportEscapesReason(t *testing.T) {
	r := testReport()
	r.Evaluation.Results[0].Reason = `observed <script>alert(1)</script> does not hold`
	out, err := Render(FromReport(r, Options{Samples: []metrics.Sample{
		{Ts: time.Unix(0, 0), Key: metrics.RunKey, Values: map[string]float64{"throughput.bps": 1}},
		{Ts: time.Unix(1, 0), Key: metrics.RunKey, Values: map[string]float64{"throughput.bps": 2}},
	}}))
	if err != nil {
		t.Fatal(err)
	}
	s := string(out)
	if strings.Contains(s, "<script") || !strings.Contains(s, "&lt;script&gt;alert(1)") {
		t.Error("markup of a criterion reason is not escaped")
	}
	if !strings.Contains(s, "<polyline") || !strings.Contains(s, "Verdict: fail") {
		t.Error("page lacks a chart or the verdict")
	}
	if v := violations(s); len(v) != 0 {
		t.Errorf("external references: %q", v)
	}
}
