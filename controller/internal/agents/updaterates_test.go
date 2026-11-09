// SPDX-License-Identifier: GPL-3.0-only

package agents_test

import (
	"errors"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

func sessionsPerSecond(entry string, v float64) *agentpb.RateUpdate {
	return &agentpb.RateUpdate{MixEntry: entry, Target: &agentpb.RateUpdate_SessionsPerSecond{SessionsPerSecond: v}}
}

// TestUpdateRates checks that the manager sends UpdateRates and returns its
// result, that the fake agent runs the new rates, that it applies a command
// completely or not at all, and that it takes the command in the running
// state only.
func TestUpdateRates(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	a := h.start(t, fakeagent.Options{ID: "agent-a"})
	prepare := &agentpb.PrepareCommand{Plan: &agentpb.Plan{
		PlanId: "p1", RunId: "run-1", AgentId: "agent-a", EngineApi: "0.2.0",
		Mix: []*agentpb.MixEntry{{Id: "web", Scenario: "http1/web_get", Weight: 1}, {Id: "dns", Scenario: "dns/lookup", Weight: 1}},
		Workers: []*agentpb.WorkerPlan{
			{Rates: []*agentpb.WorkerRate{{MixEntry: "web", Target: &agentpb.WorkerRate_SessionsPerSecond{SessionsPerSecond: 30}}}},
			{Rates: []*agentpb.WorkerRate{{MixEntry: "web", Target: &agentpb.WorkerRate_SessionsPerSecond{SessionsPerSecond: 70}}}},
		},
	}}
	var ce *agents.CommandError

	if _, err := h.mgr.UpdateRates(ctx, "nobody", &agentpb.UpdateRatesCommand{PlanId: "p1"}); !errors.Is(err, agents.ErrUnknownAgent) {
		t.Fatalf("UpdateRates to an unknown agent returned %v", err)
	}
	if _, err := h.mgr.Prepare(ctx, "agent-a", prepare); err != nil {
		t.Fatal(err)
	}
	if got := a.Rates()["web"]; got != 100 {
		t.Fatalf("prepared rate %v, want the sum 100 of the workers", got)
	}
	cmd := &agentpb.UpdateRatesCommand{PlanId: "p1", Rates: []*agentpb.RateUpdate{sessionsPerSecond("web", 25)}}
	if _, err := h.mgr.UpdateRates(ctx, "agent-a", cmd); !errors.As(err, &ce) || ce.Code != agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION {
		t.Fatalf("UpdateRates in PREPARED returned %v, want FAILED_PRECONDITION", err)
	}
	if _, err := h.mgr.Start(ctx, "agent-a", &agentpb.StartCommand{PlanId: "p1"}); err != nil {
		t.Fatal(err)
	}

	res, err := h.mgr.UpdateRates(ctx, "agent-a", cmd)
	if err != nil || res.GetAppliedAt() == nil {
		t.Fatalf("UpdateRates returned %v, %v", res, err)
	}
	if got := a.Rates(); got["web"] != 25 || got["dns"] != 0 {
		t.Fatalf("rates after the update %v, want web 25 and dns 0", got)
	}

	// An entry that is not in the plan fails the whole command.
	bad := &agentpb.UpdateRatesCommand{PlanId: "p1", Rates: []*agentpb.RateUpdate{sessionsPerSecond("web", 5), sessionsPerSecond("ghost", 1)}}
	if _, err := h.mgr.UpdateRates(ctx, "agent-a", bad); !errors.As(err, &ce) || ce.Code != agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT {
		t.Fatalf("UpdateRates with an unknown mix entry returned %v, want INVALID_ARGUMENT", err)
	}
	if got := a.Rates()["web"]; got != 25 {
		t.Fatalf("rate %v after a failed command, want the old 25", got)
	}
	other := &agentpb.UpdateRatesCommand{PlanId: "other", Rates: []*agentpb.RateUpdate{sessionsPerSecond("web", 5)}}
	if _, err := h.mgr.UpdateRates(ctx, "agent-a", other); !errors.As(err, &ce) || ce.Code != agentpb.ErrorCode_ERROR_CODE_NOT_FOUND {
		t.Fatalf("UpdateRates for another plan returned %v, want NOT_FOUND", err)
	}
	if got := a.Rates()["web"]; got != 25 {
		t.Fatalf("rate %v after a command for another plan, want 25", got)
	}
}
