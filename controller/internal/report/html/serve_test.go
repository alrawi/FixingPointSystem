// SPDX-License-Identifier: GPL-3.0-only

package html

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/report"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

var serveT0 = time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)

// serveRuns is a report.RunSource over fixed runs.
type serveRuns map[string]runs.Run

func (f serveRuns) Get(id string) (runs.Run, error) {
	r, ok := f[id]
	if !ok {
		return runs.Run{}, fmt.Errorf("get run %s: %w", id, runs.ErrRunNotFound)
	}
	return r, nil
}

// serveMetrics is a report.TotalsSource and report.SampleSource. NewAPI must
// find the sample source by itself, as it does for *metrics.Aggregator.
type serveMetrics struct {
	totals     map[string]float64
	samples    []metrics.Sample
	samplesErr error
}

func (m serveMetrics) Totals(id string) (map[string]float64, error) {
	if id == "run-nodata" {
		return nil, fmt.Errorf("totals of run %s: %w", id, metrics.ErrRunUnknown)
	}
	return m.totals, nil
}

func (m serveMetrics) Samples(id string, v api.MetricView, _ metrics.Window) ([]metrics.Sample, error) {
	if v != api.MetricViewApplication {
		return nil, fmt.Errorf("unexpected view %q", v)
	}
	if id == "run-nodata" {
		return nil, fmt.Errorf("samples of run %s: %w", id, metrics.ErrRunUnknown)
	}
	return m.samples, m.samplesErr
}

// serveObjectives is a report.ObjectiveSource over fixed objectives by
// configuration id.
type serveObjectives map[string]report.Objective

func (f serveObjectives) Objective(_ context.Context, id string) (report.Objective, error) {
	o, ok := f[id]
	if !ok {
		return report.Objective{}, errors.New("no such configuration")
	}
	return o, nil
}

func serveFixture() (serveRuns, serveMetrics) {
	finished := runs.Run{
		ID: "run-done", ConfigID: "cfg-1", State: runs.Completed, Phase: "done",
		CreatedAt: serveT0, StartedAt: serveT0.Add(time.Second), EndedAt: serveT0.Add(61 * time.Second),
		Seed: 7, EngineVersion: "0.1.0", ModuleVersions: map[string]string{"http1": "0.1.0"},
		Agents: []string{"agent-a"},
		Verdict: &runs.Verdict{Passed: true, Criteria: []runs.CriterionResult{{
			Criterion: runs.Criterion{CriterionConfig: compiler.CriterionConfig{Metric: "sessions.failed_ratio", Op: "<", Value: 0.01}},
		}}},
	}
	rs := serveRuns{"run-done": finished}
	for _, id := range []string{"run-orphan", "run-nodata", "run-broken"} {
		r := finished
		r.ID = id
		if id == "run-orphan" {
			r.ConfigID = "cfg-gone"
		}
		rs[id] = r
	}
	var samples []metrics.Sample
	for i := 0; i < 30; i++ {
		samples = append(samples, metrics.Sample{
			Ts: serveT0.Add(time.Duration(i) * time.Second), View: api.MetricViewApplication, Key: metrics.RunKey,
			Values: map[string]float64{"sessions.concurrent": float64(i * 10), "throughput.bps": float64(i) * 1e7},
		})
	}
	return rs, serveMetrics{
		totals:  map[string]float64{"sessions.attempted": 1000, "sessions.succeeded": 1000, "sessions.failed": 0, "sessions.failed_ratio": 0},
		samples: samples,
	}
}

// serveGet serves GET /v1/runs/{id}/report?format=query through the real
// router and token check.
func serveGet(t *testing.T, ops api.ServerInterface, id, query string) *httptest.ResponseRecorder {
	t.Helper()
	auth, err := api.NewStaticTokens(map[string]api.Scope{"token-read": api.ScopeRead})
	if err != nil {
		t.Fatal(err)
	}
	h := api.New(ops, auth, slog.New(slog.DiscardHandler)).Handler()
	r := httptest.NewRequest(http.MethodGet, api.BasePath+"/runs/"+id+"/report?"+query, nil)
	r.Header.Set("Authorization", "Bearer token-read")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, r)
	return rec
}

func newServeAPI(t *testing.T, ms report.TotalsSource) *report.API {
	t.Helper()
	rs, _ := serveFixture()
	ev, err := report.NewEvaluator(nil)
	if err != nil {
		t.Fatal(err)
	}
	return report.NewAPI(ev, rs, ms).
		UseHTML(Renderer).
		UseObjectives(serveObjectives{"cfg-1": {Type: "sessions_per_second", Value: 1000}})
}

func TestServesHTMLReportOfFinishedRun(t *testing.T) {
	_, ms := serveFixture()
	rec := serveGet(t, newServeAPI(t, ms), "run-done", "format=html")
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d: %s", rec.Code, rec.Body)
	}
	for k, want := range map[string]string{
		"Content-Type":            ContentType,
		"Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'",
		"X-Content-Type-Options":  "nosniff",
		"Cache-Control":           "no-store",
	} {
		if got := rec.Header().Get(k); got != want {
			t.Errorf("header %s = %q, want %q", k, got, want)
		}
	}
	body := rec.Body.String()
	for _, want := range []string{"run-done", "sessions per second 1000", "sessions.failed_ratio", "PASS", "<polyline", "Concurrent sessions"} {
		if !strings.Contains(body, want) {
			t.Errorf("page lacks %q", want)
		}
	}
	if v := violations(body); len(v) != 0 {
		t.Errorf("served page makes external requests: %q", v)
	}
	if strings.Contains(body, "http://") || strings.Contains(body, "https://") {
		t.Error("served page mentions an http URL")
	}
}

func TestHTMLReportShowsTheSameRunAsJSON(t *testing.T) {
	_, ms := serveFixture()
	ops := newServeAPI(t, ms)
	js := serveGet(t, ops, "run-done", "format=json")
	if js.Code != http.StatusOK || !strings.Contains(js.Body.String(), `"passed":true`) && !strings.Contains(js.Body.String(), `"passed": true`) {
		t.Fatalf("json: status %d: %s", js.Code, js.Body)
	}
	if got := serveGet(t, ops, "run-done", "format=html"); !strings.Contains(got.Body.String(), "PASS") {
		t.Fatal("html verdict differs from json")
	}
}

func TestHTMLReportOfUnknownRunIsTheJSONNotFound(t *testing.T) {
	_, ms := serveFixture()
	ops := newServeAPI(t, ms)
	js := serveGet(t, ops, "run-missing", "")
	hm := serveGet(t, ops, "run-missing", "format=html")
	if hm.Code != http.StatusNotFound || hm.Code != js.Code {
		t.Fatalf("html status %d, json status %d, want 404 for both", hm.Code, js.Code)
	}
	if hm.Header().Get("Content-Type") != api.ProblemContentType || js.Header().Get("Content-Type") != api.ProblemContentType {
		t.Errorf("types %q and %q, want %q", hm.Header().Get("Content-Type"), js.Header().Get("Content-Type"), api.ProblemContentType)
	}
	if hm.Body.String() != js.Body.String() {
		t.Errorf("html 404 body %q differs from json 404 body %q", hm.Body, js.Body)
	}
	if hm.Header().Get("Content-Security-Policy") != "" {
		t.Error("a problem response carries the page policy")
	}
}

func TestHTMLReportDegradesWithoutSamplesOrObjective(t *testing.T) {
	_, ms := serveFixture()
	ops := newServeAPI(t, ms)

	// The aggregator holds nothing of the run: a page without charts.
	rec := serveGet(t, ops, "run-nodata", "format=html")
	if rec.Code != http.StatusOK || strings.Contains(rec.Body.String(), "<polyline") {
		t.Errorf("run without samples: status %d, polyline %v", rec.Code, strings.Contains(rec.Body.String(), "<polyline"))
	}
	// The configuration is gone: a page without objective.
	rec = serveGet(t, ops, "run-orphan", "format=html")
	if rec.Code != http.StatusOK || strings.Contains(rec.Body.String(), "sessions per second") {
		t.Errorf("run without configuration: status %d", rec.Code)
	}
}

func TestHTMLReportSampleFailureIs500(t *testing.T) {
	_, ms := serveFixture()
	ms.samplesErr = errors.New("disk on fire")
	rec := serveGet(t, newServeAPI(t, ms), "run-done", "format=html")
	if rec.Code != http.StatusInternalServerError || rec.Header().Get("Content-Type") != api.ProblemContentType {
		t.Errorf("status %d, type %q", rec.Code, rec.Header().Get("Content-Type"))
	}
}

func TestHTMLReportIs501WithoutRenderer(t *testing.T) {
	rs, ms := serveFixture()
	ev, err := report.NewEvaluator(nil)
	if err != nil {
		t.Fatal(err)
	}
	rec := serveGet(t, report.NewAPI(ev, rs, ms), "run-done", "format=html")
	if rec.Code != http.StatusNotImplemented {
		t.Errorf("status %d, want 501", rec.Code)
	}
}
