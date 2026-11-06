// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"context"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// testConfig is a small app_sim test whose criterion allows up to 10% failed sessions.
const testConfig = `apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata: { name: run_test }
topology: app_sim
network:
  ports:
    - { name: client0, location: local/eth1 }
    - { name: server0, location: local/eth1 }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 16.0.0.0/24, count: 254 }
    - { name: servers, port: server0, role: server, ipv4: 48.0.0.0/24, count: 4 }
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
  - { metric: mix.max_abs_error_pp, op: "<=", value: 2 }
`

// memStore is an in memory ConfigStore.
type memStore map[string]string

func (m memStore) Get(_ context.Context, id string) (store.Record, error) {
	body, ok := m[id]
	if !ok {
		return store.Record{}, fmt.Errorf("get config %s: %w", id, store.ErrConfigNotFound)
	}
	return store.Record{ID: id, Name: "run_test", Body: []byte(body)}, nil
}

// catalog serves one http1/web_get scenario.
type catalog struct{}

func (catalog) Scenarios(context.Context) ([]compiler.Scenario, error) {
	return []compiler.Scenario{{
		Module: compiler.ModuleRef{Name: "http1", Version: "0.1.0", Digest: "sha256:" + strings.Repeat("0", 64)},
		Name:   "web_get",
		Source: "name: web_get\n",
	}}, nil
}

// fakeClock is a manual clock for the agent manager.
type fakeClock struct {
	mu  sync.Mutex
	now time.Time
}

func (c *fakeClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *fakeClock) Advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.now = c.now.Add(d)
}

// harness runs a real agent manager, fake agents over an in-memory gRPC
// listener, the real compiler, and the orchestrator under test.
type harness struct {
	clock *fakeClock
	mgr   *agents.Manager
	dial  fakeagent.Dialer
	orch  *runs.Orchestrator
}

func newHarness(t *testing.T, mutate func(*runs.Config)) *harness {
	t.Helper()
	h := &harness{clock: &fakeClock{now: time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)}}
	var orch atomic.Pointer[runs.Orchestrator]
	mgr, err := agents.New(agents.Config{
		HeartbeatInterval: time.Second,
		HeartbeatTimeout:  3 * time.Second,
		AutoEnroll:        true,
		Identity:          agents.ClaimedIdentity,
		Clock:             h.clock,
		Observer: func(ev agents.Event) {
			if o := orch.Load(); o != nil {
				o.Observe(ev)
			}
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	h.mgr = mgr
	h.dial = fakeagent.Serve(t, mgr)
	var n atomic.Int64
	cfg := runs.Config{
		Store:          memStore{"cfg-1": testConfig, "broken": "kind: [", "no-mix": strings.Replace(testConfig, "http1/web_get", "dns/lookup", 1)},
		Agents:         mgr,
		Catalog:        catalog{},
		Clock:          h.clock,
		NewID:          func() (string, error) { return fmt.Sprintf("run-%d", n.Add(1)), nil },
		Seed:           func() uint64 { return 1 },
		CommandTimeout: 5 * time.Second,
		DrainTimeout:   time.Second,
	}
	if mutate != nil {
		mutate(&cfg)
	}
	o, err := runs.New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	orch.Store(o)
	h.orch = o
	t.Cleanup(o.Close)
	return h
}

func (h *harness) agent(t *testing.T, id string, onCommand func(*agentpb.SessionResponse) *agentpb.CommandError) *fakeagent.Agent {
	t.Helper()
	a, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: id, Dial: h.dial, OnCommand: onCommand})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	return a
}

func testContext(t *testing.T) context.Context {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	return ctx
}

// waitFor polls cond until it holds or ten seconds pass.
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

// start creates a run on the agents and waits until every agent runs it.
func (h *harness) start(t *testing.T, fakes ...*fakeagent.Agent) runs.Run {
	t.Helper()
	ids := make([]string, len(fakes))
	for i, f := range fakes {
		ids[i] = f.ID()
	}
	run, err := h.orch.Create(testContext(t), runs.StartRequest{ConfigID: "cfg-1", Agents: ids})
	if err != nil {
		t.Fatal(err)
	}
	if run.State != runs.Pending {
		t.Fatalf("new run in state %s, want pending", run.State)
	}
	h.waitRunning(t, run.ID, fakes...)
	return run
}

func (h *harness) waitRunning(t *testing.T, id string, fakes ...*fakeagent.Agent) {
	t.Helper()
	waitFor(t, "a running run", func() bool {
		r, err := h.orch.Get(id)
		if err != nil || r.State != runs.Running {
			return false
		}
		for _, f := range fakes {
			if s, _ := f.State(); s != agentpb.AgentState_AGENT_STATE_RUNNING {
				return false
			}
		}
		return true
	})
}

func (h *harness) wait(t *testing.T, id string) runs.Run {
	t.Helper()
	r, err := h.orch.Wait(testContext(t), id)
	if err != nil {
		t.Fatal(err)
	}
	if !runs.Final(r.State) {
		t.Fatalf("run %s did not end: state %s", id, r.State)
	}
	return r
}

func count(metric string, n uint64) *agentpb.CounterValue {
	return &agentpb.CounterValue{Metric: metric, Value: &agentpb.CounterValue_Count{Count: n}}
}

func crit(metric, op string, value float64) []runs.Criterion {
	return []runs.Criterion{{CriterionConfig: compiler.CriterionConfig{Metric: metric, Op: op, Value: value}}}
}

// sessions sends one snapshot with session counters and waits until the
// controller processed it.
func sessions(t *testing.T, a *fakeagent.Agent, attempted, succeeded, failed uint64) {
	t.Helper()
	if _, err := a.Snapshot(count("sessions.attempted", attempted), count("sessions.succeeded", succeeded),
		count("sessions.failed", failed), count("bytes.tx", 1000)); err != nil {
		t.Fatal(err)
	}
	if err := a.Heartbeat(testContext(t)); err != nil {
		t.Fatal(err)
	}
}

// stops returns the stop modes that a fake agent received.
func stops(a *fakeagent.Agent) []agentpb.StopMode {
	var out []agentpb.StopMode
	for _, c := range a.Commands() {
		if s := c.GetStop(); s != nil {
			out = append(out, s.GetMode())
		}
	}
	return out
}

func TestRunCompletesWhenAgentsFinish(t *testing.T) {
	h := newHarness(t, nil)
	a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
	run := h.start(t, a, b)

	got, _ := h.mgr.Get("agent-a")
	if got.ReservedBy != run.ID {
		t.Fatalf("agent-a reserved by %q during the run, want %s", got.ReservedBy, run.ID)
	}
	sessions(t, a, 100, 100, 0)
	sessions(t, b, 50, 50, 0)
	for _, f := range []*fakeagent.Agent{a, b} {
		if err := f.Finish(); err != nil {
			t.Fatal(err)
		}
	}
	end := h.wait(t, run.ID)
	if end.State != runs.Completed || end.Degraded || end.Verdict == nil || !end.Verdict.Passed {
		t.Fatalf("run ended %+v, want completed and not degraded", end)
	}
	if end.Totals["sessions.attempted"] != 150 || end.Totals["sessions.failed_ratio"] != 0 || end.Totals["bytes.tx"] != 2000 {
		t.Fatalf("totals %v", end.Totals)
	}
	if c := end.Verdict.Criteria; len(c) != 2 || !c[0].Evaluated || c[1].Evaluated {
		t.Fatalf("criteria %+v, want the ratio evaluated and the mix error skipped", c)
	}
	if end.StartedAt.IsZero() || end.EndedAt.IsZero() || end.Seed != 7 || end.ModuleVersions["http1"] != "0.1.0" {
		t.Fatalf("run metadata %+v", end)
	}
	for _, f := range []*fakeagent.Agent{a, b} {
		if s := stops(f); len(s) != 0 {
			t.Errorf("%s got stop commands %v after it finished", f.ID(), s)
		}
		ag, _ := h.mgr.Get(f.ID())
		if ag.ReservedBy != "" {
			t.Errorf("%s still reserved by %q", f.ID(), ag.ReservedBy)
		}
	}
}

// TestFailingAgentEndsRunByCriteria is the acceptance test of fault
// containment. One of two agents fails or crashes while the run is under way.
// The run turns degraded, the other agent keeps running, and the session
// failure ratio against the criterion of 10% decides the end state.
func TestFailingAgentEndsRunByCriteria(t *testing.T) {
	tests := []struct {
		name              string
		crash             bool
		succeeded, failed uint64
		want              runs.State
		wantRatioAtLeast  float64
		wantRatioBelow    float64
	}{
		// agent-b had 100 attempted, 99 succeeded, 1 failed, so 1 of 200 failed.
		{"failed agent within criteria", false, 99, 1, runs.Completed, 0.005, 0.0051},
		// 40 succeeded, 10 failed, 50 open sessions end with the agent, so 60 of 200 failed.
		{"failed agent breaks criteria", false, 40, 10, runs.Failed, 0.3, 0.3001},
		{"crashed agent within criteria", true, 99, 1, runs.Completed, 0.005, 0.0051},
		{"crashed agent breaks criteria", true, 40, 10, runs.Failed, 0.3, 0.3001},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			h := newHarness(t, nil)
			a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
			run := h.start(t, a, b)
			sessions(t, a, 100, 100, 0)
			sessions(t, b, 100, tc.succeeded, tc.failed)
			if tc.crash {
				b.Crash()
			} else if err := b.Fail("worker panic"); err != nil {
				t.Fatal(err)
			}
			waitFor(t, "a degraded run", func() bool {
				r, _ := h.orch.Get(run.ID)
				return r.Degraded
			})
			mid, _ := h.orch.Get(run.ID)
			if mid.State != runs.Running || len(mid.FailedAgents) != 1 || mid.FailedAgents[0] != "agent-b" {
				t.Fatalf("after the fault the run is %+v, want running with agent-b failed", mid)
			}
			if s, _ := a.State(); s != agentpb.AgentState_AGENT_STATE_RUNNING {
				t.Fatalf("agent-a is %s after the fault of agent-b", s)
			}
			if err := a.Finish(); err != nil {
				t.Fatal(err)
			}
			end := h.wait(t, run.ID)
			if end.State != tc.want || !end.Degraded {
				t.Fatalf("run ended %s degraded=%v, want %s degraded (reason %q)", end.State, end.Degraded, tc.want, end.Reason)
			}
			ratio := end.Totals["sessions.failed_ratio"]
			if ratio < tc.wantRatioAtLeast || ratio >= tc.wantRatioBelow {
				t.Fatalf("sessions.failed_ratio = %v, want about %v", ratio, tc.wantRatioAtLeast)
			}
			if end.Verdict == nil || end.Verdict.Passed != (tc.want == runs.Completed) {
				t.Fatalf("verdict %+v", end.Verdict)
			}
		})
	}
}

func TestMissedHeartbeatsDegradeRun(t *testing.T) {
	h := newHarness(t, nil)
	a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
	run := h.start(t, a, b)
	// An acknowledged heartbeat flushes the heartbeats that the start sent.
	if err := b.Heartbeat(testContext(t)); err != nil {
		t.Fatal(err)
	}
	b.MissHeartbeats()
	h.clock.Advance(4 * time.Second)
	if err := a.Heartbeat(testContext(t)); err != nil {
		t.Fatal(err)
	}
	if got := h.mgr.Sweep(); len(got) != 1 || got[0] != "agent-b" {
		t.Fatalf("Sweep = %v, want agent-b", got)
	}
	waitFor(t, "a degraded run", func() bool {
		r, _ := h.orch.Get(run.ID)
		return r.Degraded
	})
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	if end := h.wait(t, run.ID); end.State != runs.Completed || !end.Degraded {
		t.Fatalf("run ended %+v, want completed and degraded", end)
	}
}

func TestEveryAgentFailing(t *testing.T) {
	h := newHarness(t, nil)
	a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
	run := h.start(t, a, b)
	if err := a.Fail("boom"); err != nil {
		t.Fatal(err)
	}
	b.Crash()
	end := h.wait(t, run.ID)
	if end.State != runs.Failed || !end.Degraded || len(end.FailedAgents) != 2 {
		t.Fatalf("run ended %+v, want failed with two failed agents", end)
	}
}

func TestGracefulStopRampsDown(t *testing.T) {
	h := newHarness(t, nil)
	a := h.agent(t, "agent-a", nil)
	run := h.start(t, a)
	sessions(t, a, 10, 10, 0)
	if _, err := h.orch.Stop(run.ID, false); err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)
	if end.State != runs.Completed || end.Totals["sessions.attempted"] != 10 {
		t.Fatalf("run ended %+v, want completed with the counters before the stop", end)
	}
	if s := stops(a); len(s) != 1 || s[0] != agentpb.StopMode_STOP_MODE_RAMP_DOWN {
		t.Fatalf("stop commands %v, want one ramp down", s)
	}
	cmds := a.Commands()
	if d := cmds[len(cmds)-1].GetStop().GetDrainTimeout().AsDuration(); d != time.Second {
		t.Fatalf("drain timeout %s, want 1s", d)
	}
	if _, err := h.orch.Stop(run.ID, true); !errors.Is(err, runs.ErrRunFinished) {
		t.Fatalf("stop of an ended run: %v, want ErrRunFinished", err)
	}
}

func TestForceStopAborts(t *testing.T) {
	h := newHarness(t, nil)
	a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
	run := h.start(t, a, b)
	if _, err := h.orch.Stop(run.ID, true); err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)
	if end.State != runs.Aborted || end.Verdict != nil {
		t.Fatalf("run ended %+v, want aborted without a verdict", end)
	}
	for _, f := range []*fakeagent.Agent{a, b} {
		if s := stops(f); len(s) != 1 || s[0] != agentpb.StopMode_STOP_MODE_ABORT {
			t.Errorf("%s stop commands %v, want one abort", f.ID(), s)
		}
	}
}

func TestForceStopDuringRampDownAborts(t *testing.T) {
	release := make(chan struct{})
	h := newHarness(t, nil)
	a := h.agent(t, "agent-a", func(cmd *agentpb.SessionResponse) *agentpb.CommandError {
		if s := cmd.GetStop(); s != nil && s.GetMode() == agentpb.StopMode_STOP_MODE_RAMP_DOWN {
			<-release
		}
		return nil
	})
	run := h.start(t, a)
	if _, err := h.orch.Stop(run.ID, false); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "a stopping run", func() bool {
		r, _ := h.orch.Get(run.ID)
		return r.State == runs.Stopping
	})
	if _, err := h.orch.Stop(run.ID, true); err != nil {
		t.Fatal(err)
	}
	close(release)
	if end := h.wait(t, run.ID); end.State != runs.Aborted {
		t.Fatalf("run ended %s, want aborted", end.State)
	}
}

func TestForceStopWhilePreparing(t *testing.T) {
	release := make(chan struct{})
	h := newHarness(t, func(c *runs.Config) { c.CommandTimeout = 300 * time.Millisecond })
	a := h.agent(t, "agent-a", func(cmd *agentpb.SessionResponse) *agentpb.CommandError {
		if cmd.GetPrepare() != nil {
			<-release
		}
		return nil
	})
	run, err := h.orch.Create(testContext(t), runs.StartRequest{ConfigID: "cfg-1", Agents: []string{a.ID()}})
	if err != nil {
		t.Fatal(err)
	}
	waitFor(t, "a preparing run", func() bool {
		r, _ := h.orch.Get(run.ID)
		return r.State == runs.Preparing
	})
	if _, err := h.orch.Stop(run.ID, true); err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)
	close(release)
	if end.State != runs.Aborted {
		t.Fatalf("run ended %s, want aborted", end.State)
	}
}

func TestPrepareFailureFailsRun(t *testing.T) {
	h := newHarness(t, nil)
	a := h.agent(t, "agent-a", nil)
	b := h.agent(t, "agent-b", func(cmd *agentpb.SessionResponse) *agentpb.CommandError {
		if cmd.GetPrepare() != nil {
			return &agentpb.CommandError{Code: agentpb.ErrorCode_ERROR_CODE_RESOURCE_EXHAUSTED, Message: "no hugepages"}
		}
		return nil
	})
	run, err := h.orch.Create(testContext(t), runs.StartRequest{ConfigID: "cfg-1", Agents: []string{a.ID(), b.ID()}})
	if err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)
	if end.State != runs.Failed || !strings.Contains(end.Reason, "no hugepages") {
		t.Fatalf("run ended %+v, want failed with the prepare error", end)
	}
	waitFor(t, "agent-a idle", func() bool {
		s, _ := a.State()
		return s == agentpb.AgentState_AGENT_STATE_IDLE
	})
	if s := stops(a); len(s) != 1 || s[0] != agentpb.StopMode_STOP_MODE_ABORT {
		t.Fatalf("agent-a stop commands %v, want one abort", s)
	}
	if s := stops(b); len(s) != 0 {
		t.Fatalf("agent-b got stop commands %v without a plan", s)
	}
}

func TestOperatorReservationPassesToRunAndBack(t *testing.T) {
	h := newHarness(t, nil)
	a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
	h.agent(t, "agent-c", nil)
	for _, id := range []string{"agent-a", "agent-b"} {
		if _, err := h.mgr.Reserve(id, agents.OperatorHolder); err != nil {
			t.Fatal(err)
		}
	}
	run, err := h.orch.Create(testContext(t), runs.StartRequest{ConfigID: "cfg-1"})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(run.Agents, ",") != "agent-a,agent-b" {
		t.Fatalf("run picked %v, want the operator reserved agents", run.Agents)
	}
	h.waitRunning(t, run.ID, a, b)
	for _, f := range []*fakeagent.Agent{a, b} {
		if err := f.Finish(); err != nil {
			t.Fatal(err)
		}
	}
	h.wait(t, run.ID)
	for _, id := range []string{"agent-a", "agent-b"} {
		if ag, _ := h.mgr.Get(id); ag.ReservedBy != agents.OperatorHolder {
			t.Errorf("%s reserved by %q after the run, want the operator", id, ag.ReservedBy)
		}
	}
}

func TestCreateErrors(t *testing.T) {
	h := newHarness(t, nil)
	h.agent(t, "agent-a", nil)
	busy := h.agent(t, "agent-b", nil)
	other := h.start(t, busy)

	tests := []struct {
		name string
		req  runs.StartRequest
		want error
	}{
		{"empty config id", runs.StartRequest{}, runs.ErrInvalidRequest},
		{"unknown config", runs.StartRequest{ConfigID: "nope", Agents: []string{"agent-a"}}, store.ErrConfigNotFound},
		{"broken config", runs.StartRequest{ConfigID: "broken", Agents: []string{"agent-a"}}, runs.ErrInvalidRequest},
		{"unknown scenario", runs.StartRequest{ConfigID: "no-mix", Agents: []string{"agent-a"}}, compiler.ErrUnknownScenario},
		{"no reserved agents", runs.StartRequest{ConfigID: "cfg-1"}, runs.ErrNoAgents},
		{"unknown agent", runs.StartRequest{ConfigID: "cfg-1", Agents: []string{"agent-x"}}, agents.ErrUnknownAgent},
		{"agent of another run", runs.StartRequest{ConfigID: "cfg-1", Agents: []string{"agent-a", "agent-b"}}, agents.ErrAgentReserved},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := h.orch.Create(testContext(t), tc.req); !errors.Is(err, tc.want) {
				t.Fatalf("Create = %v, want %v", err, tc.want)
			}
			if ag, _ := h.mgr.Get("agent-a"); ag.ReservedBy != "" {
				t.Fatalf("agent-a stays reserved by %q after a failed create", ag.ReservedBy)
			}
		})
	}
	if _, err := h.orch.Get("run-missing"); !errors.Is(err, runs.ErrRunNotFound) {
		t.Fatalf("Get = %v, want ErrRunNotFound", err)
	}
	if _, err := h.orch.Stop("run-missing", false); !errors.Is(err, runs.ErrRunNotFound) {
		t.Fatalf("Stop = %v, want ErrRunNotFound", err)
	}
	if _, err := h.orch.Wait(testContext(t), "run-missing"); !errors.Is(err, runs.ErrRunNotFound) {
		t.Fatalf("Wait = %v, want ErrRunNotFound", err)
	}
	if ag, _ := h.mgr.Get("agent-b"); ag.ReservedBy != other.ID {
		t.Fatalf("agent-b reserved by %q, want %s", ag.ReservedBy, other.ID)
	}
}

func TestCloseAbortsRuns(t *testing.T) {
	h := newHarness(t, nil)
	a := h.agent(t, "agent-a", nil)
	run := h.start(t, a)
	h.orch.Close()
	end, err := h.orch.Get(run.ID)
	if err != nil {
		t.Fatal(err)
	}
	if end.State != runs.Aborted || end.Reason != "controller shutdown" {
		t.Fatalf("run after Close %+v, want aborted by the shutdown", end)
	}
	if s := stops(a); len(s) != 1 || s[0] != agentpb.StopMode_STOP_MODE_ABORT {
		t.Fatalf("stop commands %v, want one abort", s)
	}
	if _, err := h.orch.Create(testContext(t), runs.StartRequest{ConfigID: "cfg-1", Agents: []string{"agent-a"}}); !errors.Is(err, runs.ErrClosed) {
		t.Fatalf("Create after Close = %v, want ErrClosed", err)
	}
}

func TestCustomEvaluatorDecides(t *testing.T) {
	var seen runs.Outcome
	h := newHarness(t, func(c *runs.Config) {
		c.Evaluator = func(o runs.Outcome) runs.Verdict {
			seen = o
			return runs.Verdict{Passed: false}
		}
	})
	a := h.agent(t, "agent-a", nil)
	run := h.start(t, a)
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	if end := h.wait(t, run.ID); end.State != runs.Failed || end.Reason != "criteria failed" {
		t.Fatalf("run ended %+v, want failed by the evaluator", end)
	}
	if seen.Agents != 1 || len(seen.Criteria) != 2 {
		t.Fatalf("evaluator saw %+v", seen)
	}
}

func TestNewRequiresStoreAndAgents(t *testing.T) {
	if _, err := runs.New(runs.Config{}); err == nil {
		t.Fatal("New without store and agents succeeded")
	}
}

// contractID is the id pattern of the OpenAPI contract.
var contractID = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)

// TestDefaultRunIDsCarryRunPrefix is the regression test of bug #128. Run
// ids used the cfg_ prefix of configuration ids.
func TestDefaultRunIDsCarryRunPrefix(t *testing.T) {
	seen := map[string]bool{}
	for range 100 {
		id, err := runs.RandomID()
		if err != nil {
			t.Fatal(err)
		}
		if !strings.HasPrefix(id, runs.RunIDPrefix) || len(id) != len(runs.RunIDPrefix)+24 || !contractID.MatchString(id) {
			t.Fatalf("RandomID returned %q, want run_ and 24 hex digits", id)
		}
		if seen[id] {
			t.Fatalf("RandomID repeated %q", id)
		}
		seen[id] = true
	}

	h := newHarness(t, func(c *runs.Config) { c.NewID = nil })
	a := h.agent(t, "agent-a", nil)
	run := h.start(t, a)
	if !strings.HasPrefix(run.ID, "run_") || strings.HasPrefix(run.ID, "cfg_") || !contractID.MatchString(run.ID) {
		t.Fatalf("created run id %q, want the run_ prefix", run.ID)
	}
	if _, err := h.orch.Stop(run.ID, true); err != nil {
		t.Fatal(err)
	}
	h.wait(t, run.ID)
}
