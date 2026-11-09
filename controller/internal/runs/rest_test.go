// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"

	"github.com/getkin/kin-openapi/openapi3"
	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

const contractPath = "../../../contracts/openapi/fps-api-v1.yaml"

// restHarness serves the run operations behind the real router and token
// check and validates every exchange against the contract.
type restHarness struct {
	doc     *openapi3.T
	handler http.Handler
}

func newRESTHarness(t *testing.T, o *runs.Orchestrator) *restHarness {
	t.Helper()
	doc, err := contract.Load(contractPath)
	if err != nil {
		t.Fatal(err)
	}
	auth, err := api.NewStaticTokens(map[string]api.Scope{"token-run": api.ScopeRun})
	if err != nil {
		t.Fatal(err)
	}
	return &restHarness{doc: doc, handler: api.New(runs.NewAPI(o), auth, slog.New(slog.DiscardHandler)).Handler()}
}

// call sends one request to the operation at path and method. It validates
// the response against the contract, and the request too when validReq holds.
func (h *restHarness) call(t *testing.T, method, path string, params map[string]string, body string, validReq bool) *httptest.ResponseRecorder {
	t.Helper()
	return h.callQuery(t, method, path, params, "", body, validReq)
}

// callQuery is call with the raw query string of the request.
func (h *restHarness) callQuery(t *testing.T, method, path string, params map[string]string, query, body string, validReq bool) *httptest.ResponseRecorder {
	t.Helper()
	item := h.doc.Paths.Value(path)
	if item == nil || item.GetOperation(method) == nil {
		t.Fatalf("the contract has no %s %s", method, path)
	}
	url := path
	for k, v := range params {
		url = strings.ReplaceAll(url, "{"+k+"}", v)
	}
	newReq := func() *http.Request {
		target := api.BasePath + url
		if query != "" {
			target += "?" + query
		}
		r := httptest.NewRequest(method, target, strings.NewReader(body))
		if body != "" {
			r.Header.Set("Content-Type", "application/json")
		}
		r.Header.Set("Authorization", "Bearer token-run")
		return r
	}
	rec := httptest.NewRecorder()
	h.handler.ServeHTTP(rec, newReq())
	in := &openapi3filter.RequestValidationInput{
		Request:    newReq(),
		PathParams: params,
		Route:      &routers.Route{Spec: h.doc, Path: path, PathItem: item, Method: method, Operation: item.GetOperation(method)},
		Options:    &openapi3filter.Options{AuthenticationFunc: openapi3filter.NoopAuthenticationFunc},
	}
	if validReq {
		if err := openapi3filter.ValidateRequest(context.Background(), in); err != nil {
			t.Fatalf("%s %s: request breaks the contract: %v", method, url, err)
		}
	}
	out := &openapi3filter.ResponseValidationInput{
		RequestValidationInput: in,
		Status:                 rec.Code,
		Header:                 rec.Header(),
		Body:                   io.NopCloser(bytes.NewReader(rec.Body.Bytes())),
		Options:                &openapi3filter.Options{IncludeResponseStatus: true},
	}
	if err := openapi3filter.ValidateResponse(context.Background(), out); err != nil {
		t.Fatalf("%s %s: response breaks the contract: %v\nbody: %s", method, url, err, rec.Body)
	}
	return rec
}

func decodeRun(t *testing.T, rec *httptest.ResponseRecorder) api.Run {
	t.Helper()
	var v api.Run
	if err := json.Unmarshal(rec.Body.Bytes(), &v); err != nil {
		t.Fatalf("decode %s: %v", rec.Body, err)
	}
	return v
}

func TestRESTRunLifecycle(t *testing.T) {
	h := newHarness(t, nil)
	a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
	rh := newRESTHarness(t, h.orch)

	rec := rh.call(t, http.MethodPost, "/runs", nil, `{"config_id":"cfg-1","agents":["agent-a","agent-b"],"labels":{"suite":"smoke"}}`, true)
	if rec.Code != http.StatusAccepted {
		t.Fatalf("start: %d %s", rec.Code, rec.Body)
	}
	run := decodeRun(t, rec)
	if run.State != api.RunStatePending || run.ConfigId != "cfg-1" || rec.Header().Get("Location") != "/v1/runs/"+run.Id {
		t.Fatalf("start answered %+v with Location %q", run, rec.Header().Get("Location"))
	}
	h.waitRunning(t, run.Id, a, b)

	id := map[string]string{"id": run.Id}
	rec = rh.call(t, http.MethodGet, "/runs/{id}", id, "", true)
	if got := decodeRun(t, rec); rec.Code != http.StatusOK || got.State != api.RunStateRunning || got.StartedAt == nil {
		t.Fatalf("get: %d %s", rec.Code, rec.Body)
	}

	sessions(t, a, 100, 100, 0)
	sessions(t, b, 100, 40, 10)
	if err := b.Fail("worker panic"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "a degraded run", func() bool {
		r := decodeRun(t, rh.call(t, http.MethodGet, "/runs/{id}", id, "", true))
		return r.Degraded != nil && *r.Degraded
	})

	rec = rh.call(t, http.MethodPost, "/runs/{id}:stop", id, `{"force":false}`, true)
	if rec.Code != http.StatusAccepted {
		t.Fatalf("stop: %d %s", rec.Code, rec.Body)
	}
	h.wait(t, run.Id)
	end := decodeRun(t, rh.call(t, http.MethodGet, "/runs/{id}", id, "", true))
	if end.State != api.RunStateFailed || end.Verdict == nil || *end.Verdict.Passed || end.Summary == nil ||
		*end.Summary.SessionsAttempted != 200 || *end.Summary.SessionsFailed != 60 || end.EndedAt == nil {
		t.Fatalf("ended run %s", mustJSON(t, end))
	}
	crit := (*end.Verdict.Criteria)[0]
	if *crit.Metric != "sessions.failed_ratio" || *crit.Passed || *crit.Observed != 0.3 {
		t.Fatalf("criterion %s", mustJSON(t, crit))
	}
	if skipped := (*end.Verdict.Criteria)[1]; skipped.Observed != nil || skipped.Passed != nil {
		t.Fatalf("unevaluated criterion %s", mustJSON(t, skipped))
	}
	// The run and its criteria say why they failed.
	if want := "observed 0.3 < 0.1 does not hold"; crit.Reason == nil || *crit.Reason != want {
		t.Fatalf("criterion reason %s, want %q", mustJSON(t, crit), want)
	}
	if skipped := (*end.Verdict.Criteria)[1]; skipped.Reason == nil || *skipped.Reason != "the run reported no value" {
		t.Fatalf("unevaluated criterion reason %s", mustJSON(t, skipped))
	}
	if want := "criteria failed: sessions.failed_ratio: observed 0.3 < 0.1 does not hold"; end.Reason == nil || *end.Reason != want {
		t.Fatalf("run reason %v, want %q", end.Reason, want)
	}

	if rec = rh.call(t, http.MethodPost, "/runs/{id}:stop", id, "", true); rec.Code != http.StatusConflict {
		t.Fatalf("stop of an ended run: %d %s", rec.Code, rec.Body)
	}
}

func TestRESTForceStopWithoutBody(t *testing.T) {
	h := newHarness(t, nil)
	a := h.agent(t, "agent-a", nil)
	rh := newRESTHarness(t, h.orch)
	run := h.start(t, a)
	id := map[string]string{"id": run.ID}
	if rec := rh.call(t, http.MethodPost, "/runs/{id}:stop", id, `{"force":true}`, true); rec.Code != http.StatusAccepted {
		t.Fatalf("force stop: %d %s", rec.Code, rec.Body)
	}
	if end := h.wait(t, run.ID); end.State != runs.Aborted {
		t.Fatalf("run ended %s, want aborted", end.State)
	}
}

func TestRESTErrors(t *testing.T) {
	h := newHarness(t, nil)
	h.agent(t, "agent-a", nil)
	rh := newRESTHarness(t, h.orch)
	tests := []struct {
		name, method, path string
		params             map[string]string
		body               string
		validReq           bool
		want               int
	}{
		{"unknown run", http.MethodGet, "/runs/{id}", map[string]string{"id": "nope"}, "", true, http.StatusNotFound},
		{"stop unknown run", http.MethodPost, "/runs/{id}:stop", map[string]string{"id": "nope"}, "", true, http.StatusNotFound},
		{"unknown config", http.MethodPost, "/runs", nil, `{"config_id":"nope","agents":["agent-a"]}`, true, http.StatusUnprocessableEntity},
		{"invalid config", http.MethodPost, "/runs", nil, `{"config_id":"broken","agents":["agent-a"]}`, true, http.StatusUnprocessableEntity},
		{"unknown agent", http.MethodPost, "/runs", nil, `{"config_id":"cfg-1","agents":["agent-x"]}`, true, http.StatusUnprocessableEntity},
		{"no reserved agents", http.MethodPost, "/runs", nil, `{"config_id":"cfg-1"}`, true, http.StatusConflict},
		{"unknown field", http.MethodPost, "/runs", nil, `{"config_id":"cfg-1","extra":1}`, false, http.StatusBadRequest},
		{"empty body", http.MethodPost, "/runs", nil, ``, false, http.StatusBadRequest},
		{"bad stop body", http.MethodPost, "/runs/{id}:stop", map[string]string{"id": "nope"}, `{"force":"yes"}`, false, http.StatusBadRequest},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			rec := rh.call(t, tc.method, tc.path, tc.params, tc.body, tc.validReq)
			if rec.Code != tc.want {
				t.Fatalf("%d %s, want %d", rec.Code, rec.Body, tc.want)
			}
			if ct := rec.Header().Get("Content-Type"); ct != "application/problem+json" {
				t.Fatalf("content type %q, want a problem", ct)
			}
		})
	}
}

func TestRESTOtherOperationsStayUnimplemented(t *testing.T) {
	h := newHarness(t, nil)
	rh := newRESTHarness(t, h.orch)
	if rec := rh.call(t, http.MethodGet, "/runs/{id}/report", map[string]string{"id": "nope"}, "", true); rec.Code != http.StatusNotImplemented {
		t.Fatalf("report: %d %s", rec.Code, rec.Body)
	}
}

// listPage is the body of listRuns.
type listPage struct {
	Items []api.Run `json:"items"`
	Next  *string   `json:"next_page_token"`
}

func (h *restHarness) list(t *testing.T, query string) listPage {
	t.Helper()
	rec := h.callQuery(t, http.MethodGet, "/runs", nil, query, "", true)
	if rec.Code != http.StatusOK {
		t.Fatalf("list %q: %d %s", query, rec.Code, rec.Body)
	}
	var page listPage
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatalf("decode %s: %v", rec.Body, err)
	}
	if page.Items == nil {
		t.Fatalf("list %q: items is absent in %s", query, rec.Body)
	}
	return page
}

func ids(items []api.Run) []string {
	out := make([]string, len(items))
	for i, r := range items {
		out[i] = r.Id
	}
	return out
}

func TestRESTListRunsEmpty(t *testing.T) {
	h := newHarness(t, nil)
	rh := newRESTHarness(t, h.orch)
	if page := rh.list(t, ""); len(page.Items) != 0 || page.Next != nil {
		t.Fatalf("empty list = %+v", page)
	}
}

func TestRESTListRunsOrderPagingAndFilter(t *testing.T) {
	h := newHarness(t, nil)
	rh := newRESTHarness(t, h.orch)
	var started []string // oldest first
	for _, id := range []string{"agent-a", "agent-b", "agent-c", "agent-d", "agent-e"} {
		started = append(started, h.start(t, h.agent(t, id, nil)).ID)
	}
	newest := func(n int) []string {
		var out []string
		for i := len(started) - 1; i >= len(started)-n; i-- {
			out = append(out, started[i])
		}
		return out
	}

	// Without parameters: every run, newest first, no further page.
	page := rh.list(t, "")
	if got := ids(page.Items); !slices.Equal(got, newest(5)) || page.Next != nil {
		t.Fatalf("list = %v next %v, want %v and no next", got, page.Next, newest(5))
	}

	// Paging by two walks all runs once, newest first, and ends without a token.
	var walked []string
	query := "page_size=2"
	for pages := 0; ; pages++ {
		if pages > 5 {
			t.Fatalf("paging does not end after %v", walked)
		}
		p := rh.list(t, query)
		if len(p.Items) > 2 {
			t.Fatalf("page of %d items, want at most 2", len(p.Items))
		}
		walked = append(walked, ids(p.Items)...)
		if p.Next == nil {
			break
		}
		if len(p.Items) != 2 {
			t.Fatalf("a short page of %d carries a token", len(p.Items))
		}
		query = "page_size=2&page_token=" + *p.Next
	}
	if !slices.Equal(walked, newest(5)) {
		t.Fatalf("paged walk = %v, want %v", walked, newest(5))
	}

	// A page that ends exactly at the last run has no token.
	if p := rh.list(t, "page_size=5"); p.Next != nil || len(p.Items) != 5 {
		t.Fatalf("page_size=5 = %v next %v", ids(p.Items), p.Next)
	}

	// A run that starts between two pages neither shifts nor repeats an item.
	first := rh.list(t, "page_size=2")
	extra := h.start(t, h.agent(t, "agent-f", nil)).ID
	second := rh.list(t, "page_size=2&page_token="+*first.Next)
	if want := []string{started[2], started[1]}; !slices.Equal(ids(second.Items), want) {
		t.Fatalf("second page after a new run = %v, want %v", ids(second.Items), want)
	}
	if fresh := rh.list(t, "page_size=1"); len(fresh.Items) != 1 || fresh.Items[0].Id != extra {
		t.Fatalf("newest run = %v, want %s", ids(fresh.Items), extra)
	}

	// The state filter keeps the matching runs, and pages them the same way.
	if _, err := h.orch.Stop(started[0], true); err != nil {
		t.Fatal(err)
	}
	h.wait(t, started[0])
	if got := ids(rh.list(t, "state=aborted").Items); !slices.Equal(got, []string{started[0]}) {
		t.Fatalf("state=aborted = %v, want %v", got, []string{started[0]})
	}
	running := rh.list(t, "state=running&page_size=2")
	if len(running.Items) != 2 || running.Next == nil {
		t.Fatalf("state=running page = %v next %v", ids(running.Items), running.Next)
	}
	for _, r := range running.Items {
		if r.State != api.RunStateRunning {
			t.Fatalf("state=running returned %s in state %s", r.Id, r.State)
		}
	}
	total := len(running.Items)
	for next := running.Next; next != nil; {
		p := rh.list(t, "state=running&page_size=2&page_token="+*next)
		for _, r := range p.Items {
			if r.State != api.RunStateRunning {
				t.Fatalf("state=running returned %s in state %s", r.Id, r.State)
			}
		}
		total += len(p.Items)
		next = p.Next
	}
	if total != 5 {
		t.Fatalf("running runs = %d, want 5 (six started, one aborted)", total)
	}
	if got := rh.list(t, "state=completed"); len(got.Items) != 0 || got.Next != nil {
		t.Fatalf("state=completed = %v", ids(got.Items))
	}
}

func TestRESTListRunsRejectsBadParameters(t *testing.T) {
	h := newHarness(t, nil)
	rh := newRESTHarness(t, h.orch)
	for _, query := range []string{
		"page_size=0", "page_size=501", "page_size=-1", "page_size=many",
		"state=sleeping", "page_token=%21%21", "page_token=cnVuXzE", "page_token=cjA",
	} {
		t.Run(query, func(t *testing.T) {
			rec := rh.callQuery(t, http.MethodGet, "/runs", nil, query, "", false)
			if rec.Code != http.StatusBadRequest {
				t.Fatalf("%d %s, want 400", rec.Code, rec.Body)
			}
			if ct := rec.Header().Get("Content-Type"); ct != "application/problem+json" {
				t.Fatalf("content type %q, want a problem", ct)
			}
		})
	}
}

func mustJSON(t *testing.T, v any) string {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}
