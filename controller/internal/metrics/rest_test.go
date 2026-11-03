// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/getkin/kin-openapi/openapi3"
	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

const contractPath = "../../../contracts/openapi/fps-api-v1.yaml"

// fakeRuns is a RunSource with settable states.
type fakeRuns struct {
	mu     sync.Mutex
	states map[string]runs.State
}

func (f *fakeRuns) Get(id string) (runs.Run, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	s, ok := f.states[id]
	if !ok {
		return runs.Run{}, fmt.Errorf("get run %s: %w", id, runs.ErrRunNotFound)
	}
	return runs.Run{ID: id, State: s}, nil
}

func (f *fakeRuns) set(id string, s runs.State) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.states[id] = s
}

type restHarness struct {
	doc     *openapi3.T
	agg     *Aggregator
	runs    *fakeRuns
	handler http.Handler
}

func newREST(t *testing.T) *restHarness {
	t.Helper()
	doc, err := contract.Load(contractPath)
	if err != nil {
		t.Fatal(err)
	}
	auth, err := api.NewStaticTokens(map[string]api.Scope{"token-read": api.ScopeRead})
	if err != nil {
		t.Fatal(err)
	}
	agg := newAgg(t, func(c *Config) { c.StreamInterval = 10 * time.Millisecond })
	fr := &fakeRuns{states: map[string]runs.State{"run-1": runs.Running, "run-empty": runs.Pending}}
	return &restHarness{doc: doc, agg: agg, runs: fr, handler: api.New(NewAPI(agg, fr), auth, slog.New(slog.DiscardHandler)).Handler()}
}

// get calls GET path with query and validates the exchange against the
// contract.
func (h *restHarness) get(t *testing.T, path, id, query string) *httptest.ResponseRecorder {
	t.Helper()
	item := h.doc.Paths.Value(path)
	if item == nil || item.Get == nil {
		t.Fatalf("the contract has no GET %s", path)
	}
	url := api.BasePath + strings.ReplaceAll(path, "{id}", id) + "?" + query
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
		Route:      &routers.Route{Spec: h.doc, Path: path, PathItem: item, Method: http.MethodGet, Operation: item.Get},
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

func (h *restHarness) feed(seconds uint64) {
	at := t0
	for i := uint64(0); i < seconds; i++ {
		h.agg.Add("agent-a", at, snapshot("run-1/agent-a", i+1,
			[]*agentpb.CounterValue{counter("sessions.attempted", 1<<40+i, webGet...), counter("port.packets.tx", 9, "port", "p1")},
			hist("sessions.response_time", record(100, 2000, 1000*(i+1)), webGet...)))
		at = at.Add(time.Second)
	}
}

func TestGetRunMetrics(t *testing.T) {
	h := newREST(t)
	h.feed(5)
	rec := h.get(t, "/runs/{id}/metrics", "run-1", "view=application")
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d: %s", rec.Code, rec.Body)
	}
	var set SeriesSet
	if err := json.Unmarshal(rec.Body.Bytes(), &set); err != nil {
		t.Fatal(err)
	}
	if set.View != "application" || len(set.Samples) != 10 {
		t.Fatalf("view %s with %d samples, want 10", set.View, len(set.Samples))
	}
	last := set.Samples[len(set.Samples)-1]
	// Counters above 2^24 keep every digit.
	if last.Key != "http1" || last.Values["sessions.attempted"] != float64(1<<40+4) {
		t.Fatalf("last sample %+v", last)
	}
	if last.Values["sessions.response_time.count"] != 3 {
		t.Fatalf("histogram count %v", last.Values["sessions.response_time.count"])
	}

	from, to := t0.Add(time.Second).Format(time.RFC3339), t0.Add(3*time.Second).Format(time.RFC3339)
	rec = h.get(t, "/runs/{id}/metrics", "run-1", "view=port&from="+from+"&to="+to+"&step_s=2")
	if err := json.Unmarshal(rec.Body.Bytes(), &set); err != nil {
		t.Fatal(err)
	}
	var times []time.Time
	for _, s := range set.Samples {
		if s.Key == "p1" {
			times = append(times, s.Ts)
		}
	}
	if len(times) != 2 || !times[0].Equal(t0.Add(2*time.Second)) || !times[1].Equal(t0.Add(3*time.Second)) {
		t.Fatalf("stepped times %v", times)
	}

	rec = h.get(t, "/runs/{id}/metrics", "run-empty", "view=tls")
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), `"samples":[]`) {
		t.Fatalf("empty run: %d %s", rec.Code, rec.Body)
	}
	for query, want := range map[string]int{
		"view=host":                           http.StatusBadRequest,
		"":                                    http.StatusBadRequest,
		"view=tcp&step_s=0":                   http.StatusBadRequest,
		"view=tcp&from=" + to + "&to=" + from: http.StatusBadRequest,
	} {
		if rec := h.get(t, "/runs/{id}/metrics", "run-1", query); rec.Code != want {
			t.Errorf("%q: status %d, want %d", query, rec.Code, want)
		}
	}
	if rec := h.get(t, "/runs/{id}/metrics", "run-x", "view=tcp"); rec.Code != http.StatusNotFound {
		t.Errorf("unknown run: status %d", rec.Code)
	}
}

func TestGetRunMetricsWithoutRunSource(t *testing.T) {
	agg := newAgg(t, nil)
	auth, _ := api.NewStaticTokens(map[string]api.Scope{"t": api.ScopeRead})
	srv := api.New(NewAPI(agg, nil), auth, nil).Handler()
	call := func(id string) int {
		r := httptest.NewRequest(http.MethodGet, "/v1/runs/"+id+"/metrics?view=application", nil)
		r.Header.Set("Authorization", "Bearer t")
		rec := httptest.NewRecorder()
		srv.ServeHTTP(rec, r)
		return rec.Code
	}
	if c := call("r"); c != http.StatusNotFound {
		t.Fatalf("status %d before any snapshot", c)
	}
	agg.Add("agent-a", t0, snapshot("r/agent-a", 1, nil))
	if c := call("r"); c != http.StatusOK {
		t.Fatalf("status %d after a snapshot", c)
	}
}

// event is one Server Sent Event.
type event struct {
	name, id, data string
}

func readEvents(t *testing.T, body io.Reader, until func(event) bool) []event {
	t.Helper()
	var out []event
	var cur event
	sc := bufio.NewScanner(body)
	for sc.Scan() {
		line := sc.Text()
		switch {
		case line == "":
			if cur.data != "" || cur.name != "" {
				out = append(out, cur)
				if until(cur) {
					return out
				}
			}
			cur = event{}
		case strings.HasPrefix(line, "event: "):
			cur.name = strings.TrimPrefix(line, "event: ")
		case strings.HasPrefix(line, "id: "):
			cur.id = strings.TrimPrefix(line, "id: ")
		case strings.HasPrefix(line, "data: "):
			cur.data = strings.TrimPrefix(line, "data: ")
		}
	}
	return out
}

func TestStreamRunMetrics(t *testing.T) {
	h := newREST(t)
	h.feed(2)
	srv := httptest.NewServer(h.handler)
	t.Cleanup(srv.Close)
	req, _ := http.NewRequest(http.MethodGet, srv.URL+"/v1/runs/run-1/metrics:stream?view=scenario&view=port", nil)
	req.Header.Set("Authorization", "Bearer token-read")
	resp, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK || resp.Header.Get("Content-Type") != "text/event-stream" {
		t.Fatalf("status %d, type %s", resp.StatusCode, resp.Header.Get("Content-Type"))
	}
	sampleSchema := h.doc.Components.Schemas["MetricSample"].Value
	ticks := map[string]bool{}
	go func() {
		time.Sleep(50 * time.Millisecond)
		h.feed(3)
		h.runs.set("run-1", runs.Completed)
	}()
	events := readEvents(t, resp.Body, func(e event) bool { return e.name == "end" })
	if len(events) == 0 || events[len(events)-1].name != "end" || events[len(events)-1].data != `{"state":"completed"}` {
		t.Fatalf("stream did not end with the run state: %v", events)
	}
	views := map[string]bool{}
	var lastAttempted float64
	for _, e := range events[:len(events)-1] {
		var v any
		if err := json.Unmarshal([]byte(e.data), &v); err != nil {
			t.Fatal(err)
		}
		if err := sampleSchema.VisitJSON(v); err != nil {
			t.Fatalf("event %s breaks the MetricSample schema: %v", e.data, err)
		}
		var s Sample
		_ = json.Unmarshal([]byte(e.data), &s)
		views[string(s.View)] = true
		ticks[e.id] = true
		if s.View == "scenario" && s.Key == RunKey {
			lastAttempted = s.Values["sessions.attempted"]
		}
	}
	if !views["scenario"] || !views["port"] || len(views) != 2 {
		t.Errorf("views %v", views)
	}
	if len(ticks) < 2 {
		t.Errorf("only %d ticks", len(ticks))
	}
	if lastAttempted != float64(1<<40+2) {
		t.Errorf("final attempted %v, want the newest value", lastAttempted)
	}
}

func TestStreamRejects(t *testing.T) {
	h := newREST(t)
	if rec := h.get(t, "/runs/{id}/metrics:stream", "run-1", "view=host"); rec.Code != http.StatusBadRequest {
		t.Errorf("bad view: %d", rec.Code)
	}
	if rec := h.get(t, "/runs/{id}/metrics:stream", "run-x", ""); rec.Code != http.StatusNotFound {
		t.Errorf("unknown run: %d", rec.Code)
	}
}

func TestStreamEndsWithClient(t *testing.T) {
	h := newREST(t)
	ctx, cancel := context.WithCancel(context.Background())
	r := httptest.NewRequestWithContext(ctx, http.MethodGet, "/v1/runs/run-1/metrics:stream", nil)
	r.Header.Set("Authorization", "Bearer token-read")
	done := make(chan struct{})
	go func() {
		h.handler.ServeHTTP(httptest.NewRecorder(), r)
		close(done)
	}()
	time.Sleep(30 * time.Millisecond)
	cancel()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("stream outlived its client")
	}
}
