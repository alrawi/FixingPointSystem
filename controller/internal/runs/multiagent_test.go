// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/multiagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// startLead is the lead of the common start instant in these tests.
const startLead = 50 * time.Millisecond

// multiHarness is a harness whose orchestrator takes the clock offsets of the
// agents from their heartbeats, as the controller does (CTL-52).
func multiHarness(t *testing.T) *harness {
	t.Helper()
	return newHarness(t, func(c *runs.Config) {
		c.Clocks = multiagent.HeartbeatClocks{Agents: c.Agents}
		c.StartLead = startLead
	})
}

// skewed starts a fake agent whose clock runs skew ahead of the controller
// clock. The agent reads the clock of the agent manager, so the controller
// measures exactly skew, without the delay of the heartbeat, and the test
// waits until the first measurement exists.
func (h *harness) skewed(t *testing.T, id string, skew time.Duration) *fakeagent.Agent {
	t.Helper()
	a, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: id, Dial: h.dial, Clock: h.clock, ClockSkew: skew})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	waitFor(t, "the clock offset of "+id, func() bool {
		ag, err := h.mgr.Get(id)
		return err == nil && ag.HasClockOffset
	})
	return a
}

// startOf returns the start command that a fake agent received, or nil.
func startOf(a *fakeagent.Agent) *agentpb.StartCommand {
	for _, c := range a.Commands() {
		if s := c.GetStart(); s != nil {
			return s
		}
	}
	return nil
}

// counted sends one counter snapshot and waits until the controller took it.
func counted(t *testing.T, a *fakeagent.Agent, attempted, succeeded, failed, bytesTx uint64) {
	t.Helper()
	if _, err := a.Snapshot(count("sessions.attempted", attempted), count("sessions.succeeded", succeeded),
		count("sessions.failed", failed), count("bytes.tx", bytesTx)); err != nil {
		t.Fatal(err)
	}
	if err := a.Heartbeat(testContext(t)); err != nil {
		t.Fatal(err)
	}
}

// TestRESTMultiAgentRunRefusedForAnAgentOffBy15ms is the first half of the
// acceptance test of CTL-52. The configuration runs on three fake agents, and
// one of them runs 15 ms ahead, so POST /v1/runs answers 409 and names it.
func TestRESTMultiAgentRunRefusedForAnAgentOffBy15ms(t *testing.T) {
	ms := time.Millisecond
	h := multiHarness(t)
	a, b, c := h.skewed(t, "agent-a", 0), h.skewed(t, "agent-b", 15*ms), h.skewed(t, "agent-c", -3*ms)
	if _, err := h.mgr.Reserve("agent-a", agents.OperatorHolder); err != nil {
		t.Fatal(err)
	}
	rh := newRESTHarness(t, h.orch)

	rec := rh.call(t, http.MethodPost, "/runs", nil, `{"config_id":"cfg-1","agents":["agent-a","agent-b","agent-c"]}`, true)
	if rec.Code != http.StatusConflict {
		t.Fatalf("start with agent-b 15ms off: %d %s, want 409", rec.Code, rec.Body)
	}
	var p api.Problem
	if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil {
		t.Fatal(err)
	}
	if p.Detail == nil || !strings.Contains(*p.Detail, "agent-b 15ms") || strings.Contains(*p.Detail, "agent-a") || strings.Contains(*p.Detail, "agent-c") {
		t.Fatalf("problem detail %v, want it to name agent-b at 15ms only", p.Detail)
	}

	// The refusal left no run, no command, and no reservation behind, and
	// the reservation of the operator is back.
	var list struct {
		Items []api.Run `json:"items"`
	}
	if err := json.Unmarshal(rh.call(t, http.MethodGet, "/runs", nil, "", true).Body.Bytes(), &list); err != nil || len(list.Items) != 0 {
		t.Fatalf("runs after the refusal: %+v, %v", list.Items, err)
	}
	for _, f := range []*fakeagent.Agent{a, b, c} {
		if n := len(f.Commands()); n != 0 {
			t.Errorf("%s received %d commands", f.ID(), n)
		}
		ag, _ := h.mgr.Get(f.ID())
		want := ""
		if f == a {
			want = agents.OperatorHolder
		}
		if ag.ReservedBy != want {
			t.Errorf("%s reserved by %q after the refusal, want %q", f.ID(), ag.ReservedBy, want)
		}
	}

	// The two agents within the limit still run the configuration.
	rec = rh.call(t, http.MethodPost, "/runs", nil, `{"config_id":"cfg-1","agents":["agent-a","agent-c"]}`, true)
	if rec.Code != http.StatusAccepted {
		t.Fatalf("start without agent-b: %d %s", rec.Code, rec.Body)
	}
	run := decodeRun(t, rec)
	h.waitRunning(t, run.Id, a, c)
	if _, err := h.orch.Stop(run.Id, true); err != nil {
		t.Fatal(err)
	}
	h.wait(t, run.Id)
}

// TestRESTMultiAgentRunReportsExactTotals is the second half. Three fake
// agents within 10 ms run one configuration through the coordinator, and the
// summary of the run is the exact sum of their counters.
func TestRESTMultiAgentRunReportsExactTotals(t *testing.T) {
	ms := time.Millisecond
	h := multiHarness(t)
	skews := map[string]time.Duration{"agent-a": 0, "agent-b": 2 * ms, "agent-c": -2 * ms}
	a, b, c := h.skewed(t, "agent-a", skews["agent-a"]), h.skewed(t, "agent-b", skews["agent-b"]), h.skewed(t, "agent-c", skews["agent-c"])
	rh := newRESTHarness(t, h.orch)

	rec := rh.call(t, http.MethodPost, "/runs", nil, `{"config_id":"cfg-1","agents":["agent-a","agent-b","agent-c"]}`, true)
	if rec.Code != http.StatusAccepted {
		t.Fatalf("start: %d %s", rec.Code, rec.Body)
	}
	run := decodeRun(t, rec)
	h.waitRunning(t, run.Id, a, b, c)

	// Every agent got the same start instant, converted to its own clock.
	base := h.clock.Now().Add(startLead)
	for _, f := range []*fakeagent.Agent{a, b, c} {
		st := startOf(f)
		if st == nil || st.GetStartAt() == nil {
			t.Fatalf("%s got start command %v, want one with start_at", f.ID(), st)
		}
		if got, want := st.GetStartAt().AsTime(), base.Add(skews[f.ID()]); !got.Equal(want) {
			t.Errorf("%s start_at %s, want %s", f.ID(), got, want)
		}
	}

	// The sums pass 2^32, and the counters are cumulative, so the newest
	// snapshot of an agent replaces its earlier ones.
	counted(t, a, 4_000_000_001, 4_000_000_001, 0, 5_000_000_011)
	counted(t, b, 1, 1, 0, 1)
	counted(t, b, 3_000_000_002, 2_999_999_999, 3, 6_000_000_022)
	counted(t, c, 2_000_000_003, 2_000_000_003, 0, 7_000_000_033)

	id := map[string]string{"id": run.Id}
	if rec = rh.call(t, http.MethodPost, "/runs/{id}:stop", id, `{"force":false}`, true); rec.Code != http.StatusAccepted {
		t.Fatalf("stop: %d %s", rec.Code, rec.Body)
	}
	h.wait(t, run.Id)
	end := decodeRun(t, rh.call(t, http.MethodGet, "/runs/{id}", id, "", true))
	if end.State != api.RunStateCompleted || end.Verdict == nil || !*end.Verdict.Passed || end.Summary == nil {
		t.Fatalf("ended run %s", mustJSON(t, end))
	}
	s := end.Summary
	for _, tc := range []struct {
		name string
		got  *int
		want int64
	}{
		{"sessions_attempted", s.SessionsAttempted, 9_000_000_006},
		{"sessions_succeeded", s.SessionsSucceeded, 9_000_000_003},
		{"sessions_failed", s.SessionsFailed, 3},
		{"bytes_tx", s.BytesTx, 18_000_000_066},
	} {
		if tc.got == nil || int64(*tc.got) != tc.want {
			t.Errorf("summary %s = %v, want exactly %d", tc.name, tc.got, tc.want)
		}
	}
	for _, f := range []*fakeagent.Agent{a, b, c} {
		if m := stops(f); len(m) != 1 || m[0] != agentpb.StopMode_STOP_MODE_RAMP_DOWN {
			t.Errorf("%s stop commands %v, want one ramp down", f.ID(), m)
		}
		if ag, _ := h.mgr.Get(f.ID()); ag.ReservedBy != "" {
			t.Errorf("%s still reserved by %q", f.ID(), ag.ReservedBy)
		}
	}
}

// TestMultiAgentRunCountsTheOpenSessionsOfAFailedAgent holds the contract of
// fault containment for the coordinated path. The run keeps the sessions that
// a failed agent attempted and never finished as failed sessions.
func TestMultiAgentRunCountsTheOpenSessionsOfAFailedAgent(t *testing.T) {
	h := multiHarness(t)
	a, b := h.skewed(t, "agent-a", time.Millisecond), h.skewed(t, "agent-b", -time.Millisecond)
	run := h.start(t, a, b)
	counted(t, a, 100, 100, 0, 1000)
	// 40 succeeded and 10 failed, so 50 sessions stay open and end with the agent.
	counted(t, b, 100, 40, 10, 1000)
	if err := b.Fail("worker panic"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "a degraded run", func() bool {
		r, _ := h.orch.Get(run.ID)
		return r.Degraded
	})
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)
	if end.State != runs.Failed || !end.Degraded || len(end.FailedAgents) != 1 || end.FailedAgents[0] != "agent-b" {
		t.Fatalf("run ended %+v, want failed and degraded with agent-b failed", end)
	}
	want := map[string]float64{"sessions.attempted": 200, "sessions.succeeded": 140, "sessions.failed": 60, "bytes.tx": 2000, "sessions.failed_ratio": 0.3}
	for k, v := range want {
		if got := end.Totals[k]; got != v {
			t.Errorf("total %s = %v, want %v", k, got, v)
		}
	}
	var failed float64
	for _, s := range end.Series {
		if s.Metric == "sessions.failed" && len(s.Labels) == 0 {
			failed = s.Value
		}
	}
	if failed != 60 {
		t.Errorf("series sessions.failed = %v, want 60", failed)
	}
}

// TestSingleAgentRunIgnoresTheClockCheck keeps runs on one agent as they were:
// no clock check, and a start without a common instant.
func TestSingleAgentRunIgnoresTheClockCheck(t *testing.T) {
	h := multiHarness(t)
	a := h.skewed(t, "agent-a", 500*time.Millisecond)
	run := h.start(t, a)
	if st := startOf(a); st == nil || st.GetStartAt() != nil {
		t.Fatalf("start command %v, want one without start_at", st)
	}
	if _, err := h.orch.Stop(run.ID, true); err != nil {
		t.Fatal(err)
	}
	if end := h.wait(t, run.ID); end.State != runs.Aborted {
		t.Fatalf("run ended %s, want aborted", end.State)
	}
}

// TestRESTMultiAgentRunRefusedWithoutAClockMeasurement checks that a run does
// not start on a guess: an agent without a measured offset is a conflict too.
func TestRESTMultiAgentRunRefusedWithoutAClockMeasurement(t *testing.T) {
	h := newHarness(t, func(c *runs.Config) {
		c.Clocks = multiagent.ClockSourceFunc(func(_ context.Context, id string) (time.Duration, error) {
			return 0, fmt.Errorf("agent %s: %w", id, multiagent.ErrNoClockOffset)
		})
	})
	h.agent(t, "agent-a", nil)
	h.agent(t, "agent-b", nil)
	rh := newRESTHarness(t, h.orch)
	rec := rh.call(t, http.MethodPost, "/runs", nil, `{"config_id":"cfg-1","agents":["agent-a","agent-b"]}`, true)
	if rec.Code != http.StatusConflict || !strings.Contains(rec.Body.String(), "no clock offset measurement") {
		t.Fatalf("start without a measurement: %d %s, want 409", rec.Code, rec.Body)
	}
}
