// SPDX-License-Identifier: GPL-3.0-only

package agents_test

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

// TestPrepareRefusesLabeledCriteriaForAnOldAgent checks CON-15: an agent below
// engine_api 0.2.3 skips Criterion.labels, so the manager does not send it a
// plan that has one, and it sends every other plan.
func TestPrepareRefusesLabeledCriteriaForAnOldAgent(t *testing.T) {
	r := newCaptureRig(t, 0)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	start := func(id, engine string) *fakeagent.Agent {
		caps := fakeagent.DefaultCapabilities()
		caps.EngineApi = engine
		a, err := fakeagent.Start(ctx, fakeagent.Options{ID: id, Dial: r.dial, Capabilities: caps})
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(a.Close)
		return a
	}
	old, current := start("agent-old", "0.2.2"), start("agent-new", "0.2.3")
	plan := func(id string, labels map[string]string) *agentpb.PrepareCommand {
		return &agentpb.PrepareCommand{Plan: &agentpb.Plan{
			PlanId: "p/" + id, RunId: "p", AgentId: id, EngineApi: "0.2.0",
			Criteria: []*agentpb.Criterion{{Metric: "sessions.failed", Op: agentpb.CriterionOp_CRITERION_OP_LE, Labels: labels}},
		}}
	}
	count := func(a *fakeagent.Agent) int {
		n := 0
		for _, c := range a.Commands() {
			if c.GetPrepare() != nil {
				n++
			}
		}
		return n
	}

	_, err := r.mgr.Prepare(ctx, "agent-old", plan("agent-old", map[string]string{"side": "server"}))
	if !errors.Is(err, agents.ErrPlanUnsupported) {
		t.Fatalf("labeled plan for engine api 0.2.2: error %v, want ErrPlanUnsupported", err)
	}
	if n := count(old); n != 0 {
		t.Fatalf("the controller sent %d prepare commands to the old agent", n)
	}
	if _, err := r.mgr.Prepare(ctx, "agent-new", plan("agent-new", map[string]string{"side": "server"})); err != nil {
		t.Fatalf("labeled plan for engine api 0.2.3: %v", err)
	}
	if n := count(current); n != 1 {
		t.Fatalf("the agent of 0.2.3 got %d prepare commands, want 1", n)
	}
	// A plan without labels runs on the old agent as before.
	if _, err := r.mgr.Prepare(ctx, "agent-old", plan("agent-old", nil)); err != nil {
		t.Fatalf("plan without labels for engine api 0.2.2: %v", err)
	}
}
