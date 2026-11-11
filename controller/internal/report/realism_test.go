// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

const (
	realismSchemaPath = "../../../realism/schema/realism-report.schema.json"
	realismPath       = "/runs/{id}/realism"
)

func TestEmbeddedRealismSchemaMatchesSource(t *testing.T) {
	want, err := os.ReadFile(realismSchemaPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(realismSchema, want) {
		t.Fatal("schema/realism-report.schema.json differs from realism/schema/realism-report.schema.json, run go generate ./internal/report/")
	}
}

// realismFixture returns the realism report of testdata, which holds a
// conformance check and a temporal check, each with a passing and a failing
// entry. It validates against the report schema.
func realismFixture(t *testing.T) []byte {
	t.Helper()
	data, err := os.ReadFile("testdata/realism-report.json")
	if err != nil {
		t.Fatal(err)
	}
	return data
}

// swap returns doc with the one occurrence of old replaced by repl.
func swap(t *testing.T, doc []byte, old, repl string) []byte {
	t.Helper()
	if n := bytes.Count(doc, []byte(old)); n != 1 {
		t.Fatalf("the fixture holds %d occurrences of %q, want 1", n, old)
	}
	return bytes.Replace(doc, []byte(old), []byte(repl), 1)
}

func result(metric string, labels map[string]string, observed float32, property, target string, passed bool) api.RealismResult {
	r := api.RealismResult{Metric: &metric, Observed: observed, Passed: passed, Property: property, Target: target}
	if labels != nil {
		r.Labels = &labels
	}
	return r
}

// fixtureResults are the entries that CON-22 maps the fixture to: the two
// conformance ratios against the threshold 0.99, and the KS statistic and p
// value of the three comparisons that the metric catalog labels, against the
// alpha 0.05. The comparison over both directions has no metric. Each check
// ends with its realism.check_failures series (CON-23), which takes the
// generic rule until the controller implements the CON-23 table.
func fixtureResults() []api.RealismResult {
	app := func(a, c string) map[string]string { return map[string]string{"application": a, "classifier": c} }
	ks := func(p string) map[string]string { return map[string]string{"application": "http1", "property": p} }
	const alpha = "p value >= 0.05"
	failures := func(check string) api.RealismResult {
		return result("realism.check_failures", map[string]string{"check": check}, 1,
			"realism.check_failures of check "+check, "check passed", false)
	}
	return []api.RealismResult{
		result("realism.conformance_ratio", app("http1", "ndpi"), 1, "conformance of http1 by ndpi", ">= 0.99", true),
		result("realism.conformance_ratio", app("dns", "ndpi"), 0.9, "conformance of dns by ndpi", ">= 0.99", false),
		failures("conformance"),
		result("realism.ks_statistic", ks("inter_arrival"), 0.08, "inter arrival of http1, KS statistic", alpha, true),
		result("realism.ks_p_value", ks("inter_arrival"), 0.42, "inter arrival of http1, KS p value", alpha, true),
		result("realism.ks_statistic", ks("request_bytes"), 0.31, "request bytes of http1, KS statistic", alpha, false),
		result("realism.ks_p_value", ks("request_bytes"), 0.01, "request bytes of http1, KS p value", alpha, false),
		result("realism.ks_statistic", ks("response_bytes"), 0.05, "response bytes of http1, KS statistic", alpha, true),
		result("realism.ks_p_value", ks("response_bytes"), 0.5, "response bytes of http1, KS p value", alpha, true),
		failures("temporal"),
	}
}

func showResults(rs []api.RealismResult) string {
	b, _ := json.MarshalIndent(rs, "", "  ")
	return string(b)
}

func TestMapRealismFixture(t *testing.T) {
	got, findings, err := MapRealism(realismFixture(t))
	if err != nil || len(findings) != 0 {
		t.Fatalf("MapRealism = %v, %+v", err, findings)
	}
	if want := fixtureResults(); !reflect.DeepEqual(got, want) {
		t.Fatalf("results:\n%s\nwant:\n%s", showResults(got), showResults(want))
	}
}

// TestMapRealismFallback feeds documents that the schema of today refuses,
// because only conformance and temporal checks carry metrics, to check that
// every other metric gets the target `check passed` and the outcome of its
// check, as CON-22 says, and that a conformance or KS metric without an entry
// to take its outcome from gets the same.
func TestMapRealismFallback(t *testing.T) {
	doc := `{"checks": [
		{"check": "mix_accuracy", "passed": false, "metrics": [
			{"name": "mix.max_abs_error_pp", "labels": {"application": "dns"}, "value": 3.5}]},
		{"check": "session_structure", "passed": true, "metrics": [
			{"name": "realism.structure_ok", "labels": {}, "value": 1}]},
		{"check": "conformance", "passed": true, "threshold": 0.9,
			"applications": [{"application": "http1", "passed": true}], "metrics": [
			{"name": "realism.conformance_ratio", "labels": {"application": "other", "classifier": "stub"}, "value": 0.5},
			{"name": "realism.conformance_ratio", "labels": {"application": "http1", "classifier": "stub"}, "value": 0.95},
			{"name": "realism.other", "labels": {"application": "http1"}, "value": 2}]},
		{"check": "temporal", "passed": false, "comparisons": [
			{"property": "payload_size", "application": "http1", "direction": "both", "alpha": 0.05, "status": "pass"}], "metrics": [
			{"name": "realism.ks_p_value", "labels": {"application": "http1", "property": "inter_arrival"}, "value": 0.2}]}
	]}`
	got, findings, err := MapRealism([]byte(doc))
	if err != nil || len(findings) != 0 {
		t.Fatalf("MapRealism = %v, %+v", err, findings)
	}
	want := []api.RealismResult{
		result("mix.max_abs_error_pp", map[string]string{"application": "dns"}, 3.5,
			"mix.max_abs_error_pp of application dns", "check passed", false),
		result("realism.structure_ok", nil, 1, "realism.structure_ok", "check passed", true),
		result("realism.conformance_ratio", map[string]string{"application": "other", "classifier": "stub"}, 0.5,
			"realism.conformance_ratio of application other, classifier stub", "check passed", true),
		result("realism.conformance_ratio", map[string]string{"application": "http1", "classifier": "stub"}, 0.95,
			"conformance of http1 by stub", ">= 0.9", true),
		result("realism.other", map[string]string{"application": "http1"}, 2,
			"realism.other of application http1", "check passed", true),
		result("realism.ks_p_value", map[string]string{"application": "http1", "property": "inter_arrival"}, 0.2,
			"realism.ks_p_value of application http1, property inter_arrival", "check passed", false),
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("results:\n%s\nwant:\n%s", showResults(got), showResults(want))
	}
}

func TestMapRealismFindings(t *testing.T) {
	cases := []struct {
		name, doc, path string
	}{
		{"a metric and labels that repeat",
			`{"checks": [{"check": "mix_accuracy", "passed": true, "metrics": [
				{"name": "a.b", "labels": {"x": "1", "y": "2"}, "value": 1},
				{"name": "a.b", "labels": {"y": "2", "x": "1"}, "value": 2}]}]}`,
			"/checks/0/metrics/1"},
		{"a repeat in a later check",
			`{"checks": [
				{"check": "mix_accuracy", "passed": true, "metrics": [{"name": "a.b", "labels": {}, "value": 1}]},
				{"check": "content", "passed": true, "metrics": [{"name": "a.b", "labels": {}, "value": 1}]}]}`,
			"/checks/1/metrics/0"},
		{"a value beyond a 32 bit float",
			`{"checks": [{"check": "mix_accuracy", "passed": true, "metrics": [{"name": "a.b", "labels": {}, "value": 1e300}]}]}`,
			"/checks/0/metrics/0/value"},
	}
	for _, c := range cases {
		got, findings, err := MapRealism([]byte(c.doc))
		if err != nil || got != nil || len(findings) != 1 || findings[0].Path != c.path || findings[0].Severity != api.FindingSeverityError {
			t.Errorf("%s: MapRealism = %+v, %+v, %v, want one error at %s", c.name, got, findings, err, c.path)
		}
	}
	if _, _, err := MapRealism([]byte(`{"checks": [{"check": 5}]}`)); err == nil {
		t.Error("MapRealism of a check without a name succeeded")
	}
}

// putRealism calls PUT /v1/runs/{id}/realism with the token and validates the
// answer against the contract.
func (h *harness) putRealism(t *testing.T, token, id, contentType string, body []byte) *httptest.ResponseRecorder {
	t.Helper()
	item := h.doc.Paths.Value(realismPath)
	if item == nil || item.Put == nil {
		t.Fatalf("the contract has no PUT %s", realismPath)
	}
	url := api.BasePath + strings.ReplaceAll(realismPath, "{id}", id)
	newReq := func() *http.Request {
		r := httptest.NewRequest(http.MethodPut, url, bytes.NewReader(body))
		r.Header.Set("Authorization", "Bearer "+token)
		if contentType != "" {
			r.Header.Set("Content-Type", contentType)
		}
		return r
	}
	rec := httptest.NewRecorder()
	h.handler.ServeHTTP(rec, newReq())
	in := &openapi3filter.RequestValidationInput{
		Request:    newReq(),
		PathParams: map[string]string{"id": id},
		Route:      &routers.Route{Spec: h.doc, Path: realismPath, PathItem: item, Method: http.MethodPut, Operation: item.Put},
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
		t.Fatalf("PUT %s: response breaks the contract: %v\nbody: %s", url, err, rec.Body)
	}
	return rec
}

// attach calls putRealism as a client with the scope run and the content type
// application/json.
func (h *harness) attach(t *testing.T, id string, body []byte) *httptest.ResponseRecorder {
	t.Helper()
	return h.putRealism(t, "token-run", id, JSONContentType, body)
}

func attachment(t *testing.T, rec *httptest.ResponseRecorder) api.RealismAttachment {
	t.Helper()
	if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != JSONContentType {
		t.Fatalf("status %d, type %q, body %s", rec.Code, rec.Header().Get("Content-Type"), rec.Body)
	}
	var got api.RealismAttachment
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	return got
}

func realismOfReport(t *testing.T, h *harness, id string) []api.RealismResult {
	t.Helper()
	rec := h.get(t, id, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("GET report of %s: status %d, body %s", id, rec.Code, rec.Body)
	}
	rep := verdictOfReport(t, rec.Body.Bytes())
	if rep.Realism == nil {
		t.Fatalf("report of %s has no realism array", id)
	}
	return rep.Realism
}

func problemOf(t *testing.T, rec *httptest.ResponseRecorder, status int) api.Problem {
	t.Helper()
	if rec.Code != status || rec.Header().Get("Content-Type") != api.ProblemContentType {
		t.Fatalf("status %d, type %q, want a %d problem; body %s", rec.Code, rec.Header().Get("Content-Type"), status, rec.Body)
	}
	var p api.Problem
	if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil {
		t.Fatal(err)
	}
	return p
}

func findingPaths(p api.Problem) []string {
	var out []string
	if p.Errors != nil {
		for _, f := range *p.Errors {
			out = append(out, f.Path)
		}
	}
	return out
}

func newRealismHarness(t *testing.T) *harness {
	t.Helper()
	rs, ts := fixtures()
	for _, st := range []runs.State{runs.Pending, runs.Preparing, runs.Stopping, runs.Aborted} {
		id := "run-" + string(st)
		rs[id] = baseRun(id, st, "agent-a")
	}
	return newHarness(t, rs, ts)
}

func TestAttachRealismReportOfEndedRuns(t *testing.T) {
	h := newRealismHarness(t)
	for _, id := range []string{"run-pass", "run-fail", "run-aborted"} {
		if got := realismOfReport(t, h, id); len(got) != 0 {
			t.Fatalf("%s holds %d results before any report", id, len(got))
		}
		got := attachment(t, h.attach(t, id, realismFixture(t)))
		if got.Replaced || !reflect.DeepEqual(got.Realism, fixtureResults()) {
			t.Fatalf("%s: attachment replaced=%v results:\n%s\nwant:\n%s", id, got.Replaced, showResults(got.Realism), showResults(fixtureResults()))
		}
		if rep := realismOfReport(t, h, id); !reflect.DeepEqual(rep, fixtureResults()) {
			t.Fatalf("%s: report realism:\n%s\nwant:\n%s", id, showResults(rep), showResults(fixtureResults()))
		}
	}
	// The reports of two runs stay apart.
	if got := realismOfReport(t, h, "run-degraded"); len(got) != 0 {
		t.Errorf("run-degraded holds %d results, want none", len(got))
	}
}

func TestAttachRealismReportAcceptsContentTypeParameters(t *testing.T) {
	h := newRealismHarness(t)
	rec := h.putRealism(t, "token-run", "run-pass", "application/json; charset=utf-8", realismFixture(t))
	if got := attachment(t, rec); len(got.Realism) != 8 {
		t.Fatalf("results %d, want 8", len(got.Realism))
	}
}

func TestAttachRealismReportReplaces(t *testing.T) {
	h := newRealismHarness(t)
	first := realismFixture(t)
	if got := attachment(t, h.attach(t, "run-pass", first)); got.Replaced {
		t.Fatal("the first report replaced another")
	}
	// The same document again gives the same state, and says that it replaced.
	if got := attachment(t, h.attach(t, "run-pass", first)); !got.Replaced || !reflect.DeepEqual(got.Realism, fixtureResults()) {
		t.Fatalf("second upload: replaced=%v results:\n%s", got.Replaced, showResults(got.Realism))
	}
	// A report scored again replaces the first as a whole: dns now conforms,
	// and the temporal check is gone.
	second := swap(t, first, `"labels": {"application": "dns", "classifier": "ndpi"}, "value": 0.9}`,
		`"labels": {"application": "dns", "classifier": "ndpi"}, "value": 0.995}`)
	second = swap(t, second, `"ratio": 0.9, "passed": false`, `"ratio": 0.995, "passed": true`)
	var top map[string]json.RawMessage
	if err := json.Unmarshal(second, &top); err != nil {
		t.Fatal(err)
	}
	var checks []json.RawMessage
	if err := json.Unmarshal(top["checks"], &checks); err != nil {
		t.Fatal(err)
	}
	top["checks"], _ = json.Marshal(checks[:1])
	second, _ = json.Marshal(top)
	want := fixtureResults()[:3]
	want[1] = result("realism.conformance_ratio", map[string]string{"application": "dns", "classifier": "ndpi"}, 0.995,
		"conformance of dns by ndpi", ">= 0.99", true)
	if got := attachment(t, h.attach(t, "run-pass", second)); !got.Replaced || !reflect.DeepEqual(got.Realism, want) {
		t.Fatalf("third upload: replaced=%v results:\n%s\nwant:\n%s", got.Replaced, showResults(got.Realism), showResults(want))
	}
	if rep := realismOfReport(t, h, "run-pass"); !reflect.DeepEqual(rep, want) {
		t.Fatalf("report realism:\n%s\nwant:\n%s", showResults(rep), showResults(want))
	}
}

func TestAttachRealismReportRefusalKeepsTheEarlierReport(t *testing.T) {
	h := newRealismHarness(t)
	attachment(t, h.attach(t, "run-pass", realismFixture(t)))
	refused := []struct {
		name        string
		contentType string
		body        []byte
	}{
		{"not json", JSONContentType, []byte(`{`)},
		{"wrong type", "text/plain", realismFixture(t)},
		{"breaks the schema", JSONContentType, []byte(`{"report": "fps-realism-report"}`)},
	}
	for _, c := range refused {
		if rec := h.putRealism(t, "token-run", "run-pass", c.contentType, c.body); rec.Code < 400 || rec.Code >= 500 {
			t.Fatalf("%s: status %d, want a 4xx problem", c.name, rec.Code)
		}
		if rep := realismOfReport(t, h, "run-pass"); !reflect.DeepEqual(rep, fixtureResults()) {
			t.Fatalf("%s changed the report:\n%s", c.name, showResults(rep))
		}
	}
	if got := attachment(t, h.attach(t, "run-pass", realismFixture(t))); !got.Replaced {
		t.Error("the earlier report was lost, so the next upload did not replace")
	}
}

func TestAttachRealismReportRefusesARunThatHasNotEnded(t *testing.T) {
	h := newRealismHarness(t)
	for _, id := range []string{"run-live", "run-pending", "run-preparing", "run-stopping"} {
		p := problemOf(t, h.attach(t, id, realismFixture(t)), http.StatusConflict)
		state := strings.TrimPrefix(id, "run-")
		if id == "run-live" {
			state = "running"
		}
		if p.Detail == nil || !strings.Contains(*p.Detail, state) {
			t.Errorf("%s: detail %v does not name the state %s", id, p.Detail, state)
		}
		if got := realismOfReport(t, h, id); len(got) != 0 {
			t.Errorf("%s holds %d results after the refusal", id, len(got))
		}
	}
}

func TestAttachRealismReportUnknownRun(t *testing.T) {
	h := newRealismHarness(t)
	problemOf(t, h.attach(t, "run-missing", realismFixture(t)), http.StatusNotFound)
}

func TestAttachRealismReportContentType(t *testing.T) {
	h := newRealismHarness(t)
	for _, ct := range []string{"", "text/plain", "application/yaml", "application/json+x", ";;"} {
		problemOf(t, h.putRealism(t, "token-run", "run-pass", ct, realismFixture(t)), http.StatusUnsupportedMediaType)
	}
}

func TestAttachRealismReportMalformedBody(t *testing.T) {
	h := newRealismHarness(t)
	for _, body := range []string{"", "{", "not json", `{"checks": []} {"checks": []}`, `{"checks": []}x`} {
		problemOf(t, h.attach(t, "run-pass", []byte(body)), http.StatusBadRequest)
	}
}

func TestAttachRealismReportSizeLimit(t *testing.T) {
	h := newRealismHarness(t)
	doc := realismFixture(t)
	pad := func(n int) []byte {
		return append(bytes.TrimRight(doc, "\n"), bytes.Repeat([]byte(" "), n-len(bytes.TrimRight(doc, "\n")))...)
	}
	// A report of exactly 4 MiB is served, and one byte more is not.
	if got := attachment(t, h.attach(t, "run-pass", pad(MaxRealismBytes))); len(got.Realism) != 8 {
		t.Fatalf("a report of %d bytes holds %d results", MaxRealismBytes, len(got.Realism))
	}
	p := problemOf(t, h.attach(t, "run-pass", pad(MaxRealismBytes+1)), http.StatusRequestEntityTooLarge)
	if p.Detail == nil || !strings.Contains(*p.Detail, "4194304") {
		t.Errorf("detail %v does not name the limit", p.Detail)
	}
	if rep := realismOfReport(t, h, "run-pass"); len(rep) != 8 {
		t.Errorf("the refused report changed the stored one: %d results", len(rep))
	}
}

func TestAttachRealismReportFindings(t *testing.T) {
	h := newRealismHarness(t)
	doc := realismFixture(t)
	cases := []struct {
		name string
		body []byte
		want []string
	}{
		{"missing properties", []byte(`{"report": "fps-realism-report"}`),
			[]string{"/capture", "/checks", "/generator", "/labels", "/passed", "/version"}},
		{"a wrong constant", swap(t, doc, `"report": "fps-realism-report"`, `"report": "other"`), []string{"/report"}},
		{"a value out of range", swap(t, doc, `"threshold": 0.99`, `"threshold": 1.5`), []string{"/checks/0/threshold"}},
		{"an unknown property", swap(t, doc, `"classifier_version": "4.8.0",`, `"classifier_version": "4.8.0", "extra": 1,`),
			[]string{"/checks/0/extra"}},
		{"a metric that repeats", swap(t, doc,
			`{"name": "realism.conformance_ratio", "labels": {"application": "dns", "classifier": "ndpi"}, "value": 0.9}`,
			`{"name": "realism.conformance_ratio", "labels": {"application": "http1", "classifier": "ndpi"}, "value": 1.0}`),
			[]string{"/checks/0/metrics/1"}},
	}
	for _, c := range cases {
		p := problemOf(t, h.attach(t, "run-pass", c.body), http.StatusUnprocessableEntity)
		if got := findingPaths(p); !reflect.DeepEqual(got, c.want) {
			t.Errorf("%s: findings at %v, want %v", c.name, got, c.want)
		}
		for _, f := range *p.Errors {
			if f.Severity != api.FindingSeverityError || f.Message == "" {
				t.Errorf("%s: finding %+v", c.name, f)
			}
		}
	}
	if rep := realismOfReport(t, h, "run-pass"); len(rep) != 0 {
		t.Errorf("a refused report left %d results", len(rep))
	}
}

func TestAttachRealismReportNeedsScopeRun(t *testing.T) {
	h := newRealismHarness(t)
	problemOf(t, h.putRealism(t, "token-read", "run-pass", JSONContentType, realismFixture(t)), http.StatusForbidden)
	if got := realismOfReport(t, h, "run-pass"); len(got) != 0 {
		t.Errorf("a refused token left %d results", len(got))
	}
}

func TestMemoryRealism(t *testing.T) {
	m := NewMemoryRealism()
	if m.Results("r") != nil {
		t.Fatal("an empty store holds results")
	}
	if _, ok := m.Document("r"); ok {
		t.Fatal("an empty store holds a document")
	}
	doc := []byte(`{"a":1}`)
	if m.Replace("r", doc, fixtureResults()[:1]) {
		t.Fatal("the first Replace reported a replacement")
	}
	doc[0] = '['
	got, ok := m.Document("r")
	if !ok || string(got) != `{"a":1}` {
		t.Fatalf("Document = %q, %v: the store shares the caller's slice", got, ok)
	}
	res := m.Results("r")
	res[0].Passed = false
	if !m.Results("r")[0].Passed {
		t.Fatal("Results shares its slice with the store")
	}
	if !m.Replace("r", doc, nil) {
		t.Fatal("the second Replace reported no replacement")
	}
	if got := m.Results("r"); len(got) != 0 {
		t.Fatalf("Results after a report without results = %v", got)
	}
}

func TestRealismSchemaCompiles(t *testing.T) {
	var v realismValidator
	findings, err := v.check([]byte(`[1]`))
	if err != nil || len(findings) == 0 || findings[0].Path != "" {
		t.Fatalf("check of an array = %+v, %v, want an error at the root", findings, err)
	}
}

func TestPointerEscapes(t *testing.T) {
	if got := pointer([]string{"a/b", "c~d", "0"}); got != "/a~1b/c~0d/0" {
		t.Errorf("pointer = %q", got)
	}
}
