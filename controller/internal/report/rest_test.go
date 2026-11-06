// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	"context"
	"encoding/csv"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/getkin/kin-openapi/openapi3"
	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"

	golden "github.com/alrawi/FixingPointSystem/tests/golden/go"
)

const (
	contractPath = "../../../contracts/openapi/fps-api-v1.yaml"
	reportPath   = "/runs/{id}/report"
)

var t0 = time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)

// fakeRuns is a RunSource over fixed runs.
type fakeRuns map[string]runs.Run

func (f fakeRuns) Get(id string) (runs.Run, error) {
	r, ok := f[id]
	if !ok {
		return runs.Run{}, fmt.Errorf("get run %s: %w", id, runs.ErrRunNotFound)
	}
	return r, nil
}

// fakeTotals is a TotalsSource over fixed totals.
type fakeTotals map[string]map[string]float64

func (f fakeTotals) Totals(id string) (map[string]float64, error) {
	if id == "run-broken" {
		return nil, errors.New("disk on fire")
	}
	t, ok := f[id]
	if !ok {
		return nil, fmt.Errorf("totals of run %s: %w", id, metrics.ErrRunUnknown)
	}
	return t, nil
}

// verdictOf returns a stored verdict with the criteria configuration, as the
// orchestrator leaves it after the stop.
func verdictOf(passed bool, cs ...runs.Criterion) *runs.Verdict {
	v := &runs.Verdict{Passed: passed}
	for _, c := range cs {
		v.Criteria = append(v.Criteria, runs.CriterionResult{Criterion: c})
	}
	return v
}

func baseRun(id string, state runs.State, agents ...string) runs.Run {
	return runs.Run{
		ID: id, ConfigID: "cfg-enterprise", State: state,
		Phase: "done", Labels: map[string]string{"team": "qa"},
		CreatedAt: t0, StartedAt: t0.Add(2 * time.Second), EndedAt: t0.Add(62 * time.Second),
		Seed: 42, EngineVersion: "0.1.0", ModuleVersions: map[string]string{"http1": "0.1.0", "dns": "0.1.0"},
		Agents: agents,
	}
}

// fixtures returns the three golden runs and their aggregator totals.
func fixtures() (fakeRuns, fakeTotals) {
	criteria := []runs.Criterion{
		crit("sessions.failed_ratio", "<", 0.001),
		crit("sessions.response_time.p99", "<=", 50000),
		crit("throughput.bps", ">=", 1e9),
		crit("mix.max_abs_error_pp", "<=", 2),
	}
	passing := baseRun("run-pass", runs.Completed, "agent-a", "agent-b")
	passing.Totals = map[string]float64{"sessions.attempted": 200000, "sessions.succeeded": 199990, "sessions.failed": 10, "sessions.failed_ratio": 0.00005, "bytes.tx": 4e9, "bytes.rx": 3.5e10}
	passing.Verdict = verdictOf(true, criteria...)

	failing := baseRun("run-fail", runs.Failed, "agent-a", "agent-b")
	failing.Reason = "criteria failed: sessions.failed_ratio: observed 0.005 < 0.001 does not hold; " +
		"sessions.response_time.p99: observed 87000 <= 50000 does not hold; " +
		"throughput.bps: observed 610000000 >= 1000000000 does not hold"
	failing.Totals = map[string]float64{"sessions.attempted": 100000, "sessions.succeeded": 99500, "sessions.failed": 500, "sessions.failed_ratio": 0.005, "bytes.tx": 2e9, "bytes.rx": 1.7e10}
	failing.Verdict = verdictOf(false, criteria...)

	// Agent b crashed with 300 sessions open. The orchestrator counts them
	// as failed, while the aggregator only saw the 20 that agent b reported.
	degraded := baseRun("run-degraded", runs.Failed, "agent-a", "agent-b")
	degraded.Degraded = true
	degraded.FailedAgents = []string{"agent-b"}
	degraded.Reason = "criteria failed: sessions.failed_ratio: observed 0.005333333333333333 < 0.001 does not hold"
	degraded.Totals = map[string]float64{"sessions.attempted": 60000, "sessions.succeeded": 59680, "sessions.failed": 320, "sessions.failed_ratio": 320.0 / 60000, "bytes.tx": 1e9, "bytes.rx": 8e9}
	degraded.Verdict = verdictOf(false, criteria[0], crit("sessions.failed_ratio", "<", 0.01), crit("sessions.response_time.p99", "<=", 50000))

	running := baseRun("run-live", runs.Running, "agent-a")
	running.EndedAt = time.Time{}
	running.Phase = "sustain"

	rs := fakeRuns{"run-pass": passing, "run-fail": failing, "run-degraded": degraded, "run-live": running, "run-broken": baseRun("run-broken", runs.Running)}
	hist := func(p99, mean float64) map[string]float64 {
		return map[string]float64{
			"sessions.response_time.p50": mean * 0.8, "sessions.response_time.p90": p99 * 0.7, "sessions.response_time.p99": p99,
			"sessions.response_time.p999": p99 * 1.5, "sessions.response_time.min": 120, "sessions.response_time.max": p99 * 3,
			"sessions.response_time.mean": mean, "sessions.response_time.count": 150000,
		}
	}
	with := func(base map[string]float64, extra map[string]float64) map[string]float64 {
		for k, v := range extra {
			base[k] = v
		}
		return base
	}
	ts := fakeTotals{
		"run-pass": with(hist(41000, 9000), map[string]float64{
			"sessions.attempted": 200000, "sessions.succeeded": 199990, "sessions.failed": 10, "sessions.failed_ratio": 0.00005,
			"bytes.tx": 4e9, "bytes.rx": 3.5e10, "throughput.bps": 1.25e9, "sessions.rate": 3333.5, "sessions.concurrent": 812,
		}),
		"run-fail": with(hist(87000, 15000), map[string]float64{
			"sessions.attempted": 100000, "sessions.succeeded": 99500, "sessions.failed": 500, "sessions.failed_ratio": 0.005,
			"bytes.tx": 2e9, "bytes.rx": 1.7e10, "throughput.bps": 6.1e8, "sessions.rate": 1666.25, "sessions.concurrent": 640,
		}),
		"run-degraded": with(hist(48000, 11000), map[string]float64{
			"sessions.attempted": 60000, "sessions.succeeded": 59680, "sessions.failed": 20, "sessions.failed_ratio": 20.0 / 60000,
			"bytes.tx": 1e9, "bytes.rx": 8e9, "throughput.bps": 1.2e9, "sessions.concurrent": 300,
		}),
		"run-live": {"sessions.attempted": 50, "sessions.failed": 0},
	}
	return rs, ts
}

type harness struct {
	doc     *openapi3.T
	handler http.Handler
}

func newHarness(t *testing.T, rs RunSource, ts TotalsSource) *harness {
	t.Helper()
	doc, err := contract.Load(contractPath)
	if err != nil {
		t.Fatal(err)
	}
	auth, err := api.NewStaticTokens(map[string]api.Scope{"token-read": api.ScopeRead})
	if err != nil {
		t.Fatal(err)
	}
	ops := NewAPI(newEvaluator(t), rs, ts)
	return &harness{doc: doc, handler: api.New(ops, auth, slog.New(slog.DiscardHandler)).Handler()}
}

// get calls GET /v1/runs/{id}/report with query and validates the exchange
// against the contract.
func (h *harness) get(t *testing.T, id, query string) *httptest.ResponseRecorder {
	t.Helper()
	item := h.doc.Paths.Value(reportPath)
	if item == nil || item.Get == nil {
		t.Fatalf("the contract has no GET %s", reportPath)
	}
	url := api.BasePath + strings.ReplaceAll(reportPath, "{id}", id)
	if query != "" {
		url += "?" + query
	}
	newReq := func() *http.Request {
		r := httptest.NewRequest(http.MethodGet, url, nil)
		r.Header.Set("Authorization", "Bearer token-read")
		return r
	}
	rec := httptest.NewRecorder()
	h.handler.ServeHTTP(rec, newReq())
	in := &openapi3filter.RequestValidationInput{
		Request:    newReq(),
		PathParams: map[string]string{"id": id},
		Route:      &routers.Route{Spec: h.doc, Path: reportPath, PathItem: item, Method: http.MethodGet, Operation: item.Get},
		Options:    &openapi3filter.Options{AuthenticationFunc: openapi3filter.NoopAuthenticationFunc},
	}
	out := &openapi3filter.ResponseValidationInput{
		RequestValidationInput: in,
		Status:                 rec.Code,
		Header:                 rec.Header(),
		Body:                   io.NopCloser(bytes.NewReader(rec.Body.Bytes())),
		Options:                &openapi3filter.Options{IncludeResponseStatus: true},
	}
	if err := openapi3filter.ValidateResponse(context.Background(), out); err != nil {
		t.Fatalf("GET %s: response breaks the contract: %v\nbody: %s", url, err, rec.Body)
	}
	return rec
}

func TestGoldenReports(t *testing.T) {
	rs, ts := fixtures()
	h := newHarness(t, rs, ts)
	for _, name := range []string{"pass", "fail", "degraded"} {
		id := "run-" + name
		t.Run(name, func(t *testing.T) {
			rec := h.get(t, id, "")
			if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != JSONContentType {
				t.Fatalf("json: status %d, type %q: %s", rec.Code, rec.Header().Get("Content-Type"), rec.Body)
			}
			golden.Assert(t, name+".json", rec.Body.Bytes())
			if explicit := h.get(t, id, "format=json"); !bytes.Equal(explicit.Body.Bytes(), rec.Body.Bytes()) {
				t.Fatal("format=json differs from the default format")
			}
			rec = h.get(t, id, "format=csv")
			if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != CSVContentType {
				t.Fatalf("csv: status %d, type %q: %s", rec.Code, rec.Header().Get("Content-Type"), rec.Body)
			}
			if got, want := rec.Header().Get("Content-Disposition"), `attachment; filename="report-`+id+`.csv"`; got != want {
				t.Fatalf("content disposition %q, want %q", got, want)
			}
			golden.Assert(t, name+".csv", rec.Body.Bytes())
		})
	}
}

// verdictOfReport decodes the verdict of a JSON report.
func verdictOfReport(t *testing.T, body []byte) api.RunReport {
	t.Helper()
	var rep api.RunReport
	if err := json.Unmarshal(body, &rep); err != nil {
		t.Fatal(err)
	}
	return rep
}

func TestGoldenVerdicts(t *testing.T) {
	rs, ts := fixtures()
	h := newHarness(t, rs, ts)
	cases := map[string]struct {
		passed  bool
		results []*bool
	}{
		// The mix gauge is missing, so it shows neither observed nor passed.
		"run-pass": {true, []*bool{ptr(true), ptr(true), ptr(true), nil}},
		"run-fail": {false, []*bool{ptr(false), ptr(false), ptr(false), nil}},
		// The orchestrator's 320 failed sessions give 0.0053, which breaks
		// the first criterion and meets the second.
		"run-degraded": {false, []*bool{ptr(false), ptr(true), ptr(true)}},
	}
	for id, c := range cases {
		rep := verdictOfReport(t, h.get(t, id, "").Body.Bytes())
		v := rep.Run.Verdict
		if v == nil || v.Passed == nil || *v.Passed != c.passed || v.Criteria == nil || len(*v.Criteria) != len(c.results) {
			t.Fatalf("%s: verdict %+v", id, v)
		}
		for i, want := range c.results {
			got := (*v.Criteria)[i]
			switch {
			case want == nil && (got.Passed != nil || got.Observed != nil):
				t.Errorf("%s criterion %d: missing metric shows %v and %v", id, i, got.Passed, got.Observed)
			case want != nil && (got.Passed == nil || *got.Passed != *want || got.Observed == nil):
				t.Errorf("%s criterion %d: passed %v, want %v", id, i, got.Passed, *want)
			}
		}
		if id == "run-degraded" {
			if rep.Run.Degraded == nil || !*rep.Run.Degraded || *rep.Run.Summary.SessionsFailed != 320 {
				t.Errorf("degraded run lost the orchestrator counters: %+v", rep.Run.Summary)
			}
		}
	}
}

func TestReportCSVParses(t *testing.T) {
	rs, ts := fixtures()
	h := newHarness(t, rs, ts)
	rows, err := csv.NewReader(bytes.NewReader(h.get(t, "run-fail", "format=csv").Body.Bytes())).ReadAll()
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(rows[0], ",") != strings.Join(CSVHeader, ",") {
		t.Fatalf("header %v", rows[0])
	}
	statuses := map[string]int{}
	for _, row := range rows[1:] {
		if len(row) != len(CSVHeader) {
			t.Fatalf("row %v has %d cells", row, len(row))
		}
		if row[0] == "criterion" {
			statuses[row[5]]++
		}
	}
	if statuses["fail"] != 3 || statuses["missing"] != 1 {
		t.Fatalf("criterion results %v", statuses)
	}
}

func TestReportOfRunWithoutVerdict(t *testing.T) {
	rs, ts := fixtures()
	h := newHarness(t, rs, ts)
	rep := verdictOfReport(t, h.get(t, "run-live", "").Body.Bytes())
	if rep.Run.Verdict != nil || rep.Run.State != runs.Running || rep.Run.Summary == nil || *rep.Run.Summary.SessionsAttempted != 50 {
		t.Fatalf("report %+v", rep.Run)
	}
	body := h.get(t, "run-live", "format=csv").Body.String()
	if !strings.Contains(body, "run,verdict,,,not evaluated,,\n") {
		t.Fatalf("csv lacks the not evaluated verdict:\n%s", body)
	}
}

func TestReportErrors(t *testing.T) {
	rs, ts := fixtures()
	h := newHarness(t, rs, ts)
	cases := []struct {
		id, query string
		status    int
	}{
		{"run-missing", "", http.StatusNotFound},
		{"run-pass", "format=html", http.StatusNotImplemented},
		{"run-broken", "", http.StatusInternalServerError},
	}
	for _, c := range cases {
		rec := h.get(t, c.id, c.query)
		if rec.Code != c.status || rec.Header().Get("Content-Type") != api.ProblemContentType {
			t.Errorf("%s?%s: status %d, type %q, want %d", c.id, c.query, rec.Code, rec.Header().Get("Content-Type"), c.status)
		}
	}
	// An unknown format never reaches the contract validator, which rejects
	// the request itself, so the handler is called directly.
	bad := api.GetRunReportParamsFormat("pdf")
	rec := httptest.NewRecorder()
	NewAPI(newEvaluator(t), rs, ts).GetRunReport(rec, httptest.NewRequest(http.MethodGet, "/v1/runs/run-pass/report?format=pdf", nil), "run-pass", api.GetRunReportParams{Format: &bad})
	if rec.Code != http.StatusBadRequest {
		t.Errorf("format pdf: status %d", rec.Code)
	}
}

func TestReportWithoutAggregator(t *testing.T) {
	rs, _ := fixtures()
	h := newHarness(t, rs, nil)
	rep := verdictOfReport(t, h.get(t, "run-pass", "").Body.Bytes())
	cs := *rep.Run.Verdict.Criteria
	// Without the aggregator the histogram and throughput criteria are
	// missing, and the ratio still passes.
	if !*rep.Run.Verdict.Passed || cs[0].Passed == nil || cs[1].Passed != nil || cs[2].Passed != nil {
		t.Fatalf("verdict %+v", rep.Run.Verdict)
	}
}

// TestReportFromRealAggregator feeds counter snapshots of two agents through
// the metrics aggregator of CTL-08 and checks that the report derives the
// failure ratio from the merged counters.
func TestReportFromRealAggregator(t *testing.T) {
	agg, err := metrics.New(metrics.Config{})
	if err != nil {
		t.Fatal(err)
	}
	labels := map[string]string{"application": "http1", "scenario": "web_get"}
	count := func(metric string, n uint64) *agentpb.CounterValue {
		return &agentpb.CounterValue{Metric: metric, Labels: labels, Value: &agentpb.CounterValue_Count{Count: n}}
	}
	for _, a := range []struct {
		id                string
		attempted, failed uint64
	}{{"agent-a", 1000, 0}, {"agent-b", 1000, 30}} {
		snap := &agentpb.CounterSnapshot{PlanId: "run-agg/" + a.id, Sequence: 1, Counters: []*agentpb.CounterValue{
			count("sessions.attempted", a.attempted), count("sessions.failed", a.failed),
		}}
		if !agg.Add(a.id, t0, snap) {
			t.Fatalf("aggregator rejected the snapshot of %s", a.id)
		}
	}
	run := baseRun("run-agg", runs.Completed, "agent-a", "agent-b")
	run.Verdict = verdictOf(true, crit("sessions.failed_ratio", "<", 0.01), crit("sessions.failed_ratio", "==", 0.015))
	h := newHarness(t, fakeRuns{"run-agg": run}, agg)
	rep := verdictOfReport(t, h.get(t, "run-agg", "").Body.Bytes())
	cs := *rep.Run.Verdict.Criteria
	if *rep.Run.Verdict.Passed || *cs[0].Passed || !*cs[1].Passed || *cs[1].Observed != float32(0.015) {
		t.Fatalf("verdict %+v %+v %+v", rep.Run.Verdict.Passed, cs[0], cs[1])
	}
}

func TestCellTextGuardsFormulas(t *testing.T) {
	for in, want := range map[string]string{"=1+1": "'=1+1", "-x": "'-x", "@a": "'@a", "plain": "plain", "": ""} {
		if got := cellText(in); got != want {
			t.Errorf("cellText(%q) = %q, want %q", in, got, want)
		}
	}
}
