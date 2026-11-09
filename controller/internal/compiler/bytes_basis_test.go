// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"errors"
	"math"
	"path/filepath"
	"strings"
	"testing"
)

// bytesBasisPlan compiles the enterprise example (basis bytes, 10 Gbit/s, one
// agent) with the given change of the request.
func bytesBasisPlan(t *testing.T, mutate func(*Request)) (Plan, error) {
	t.Helper()
	req := Request{
		RunID:     "run-0001",
		Config:    loadConfig(t, filepath.Join(contracts, "examples", "tests", "enterprise_mix_10g.yaml")),
		Agents:    []Agent{{ID: "agent-a", Cores: 8, Backend: "dpdk"}},
		Scenarios: exampleCatalog(t),
	}
	if mutate != nil {
		mutate(&req)
	}
	plans, err := Compile(req)
	if err != nil {
		return Plan{}, err
	}
	return plans[0], nil
}

// TestBytesBasisSessionShares checks that the basis bytes turns the byte
// weights into session shares: the shares are proportional to weight over
// bytes per session, they sum to one, sessions at these shares carry the byte
// weights again, and the session rates carry the throughput objective.
func TestBytesBasisSessionShares(t *testing.T) {
	plan, err := bytesBasisPlan(t, nil)
	if err != nil {
		t.Fatal(err)
	}
	byteWeight := map[string]float64{"http1/web_browse": 0.6, "http1/web_get": 0.399, "dns/lookup": 0.001}
	var wantSum float64
	for ref, w := range byteWeight {
		wantSum += w / bytesPerSession[ref]
	}
	var sum, carried float64
	shares := map[string]float64{}
	for _, m := range plan.Mix {
		want := byteWeight[m.Scenario] / bytesPerSession[m.Scenario] / wantSum
		if math.Abs(m.Weight-want) > 1e-12 {
			t.Errorf("session share of %s is %v, want %v", m.ID, m.Weight, want)
		}
		sum += m.Weight
		shares[m.ID] = m.Weight
		carried += m.Weight * bytesPerSession[m.Scenario]
	}
	if len(plan.Mix) != 3 {
		t.Fatalf("%d mix entries, want 3", len(plan.Mix))
	}
	if math.Abs(sum-1) > 1e-12 {
		t.Errorf("session shares sum to %v, want 1", sum)
	}
	for _, m := range plan.Mix {
		if got := m.Weight * bytesPerSession[m.Scenario] / carried; math.Abs(got-byteWeight[m.Scenario]) > 1e-12 {
			t.Errorf("byte share of %s is %v, want its weight %v", m.ID, got, byteWeight[m.Scenario])
		}
	}

	// The total of the worker rates, in bits per second, is the objective, and
	// the rates keep the session shares.
	rate := map[string]float64{}
	for _, w := range plan.Workers {
		for _, r := range w.Rates {
			rate[r.MixEntry] += *r.SessionsPerSecond
		}
	}
	var bps, total float64
	for _, m := range plan.Mix {
		bps += 8 * rate[m.ID] * bytesPerSession[m.Scenario]
		total += rate[m.ID]
	}
	if math.Abs(bps/10e9-1) > 1e-9 {
		t.Errorf("worker rates carry %v bit/s, want 10e9", bps)
	}
	for id, s := range shares {
		if math.Abs(rate[id]/total-s) > 1e-9 {
			t.Errorf("rate share of %s is %v, want %v", id, rate[id]/total, s)
		}
	}
}

// TestBytesBasisNeedsEstimates checks that a scenario without a bytes per
// session estimate fails the compilation for the basis bytes whatever the
// objective, and that the basis sessions does not need one.
func TestBytesBasisNeedsEstimates(t *testing.T) {
	zero := func(r *Request) {
		r.Config.Load.Objective = Objective{Type: "sessions_per_second", Value: 1000}
		for i := range r.Scenarios {
			if r.Scenarios[i].Name == "lookup" {
				r.Scenarios[i].BytesPerSession = 0
			}
		}
	}
	_, err := bytesBasisPlan(t, zero)
	if !errors.Is(err, ErrNoByteEstimate) || !strings.Contains(err.Error(), "dns/lookup") {
		t.Fatalf("error %v, want ErrNoByteEstimate naming dns/lookup", err)
	}
	plan, err := bytesBasisPlan(t, func(r *Request) {
		zero(r)
		r.Config.Profile.Basis = "sessions"
	})
	if err != nil {
		t.Fatalf("basis sessions: %v", err)
	}
	for _, m := range plan.Mix {
		if m.ID == "lookup" && math.Abs(m.Weight-0.001) > 1e-12 {
			t.Errorf("session weight of lookup is %v, want the configured 0.001", m.Weight)
		}
	}
}
