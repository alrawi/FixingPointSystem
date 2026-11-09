// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// feedSides gives the run different counts on the two sides, so that a reader
// can tell which side an answer counts.
func (h *restHarness) feedSides() {
	web := func(side string) []string { return append(append([]string{}, webGet...), "side", side) }
	h.agg.Add("agent-a", t0, snapshot("run-1/agent-a", 1, []*agentpb.CounterValue{
		counter("sessions.attempted", 100, web("client")...),
		counter("sessions.attempted", 70, web("server")...),
	}))
}

func attemptedOf(t *testing.T, body []byte) float64 {
	t.Helper()
	var set SeriesSet
	if err := json.Unmarshal(body, &set); err != nil {
		t.Fatal(err)
	}
	return sampleOf(t, set.Samples, RunKey).Values["sessions.attempted"]
}

// TestGetRunMetricsSide checks CON-12: side=server answers with the server
// side, and an omitted side or side=client keeps the client side answer.
func TestGetRunMetricsSide(t *testing.T) {
	h := newREST(t)
	h.feedSides()
	for query, want := range map[string]float64{
		"view=application":             100,
		"view=application&side=client": 100,
		"view=application&side=server": 70,
	} {
		rec := h.get(t, "/runs/{id}/metrics", "run-1", query)
		if rec.Code != http.StatusOK {
			t.Fatalf("%q: status %d: %s", query, rec.Code, rec.Body)
		}
		if got := attemptedOf(t, rec.Body.Bytes()); got != want {
			t.Errorf("%q: sessions.attempted = %v, want %v", query, got, want)
		}
	}
	if rec := h.get(t, "/runs/{id}/metrics", "run-1", "view=application&side=both"); rec.Code != http.StatusBadRequest {
		t.Errorf("side=both: status %d, want 400", rec.Code)
	}
}

// TestStreamRunMetricsSide checks that the stream takes the same parameter.
func TestStreamRunMetricsSide(t *testing.T) {
	h := newREST(t)
	h.feedSides()
	h.runs.set("run-1", runs.Completed)
	srv := httptest.NewServer(h.handler)
	t.Cleanup(srv.Close)
	for query, want := range map[string]float64{"": 100, "&side=client": 100, "&side=server": 70} {
		req, _ := http.NewRequest(http.MethodGet, srv.URL+"/v1/runs/run-1/metrics:stream?view=application"+query, nil)
		req.Header.Set("Authorization", "Bearer token-read")
		resp, err := srv.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		events := readEvents(t, resp.Body, func(e event) bool { return e.name == "end" })
		_ = resp.Body.Close()
		var got float64
		for _, e := range events {
			var s Sample
			if json.Unmarshal([]byte(e.data), &s) == nil && s.Key == RunKey {
				got = s.Values["sessions.attempted"]
			}
		}
		if got != want {
			t.Errorf("stream %q: sessions.attempted = %v, want %v", query, got, want)
		}
	}
}
