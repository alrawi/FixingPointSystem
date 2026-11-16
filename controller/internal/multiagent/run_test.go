// SPDX-License-Identifier: GPL-3.0-only

package multiagent_test

import (
	"context"
	"errors"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/multiagent"
)

// testConfig is a small app_sim test. The server pool has enough hosts for
// three agents with two workers each.
const testConfig = `apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata: { name: multi_test }
topology: app_sim
network:
  ports:
    - { name: client0, location: eth1 }
    - { name: server0, location: eth1 }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 16.0.0.0/24, count: 254 }
    - { name: servers, port: server0, role: server, ipv4: 48.0.0.0/24, count: 60 }
profile:
  basis: sessions
  mix:
    - { scenario: http1/web_get, weight: 1 }
load:
  objective: { type: sessions_per_second, value: 1000 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 7 }
criteria:
  - { metric: sessions.failed_ratio, op: "<", value: 0.1 }
`

var catalog = []compiler.Scenario{{
	Module: compiler.ModuleRef{Name: "http1", Version: "0.1.0", Digest: "sha256:" + strings.Repeat("0", 64)},
	Name:   "web_get",
	Source: "name: web_get\n",
}}

// fixedClock is a controller clock that never moves.
type fixedClock struct{ t time.Time }

func (c fixedClock) Now() time.Time { return c.t }

var epoch = time.Date(2026, 11, 16, 12, 0, 0, 0, time.UTC)

// harness is a real agent manager with fake agents over an in-memory gRPC
// listener, and the real compiler.
type harness struct {
	mgr  *agents.Manager
	dial fakeagent.Dialer
	run  atomic.Pointer[multiagent.Run]
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	h := &harness{}
	mgr, err := agents.New(agents.Config{
		HeartbeatInterval: time.Second,
		HeartbeatTimeout:  time.Minute,
		AutoEnroll:        true,
		Identity:          agents.ClaimedIdentity,
		Clock:             fixedClock{epoch},
		Observer: func(ev agents.Event) {
			if r := h.run.Load(); r != nil {
				r.Observe(ev)
			}
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	h.mgr = mgr
	h.dial = fakeagent.Serve(t, mgr)
	return h
}

func (h *harness) fakes(t *testing.T, onPrepare func(id string) *agentpb.CommandError, ids ...string) []*fakeagent.Agent {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	t.Cleanup(cancel)
	out := make([]*fakeagent.Agent, len(ids))
	for i, id := range ids {
		opts := fakeagent.Options{ID: id, Dial: h.dial}
		if onPrepare != nil {
			opts.OnCommand = func(cmd *agentpb.SessionResponse) *agentpb.CommandError {
				if _, ok := cmd.GetBody().(*agentpb.SessionResponse_Prepare); ok {
					return onPrepare(id)
				}
				return nil
			}
		}
		a, err := fakeagent.Start(ctx, opts)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(a.Close)
		out[i] = a
	}
	waitFor(t, "every fake agent to be online", func() bool {
		for _, id := range ids {
			if a, err := h.mgr.Get(id); err != nil || a.State != agents.StateOnline {
				return false
			}
		}
		return true
	})
	return out
}

func (h *harness) coordinator(t *testing.T, clocks multiagent.ClockSource) *multiagent.Coordinator {
	t.Helper()
	c, err := multiagent.New(multiagent.Config{
		Agents: h.mgr, Clocks: clocks, Clock: fixedClock{epoch},
		StartLead: 3 * time.Second, CommandTimeout: 10 * time.Second, DrainTimeout: time.Second,
	})
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func request(t *testing.T, ids ...string) multiagent.Request {
	t.Helper()
	cfg, err := compiler.ParseConfig([]byte(testConfig))
	if err != nil {
		t.Fatal(err)
	}
	return multiagent.Request{RunID: "run-1", Config: cfg, Agents: ids, Scenarios: catalog, Seed: 1}
}

func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for !cond() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(2 * time.Millisecond)
	}
}

func startCommand(t *testing.T, a *fakeagent.Agent) *agentpb.StartCommand {
	t.Helper()
	for _, c := range a.Commands() {
		if s := c.GetStart(); s != nil {
			return s
		}
	}
	t.Fatalf("agent %s received no start command", a.ID())
	return nil
}

// TestThreeAgentsAggregateCountersExactly is the first acceptance item of
// CTL-16. Each agent reports counters above 2^53, where a float64 sum loses
// the low bits, and the totals must still equal the exact integer sums.
func TestThreeAgentsAggregateCountersExactly(t *testing.T) {
	h := newHarness(t)
	fakes := h.fakes(t, nil, "agent-a", "agent-b", "agent-c")
	clocks := offsets{"agent-a": 0, "agent-b": 4 * time.Millisecond, "agent-c": -7 * time.Millisecond}
	ctx := context.Background()

	run, err := h.coordinator(t, clocks).Start(ctx, request(t, "agent-a", "agent-b", "agent-c"))
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	h.run.Store(run)

	if got := len(run.Plans()); got != 3 {
		t.Fatalf("%d plans, want 3", got)
	}
	for _, f := range fakes {
		if s, _ := f.State(); s != agentpb.AgentState_AGENT_STATE_RUNNING {
			t.Fatalf("agent %s is %s, want running", f.ID(), s)
		}
	}

	const big = uint64(1<<53) + 1
	attempted := []uint64{big, big, big + 2}
	succeeded := []uint64{big - 1, big - 3, big - 5}
	for i, f := range fakes {
		// A first snapshot with lower values must be replaced by the second.
		if _, err := f.Snapshot(counter("sessions.attempted", 10), counter("sessions.succeeded", 9)); err != nil {
			t.Fatal(err)
		}
		if _, err := f.Snapshot(counter("sessions.attempted", attempted[i]), counter("sessions.succeeded", succeeded[i])); err != nil {
			t.Fatal(err)
		}
	}
	wantAttempted := 3*big + 2
	wantSucceeded := 3*big - 9
	waitFor(t, "the snapshots of three agents", func() bool {
		tot, err := run.Collector().Totals()
		return err == nil && tot["sessions.attempted"] == wantAttempted && tot["sessions.succeeded"] == wantSucceeded
	})

	res, err := run.Stop(ctx, agentpb.StopMode_STOP_MODE_RAMP_DOWN)
	if err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if res.Totals["sessions.attempted"] != wantAttempted || res.Totals["sessions.succeeded"] != wantSucceeded {
		t.Fatalf("totals %v, want attempted %d and succeeded %d", res.Totals, wantAttempted, wantSucceeded)
	}
	if len(res.Final) != 3 {
		t.Fatalf("final snapshots from %v, want all three agents", res.Final)
	}
	if _, err := run.Stop(ctx, agentpb.StopMode_STOP_MODE_RAMP_DOWN); !errors.Is(err, multiagent.ErrStopped) {
		t.Fatalf("second Stop: %v, want ErrStopped", err)
	}
	for _, f := range fakes {
		a, err := h.mgr.Get(f.ID())
		if err != nil {
			t.Fatal(err)
		}
		if a.ReservedBy != "" {
			t.Fatalf("agent %s is still reserved by %s", f.ID(), a.ReservedBy)
		}
	}
}

// TestStartInstantFollowsEachAgentClock checks that all agents start at one
// controller instant, which the run converts to the clock of each agent.
func TestStartInstantFollowsEachAgentClock(t *testing.T) {
	h := newHarness(t)
	fakes := h.fakes(t, nil, "agent-a", "agent-b", "agent-c")
	clocks := offsets{"agent-a": 0, "agent-b": 4 * time.Millisecond, "agent-c": -7 * time.Millisecond}
	run, err := h.coordinator(t, clocks).Start(context.Background(), request(t, "agent-a", "agent-b", "agent-c"))
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	h.run.Store(run)
	t.Cleanup(func() { _, _ = run.Stop(context.Background(), agentpb.StopMode_STOP_MODE_ABORT) })

	if want := epoch.Add(3 * time.Second); !run.StartAt().Equal(want) {
		t.Fatalf("StartAt %s, want %s", run.StartAt(), want)
	}
	for _, f := range fakes {
		at := startCommand(t, f).GetStartAt().AsTime()
		if got := at.Add(-clocks[f.ID()]); !got.Equal(run.StartAt()) {
			t.Fatalf("agent %s starts at %s on its clock, which is %s on the controller clock, want %s",
				f.ID(), at, got, run.StartAt())
		}
	}
}

// TestRefusesToStartWithAnOffsetAboveTenMilliseconds is the second acceptance
// item of CTL-16. The refusal comes before any command, so the agents stay
// idle and unreserved.
func TestRefusesToStartWithAnOffsetAboveTenMilliseconds(t *testing.T) {
	h := newHarness(t)
	fakes := h.fakes(t, nil, "agent-a", "agent-b", "agent-c")
	clocks := offsets{"agent-a": 0, "agent-b": 10*time.Millisecond + time.Microsecond, "agent-c": -3 * time.Millisecond}

	run, err := h.coordinator(t, clocks).Start(context.Background(), request(t, "agent-a", "agent-b", "agent-c"))
	if run != nil {
		t.Fatal("Start returned a run")
	}
	if !errors.Is(err, multiagent.ErrClockOffset) {
		t.Fatalf("error %v, want ErrClockOffset", err)
	}
	var ce *multiagent.ClockError
	if !errors.As(err, &ce) || len(ce.Offenders) != 1 || ce.Offenders[0].AgentID != "agent-b" {
		t.Fatalf("error %v does not name agent-b as the only offender", err)
	}
	for _, f := range fakes {
		if cmds := f.Commands(); len(cmds) != 0 {
			t.Fatalf("agent %s received %d commands after the refusal", f.ID(), len(cmds))
		}
		a, err := h.mgr.Get(f.ID())
		if err != nil {
			t.Fatal(err)
		}
		if a.ReservedBy != "" {
			t.Fatalf("agent %s is reserved by %s after the refusal", f.ID(), a.ReservedBy)
		}
	}
}

func TestStartAcceptsAnOffsetOfExactlyTenMilliseconds(t *testing.T) {
	h := newHarness(t)
	h.fakes(t, nil, "agent-a", "agent-b")
	clocks := offsets{"agent-a": 10 * time.Millisecond, "agent-b": -10 * time.Millisecond}
	run, err := h.coordinator(t, clocks).Start(context.Background(), request(t, "agent-a", "agent-b"))
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	h.run.Store(run)
	if _, err := run.Stop(context.Background(), agentpb.StopMode_STOP_MODE_ABORT); err != nil {
		t.Fatalf("Stop: %v", err)
	}
}

func TestStartWithoutAMeasurementIsRefused(t *testing.T) {
	h := newHarness(t)
	h.fakes(t, nil, "agent-a", "agent-b")
	_, err := h.coordinator(t, offsets{"agent-a": 0}).Start(context.Background(), request(t, "agent-a", "agent-b"))
	if !errors.Is(err, multiagent.ErrNoClockOffset) {
		t.Fatalf("error %v, want ErrNoClockOffset", err)
	}
}

// TestFailedPrepareAbortsEveryAgentAndReleases checks that a start that fails
// after the reservation leaves every agent idle and free.
func TestFailedPrepareAbortsEveryAgentAndReleases(t *testing.T) {
	h := newHarness(t)
	var mu sync.Mutex
	prepared := map[string]bool{}
	fakes := h.fakes(t, func(id string) *agentpb.CommandError {
		if id == "agent-b" {
			return &agentpb.CommandError{Code: agentpb.ErrorCode_ERROR_CODE_INTERNAL, Message: "disk full"}
		}
		mu.Lock()
		prepared[id] = true
		mu.Unlock()
		return nil
	}, "agent-a", "agent-b", "agent-c")
	clocks := offsets{"agent-a": 0, "agent-b": 0, "agent-c": 0}

	run, err := h.coordinator(t, clocks).Start(context.Background(), request(t, "agent-a", "agent-b", "agent-c"))
	if err == nil || run != nil {
		t.Fatalf("Start returned %v, %v, want an error", run, err)
	}
	if !strings.Contains(err.Error(), "agent-b") || !strings.Contains(err.Error(), "disk full") {
		t.Fatalf("error %q does not name the failed agent and the reason", err)
	}
	for _, id := range []string{"agent-a", "agent-c"} {
		mu.Lock()
		ok := prepared[id]
		mu.Unlock()
		if !ok {
			t.Fatalf("agent %s was never prepared", id)
		}
	}
	waitFor(t, "the prepared agents to return to idle", func() bool {
		for _, f := range fakes {
			if s, _ := f.State(); s == agentpb.AgentState_AGENT_STATE_PREPARED || s == agentpb.AgentState_AGENT_STATE_RUNNING {
				return false
			}
		}
		return true
	})
	for _, f := range fakes {
		a, err := h.mgr.Get(f.ID())
		if err != nil {
			t.Fatal(err)
		}
		if a.ReservedBy != "" {
			t.Fatalf("agent %s is still reserved by %s", f.ID(), a.ReservedBy)
		}
	}
}

func TestStartNeedsAgentsAndARunID(t *testing.T) {
	h := newHarness(t)
	c := h.coordinator(t, offsets{})
	if _, err := c.Start(context.Background(), request(t)); !errors.Is(err, multiagent.ErrNoAgents) {
		t.Fatalf("error %v, want ErrNoAgents", err)
	}
	req := request(t, "agent-a")
	req.RunID = ""
	if _, err := c.Start(context.Background(), req); err == nil {
		t.Fatal("a request without a run id started")
	}
}

func TestNewRequiresAgentsAndClocks(t *testing.T) {
	if _, err := multiagent.New(multiagent.Config{}); err == nil {
		t.Fatal("New accepted an empty config")
	}
}

func counter(metric string, n uint64) *agentpb.CounterValue {
	return &agentpb.CounterValue{Metric: metric, Value: &agentpb.CounterValue_Count{Count: n}}
}
