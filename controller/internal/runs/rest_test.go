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
	item := h.doc.Paths.Value(path)
	if item == nil || item.GetOperation(method) == nil {
		t.Fatalf("the contract has no %s %s", method, path)
	}
	url := path
	for k, v := range params {
		url = strings.ReplaceAll(url, "{"+k+"}", v)
	}
	newReq := func() *http.Request {
		r := httptest.NewRequest(method, api.BasePath+url, strings.NewReader(body))
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
	if rec := rh.call(t, http.MethodGet, "/runs", nil, "", true); rec.Code != http.StatusNotImplemented {
		t.Fatalf("list runs: %d %s", rec.Code, rec.Body)
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
