// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"context"
	"log/slog"
	"strings"
	"sync"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// scenario returns the labels of a scenario counter of the http1 web_get test.
func scenario(side string, extra ...string) map[string]string {
	l := map[string]string{"application": "http1", "scenario": "web_get", "side": side}
	for i := 0; i+1 < len(extra); i += 2 {
		l[extra[i]] = extra[i+1]
	}
	return l
}

func labeled(metric string, n uint64, labels map[string]string) *agentpb.CounterValue {
	return &agentpb.CounterValue{Metric: metric, Labels: labels, Value: &agentpb.CounterValue_Count{Count: n}}
}

// snapshot sends one snapshot and waits until the controller processed it.
func snapshot(t *testing.T, a *fakeagent.Agent, counters ...*agentpb.CounterValue) {
	t.Helper()
	if _, err := a.Snapshot(counters...); err != nil {
		t.Fatal(err)
	}
	if err := a.Heartbeat(testContext(t)); err != nil {
		t.Fatal(err)
	}
}

// bothSides returns the counters of a session mix that the client and the
// server side both report, so that a total over both sides counts each
// session twice.
func bothSides(attempted, succeeded, failed uint64) []*agentpb.CounterValue {
	var out []*agentpb.CounterValue
	for _, side := range []string{"client", "server"} {
		out = append(out,
			labeled("sessions.attempted", attempted, scenario(side)),
			labeled("sessions.succeeded", succeeded, scenario(side)),
			labeled("bytes.tx", 1000, scenario(side)),
		)
		if failed > 0 {
			out = append(out, labeled("sessions.failed", failed, scenario(side, "reason", "connect_timeout")))
		}
	}
	return out
}

// TestLabeledCountersReportTheClientSide checks that the run totals and the
// criteria count the client side only, that the outcome carries the run id and
// the labeled counters, and that the open sessions of a failed agent count as
// failed sessions of the reason aborted.
func TestLabeledCountersReportTheClientSide(t *testing.T) {
	var seen runs.Outcome
	h := newHarness(t, func(c *runs.Config) {
		c.Evaluator = func(o runs.Outcome) runs.Verdict {
			seen = o
			return runs.EvaluateTotals(o)
		}
	})
	a, b := h.agent(t, "agent-a", nil), h.agent(t, "agent-b", nil)
	run := h.start(t, a, b)
	snapshot(t, a, bothSides(100, 100, 0)...)
	// Agent b attempted 100 sessions, of which 40 succeeded and 10 failed.
	// The other 50 end with the agent.
	snapshot(t, b, bothSides(100, 40, 10)...)
	b.Crash()
	waitFor(t, "a degraded run", func() bool {
		r, _ := h.orch.Get(run.ID)
		return r.Degraded
	})
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)

	if seen.RunID != run.ID {
		t.Errorf("outcome run id %q, want %q", seen.RunID, run.ID)
	}
	// The criterion on the failure ratio counts 60 of 200 client sessions.
	if end.State != runs.Failed || end.Totals["sessions.attempted"] != 200 || end.Totals["sessions.failed"] != 60 || end.Totals["sessions.failed_ratio"] != 0.3 {
		t.Fatalf("run ended %s with totals %v", end.State, end.Totals)
	}
	if !strings.HasPrefix(end.Reason, "criteria failed: sessions.failed_ratio: observed 0.3 < 0.1 does not hold") {
		t.Errorf("reason %q", end.Reason)
	}
	got := map[string]float64{}
	for _, s := range end.Series {
		got[s.Metric+" "+s.Labels["side"]+" "+s.Labels["reason"]] += s.Value
	}
	for key, want := range map[string]float64{
		"sessions.attempted client ":             200,
		"sessions.attempted server ":             200,
		"sessions.failed client connect_timeout": 10,
		"sessions.failed server connect_timeout": 10,
		"sessions.failed client aborted":         50,
		"sessions.failed server aborted":         50,
		"bytes.tx client ":                       2000,
	} {
		if got[key] != want {
			t.Errorf("series %q = %v, want %v", key, got[key], want)
		}
	}
}

// messageHandler is a slog.Handler that closes seen when it logs message.
type messageHandler struct {
	message string
	seen    chan struct{}
	once    sync.Once
}

func (*messageHandler) Enabled(context.Context, slog.Level) bool { return true }

func (m *messageHandler) Handle(_ context.Context, r slog.Record) error {
	if r.Message == m.message {
		m.once.Do(func() { close(m.seen) })
	}
	return nil
}

func (m *messageHandler) WithAttrs([]slog.Attr) slog.Handler { return m }
func (m *messageHandler) WithGroup(string) slog.Handler      { return m }

// TestEveryFailedRunHasAReason checks the reason of each way a run fails or
// aborts.
func TestEveryFailedRunHasAReason(t *testing.T) {
	t.Run("every agent failed", func(t *testing.T) {
		// Crash the agent only after the orchestrator has seen the answer to
		// its start command; a crash before that ends the run with a start
		// reason instead (see "forced stop" below).
		started := make(chan struct{})
		h := newHarness(t, func(c *runs.Config) {
			c.Logger = slog.New(&messageHandler{message: "run started on every agent", seen: started})
		})
		a := h.agent(t, "agent-a", nil)
		run := h.start(t, a)
		select {
		case <-started:
		case <-testContext(t).Done():
			t.Fatal("the orchestrator did not finish starting the agents")
		}
		a.Crash()
		end := h.wait(t, run.ID)
		if end.State != runs.Failed || end.Reason != "every agent failed" {
			t.Fatalf("run ended %s with reason %q", end.State, end.Reason)
		}
	})
	t.Run("forced stop", func(t *testing.T) {
		// The agent runs the plan before the orchestrator has seen the answer
		// to its start command, and a forced stop in between aborts the run
		// as "stopped while starting". Wait for the orchestrator itself.
		started := make(chan struct{})
		h := newHarness(t, func(c *runs.Config) {
			c.Logger = slog.New(&messageHandler{message: "run started on every agent", seen: started})
		})
		a := h.agent(t, "agent-a", nil)
		run := h.start(t, a)
		select {
		case <-started:
		case <-testContext(t).Done():
			t.Fatal("the orchestrator did not finish starting the agents")
		}
		if _, err := h.orch.Stop(run.ID, true); err != nil {
			t.Fatal(err)
		}
		end := h.wait(t, run.ID)
		if end.State != runs.Aborted || end.Reason != "stopped by request" {
			t.Fatalf("run ended %s with reason %q", end.State, end.Reason)
		}
	})
	t.Run("an evaluator that fails the run without a criterion", func(t *testing.T) {
		h := newHarness(t, func(c *runs.Config) {
			c.Evaluator = func(runs.Outcome) runs.Verdict { return runs.Verdict{} }
		})
		a := h.agent(t, "agent-a", nil)
		run := h.start(t, a)
		if err := a.Finish(); err != nil {
			t.Fatal(err)
		}
		if end := h.wait(t, run.ID); end.State != runs.Failed || end.Reason != "criteria failed" {
			t.Fatalf("run ended %s with reason %q", end.State, end.Reason)
		}
	})
	t.Run("a completed run has none", func(t *testing.T) {
		h := newHarness(t, nil)
		a := h.agent(t, "agent-a", nil)
		run := h.start(t, a)
		if err := a.Finish(); err != nil {
			t.Fatal(err)
		}
		if end := h.wait(t, run.ID); end.State != runs.Completed || end.Reason != "" {
			t.Fatalf("run ended %s with reason %q", end.State, end.Reason)
		}
	})
}

// TestCriteriaNamedInTheReason checks that the reason names at most three
// failed criteria and counts the rest.
func TestCriteriaNamedInTheReason(t *testing.T) {
	h := newHarness(t, func(c *runs.Config) {
		c.Evaluator = func(runs.Outcome) runs.Verdict {
			v := runs.Verdict{}
			for i := range 5 {
				v.Criteria = append(v.Criteria, runs.CriterionResult{
					Criterion: runs.Criterion{CriterionConfig: compiler.CriterionConfig{Metric: "bytes.tx"}},
					Evaluated: true, Reason: "reason " + string(rune('a'+i)),
				})
			}
			return v
		}
	})
	a := h.agent(t, "agent-a", nil)
	run := h.start(t, a)
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	want := "criteria failed: bytes.tx: reason a; bytes.tx: reason b; bytes.tx: reason c; and 2 more"
	if end := h.wait(t, run.ID); end.Reason != want {
		t.Fatalf("reason %q, want %q", end.Reason, want)
	}
}

func TestToAPIReason(t *testing.T) {
	for state, want := range map[runs.State]bool{runs.Pending: false, runs.Running: false, runs.Completed: false, runs.Failed: true, runs.Aborted: true} {
		got := runs.ToAPI(runs.Run{ID: "r", ConfigID: "c", State: state, Reason: "why"})
		if (got.Reason != nil) != want || (want && *got.Reason != "why") {
			t.Errorf("state %s: reason %v, want shown %v", state, got.Reason, want)
		}
	}
}

func TestToAPICriterionFields(t *testing.T) {
	crit := runs.Criterion{CriterionConfig: compiler.CriterionConfig{Metric: "bytes.tx", Op: "<", Value: 1}}
	run := runs.Run{ID: "r", ConfigID: "c", State: runs.Failed, Verdict: &runs.Verdict{Criteria: []runs.CriterionResult{
		{Criterion: crit, Evaluated: true, Observed: 5, Reason: "observed 5 < 1 does not hold"},
		{Criterion: crit, Invalid: true, Reason: "metric bytes.tx has no label port"},
		{Criterion: crit, Reason: "the run reported no value"},
	}}}
	cs := *runs.ToAPI(run).Verdict.Criteria
	if cs[0].Observed == nil || *cs[0].Observed != 5 || cs[0].Passed == nil || *cs[0].Passed || *cs[0].Reason != "observed 5 < 1 does not hold" {
		t.Errorf("evaluated criterion %+v", cs[0])
	}
	if cs[1].Observed != nil || cs[1].Passed == nil || *cs[1].Passed || *cs[1].Reason != "metric bytes.tx has no label port" {
		t.Errorf("invalid criterion %+v", cs[1])
	}
	if cs[2].Observed != nil || cs[2].Passed != nil || *cs[2].Reason != "the run reported no value" {
		t.Errorf("missing criterion %+v", cs[2])
	}
}

// TestEvaluateTotalsSkipsLabelFilters checks that the default evaluator, which
// has no labels, leaves a criterion with a where filter without a result.
func TestEvaluateTotalsSkipsLabelFilters(t *testing.T) {
	c := runs.Criterion{CriterionConfig: compiler.CriterionConfig{Metric: "bytes.tx", Op: ">", Value: 0}, Where: map[string]string{"application": "dns"}}
	v := runs.EvaluateTotals(runs.Outcome{Agents: 1, Criteria: []runs.Criterion{c}, Totals: map[string]float64{"bytes.tx": 5}})
	if !v.Passed || v.Criteria[0].Evaluated || v.Criteria[0].Reason == "" {
		t.Fatalf("verdict %+v", v)
	}
}
