// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"context"
	"fmt"
	"math/rand/v2"
	"strconv"
	"sync"
	"testing"
	"time"

	"google.golang.org/protobuf/encoding/protojson"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

// pipeline is an agent manager whose observer feeds an aggregator, with fake
// agents that dial it.
type pipeline struct {
	agg  *Aggregator
	dial fakeagent.Dialer
	mu   sync.Mutex
	seen int
}

func newPipeline(t *testing.T) *pipeline {
	t.Helper()
	agg, err := New(Config{})
	if err != nil {
		t.Fatal(err)
	}
	p := &pipeline{agg: agg}
	mgr, err := agents.New(agents.Config{
		AutoEnroll: true,
		Identity:   agents.ClaimedIdentity,
		Observer: func(ev agents.Event) {
			agg.Observe(ev)
			if ev.Kind == agents.EventSnapshot {
				p.mu.Lock()
				p.seen++
				p.mu.Unlock()
			}
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(mgr.Shutdown)
	p.dial = fakeagent.Serve(t, mgr)
	return p
}

func (p *pipeline) agent(t *testing.T, id string) *fakeagent.Agent {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	a, err := fakeagent.Start(ctx, fakeagent.Options{ID: id, Dial: p.dial})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	return a
}

// waitSeen waits until the manager delivered n snapshot events.
func (p *pipeline) waitSeen(t *testing.T, n int) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		p.mu.Lock()
		seen := p.seen
		p.mu.Unlock()
		if seen >= n {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("saw fewer than %d snapshots", n)
}

func cv(metric string, n uint64, kv ...string) *agentpb.CounterValue {
	labels := map[string]string{}
	for i := 0; i+1 < len(kv); i += 2 {
		labels[kv[i]] = kv[i+1]
	}
	return &agentpb.CounterValue{Metric: metric, Labels: labels, Value: &agentpb.CounterValue_Count{Count: n}}
}

// TestCountersEqualSumOfFakeAgentSnapshots sends cumulative snapshots from
// three fake agents, one of them per worker, with repeated and stale
// sequences, and checks that every merged counter equals the sum of the
// newest snapshot of every agent and worker.
func TestCountersEqualSumOfFakeAgentSnapshots(t *testing.T) {
	p := newPipeline(t)
	rng := rand.New(rand.NewPCG(7, 8)) //nolint:gosec // G404: seeded test data
	type key struct{ metric, app, sc string }
	keys := []key{
		{"sessions.attempted", "http1", "web_get"},
		{"sessions.attempted", "dns", "query"},
		{"sessions.succeeded", "http1", "web_get"},
		{"bytes.tx", "http1", "web_get"},
		{"bytes.rx", "dns", "query"},
	}
	type src struct {
		agent  string
		worker uint32
		agg    bool
	}
	sources := []src{{"agent-a", 0, true}, {"agent-b", 1, false}, {"agent-b", 2, false}, {"agent-c", 0, true}}
	fakes := map[string]*fakeagent.Agent{}
	for _, s := range sources {
		if fakes[s.agent] == nil {
			fakes[s.agent] = p.agent(t, s.agent)
		}
	}
	newest := map[src]map[key]uint64{}
	seq := map[string]uint64{}
	sent := 0
	for round := 0; round < 6; round++ {
		for _, s := range sources {
			cum := newest[s]
			if cum == nil {
				cum = map[key]uint64{}
				newest[s] = cum
			}
			snap := &agentpb.CounterSnapshot{PlanId: "run-1/" + s.agent, WorkerId: s.worker, Aggregate: s.agg}
			for _, k := range keys {
				cum[k] += rng.Uint64N(1 << 30)
				snap.Counters = append(snap.Counters, cv(k.metric, cum[k], "application", k.app, "scenario", k.sc))
			}
			seq[s.agent]++
			snap.Sequence = seq[s.agent]
			if err := fakes[s.agent].SendSnapshot(snap); err != nil {
				t.Fatal(err)
			}
			sent++
		}
	}
	p.waitSeen(t, sent)
	// A replay of an old sequence, as after a reconnection, changes nothing.
	old := &agentpb.CounterSnapshot{PlanId: "run-1/agent-a", Sequence: 1, Aggregate: true,
		Counters: []*agentpb.CounterValue{cv("sessions.attempted", 1, "application", "http1", "scenario", "web_get")}}
	if p.agg.Add("agent-a", time.Time{}, old) {
		t.Fatal("stale snapshot accepted")
	}
	// A final snapshot without counters keeps the cumulative values.
	if !p.agg.Add("agent-c", time.Time{}, &agentpb.CounterSnapshot{PlanId: "run-1/agent-c", Sequence: seq["agent-c"] + 1, Aggregate: true, IsFinal: true}) {
		t.Fatal("final snapshot rejected")
	}

	want := map[key]uint64{}
	for _, cum := range newest {
		for k, v := range cum {
			want[k] += v
		}
	}
	series, err := p.agg.Series("run-1")
	if err != nil {
		t.Fatal(err)
	}
	got := map[key]uint64{}
	for _, s := range series {
		if s.Metric.Kind != Counter {
			continue
		}
		app, _ := s.Label("application")
		sc, _ := s.Label("scenario")
		got[key{s.Metric.Name, app, sc}] = s.Count
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("%v = %d, want the sum %d", k, got[k], v)
		}
	}
	if len(got) != len(want) {
		t.Errorf("got %d counter series, want %d", len(got), len(want))
	}
	totals, err := p.agg.Totals("run-1")
	if err != nil {
		t.Fatal(err)
	}
	var attempted uint64
	for k, v := range want {
		if k.metric == "sessions.attempted" {
			attempted += v
		}
	}
	if totals["sessions.attempted"] != float64(attempted) {
		t.Errorf("total sessions.attempted = %v, want %d", totals["sessions.attempted"], attempted)
	}
}

// TestHistogramsOfFakeAgentsMatchReference sends the snapshots of the Rust
// reference through fake agents and checks the merged histograms and
// counters against the reference merge.
func TestHistogramsOfFakeAgentsMatchReference(t *testing.T) {
	ref := loadReference(t)
	p := newPipeline(t)
	fakes := map[string]*fakeagent.Agent{}
	for _, s := range ref.Snapshots {
		if fakes[s.Agent] == nil {
			fakes[s.Agent] = p.agent(t, s.Agent)
		}
		var snap agentpb.CounterSnapshot
		if err := protojson.Unmarshal(s.Snapshot, &snap); err != nil {
			t.Fatal(err)
		}
		if err := fakes[s.Agent].SendSnapshot(&snap); err != nil {
			t.Fatal(err)
		}
	}
	p.waitSeen(t, len(ref.Snapshots))
	series, err := p.agg.Series("run-1")
	if err != nil {
		t.Fatal(err)
	}
	find := func(metric string, labels map[string]string) *Series {
		for _, s := range series {
			if s.Metric.Name != metric || len(s.Labels) != len(labels) {
				continue
			}
			match := true
			for k, v := range labels {
				if got, _ := s.Label(k); got != v {
					match = false
				}
			}
			if match {
				return s
			}
		}
		return nil
	}
	for _, c := range ref.Merged.Counters {
		s := find(c.Metric, c.Labels)
		if s == nil {
			t.Fatalf("missing %s %v", c.Metric, c.Labels)
		}
		if c.Count != "" && strconv.FormatUint(s.Count, 10) != c.Count {
			t.Errorf("%s %v = %d, reference %s", c.Metric, c.Labels, s.Count, c.Count)
		}
	}
	for _, rh := range ref.Merged.Histograms {
		s := find(rh.Metric, rh.Labels)
		if s == nil {
			t.Fatalf("missing %s %v", rh.Metric, rh.Labels)
		}
		h := s.Hist
		for name, want := range map[string]string{
			"count": rh.Count, "min": rh.Min, "max": rh.Max,
			"p50": rh.P50, "p90": rh.P90, "p99": rh.P99, "p999": rh.P999,
		} {
			var st Stat
			for _, x := range Stats {
				if x.Name == name {
					st = x
				}
			}
			if got := fmt.Sprintf("%.0f", h.Value(st)); got != want {
				t.Errorf("%s %v %s = %s, reference %s", rh.Metric, rh.Labels, name, got, want)
			}
		}
	}
}
