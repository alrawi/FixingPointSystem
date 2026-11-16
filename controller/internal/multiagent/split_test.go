// SPDX-License-Identifier: GPL-3.0-only

package multiagent_test

import (
	"errors"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/multiagent"
)

func inventory(ids ...string) []compiler.Agent {
	out := make([]compiler.Agent, len(ids))
	for i, id := range ids {
		out[i] = compiler.Agent{ID: id, Cores: 2}
	}
	return out
}

func plan(agent string) compiler.Plan {
	return compiler.Plan{PlanID: "run/" + agent, RunID: "run", AgentID: agent, Seed: 9}
}

func compiled(ps ...compiler.Plan) multiagent.Compiler {
	return multiagent.CompilerFunc(func(compiler.Request) ([]compiler.Plan, error) { return ps, nil })
}

func TestSplitReturnsOnePlanPerAgentSortedByID(t *testing.T) {
	got, err := multiagent.Split(compiled(plan("c"), plan("a"), plan("b")), compiler.Request{Agents: inventory("a", "b", "c")})
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 3 || got[0].AgentID != "a" || got[1].AgentID != "b" || got[2].AgentID != "c" {
		t.Fatalf("plans %+v", got)
	}
}

func TestSplitRejectsAMismatch(t *testing.T) {
	other := plan("b")
	other.Seed = 10
	otherRun := plan("b")
	otherRun.RunID = "run2"
	noSeed := plan("b")
	noSeed.Seed = 0
	sameID := plan("b")
	sameID.PlanID = "run/a"
	tests := []struct {
		name string
		ps   []compiler.Plan
		ids  []string
	}{
		{"missing plan", []compiler.Plan{plan("a")}, []string{"a", "b"}},
		{"stranger", []compiler.Plan{plan("a"), plan("x")}, []string{"a"}},
		{"duplicate agent", []compiler.Plan{plan("a"), plan("a")}, []string{"a"}},
		{"duplicate plan id", []compiler.Plan{plan("a"), sameID}, []string{"a", "b"}},
		{"seed differs", []compiler.Plan{plan("a"), other}, []string{"a", "b"}},
		{"no seed", []compiler.Plan{plan("a"), noSeed}, []string{"a", "b"}},
		{"run differs", []compiler.Plan{plan("a"), otherRun}, []string{"a", "b"}},
		{"no plans", nil, []string{"a"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := multiagent.Split(compiled(tt.ps...), compiler.Request{Agents: inventory(tt.ids...)})
			if !errors.Is(err, multiagent.ErrSplit) {
				t.Fatalf("error %v, want ErrSplit", err)
			}
		})
	}
}

func TestSplitNeedsAgentsAndPassesCompilerErrors(t *testing.T) {
	if _, err := multiagent.Split(compiled(), compiler.Request{}); !errors.Is(err, multiagent.ErrSplit) {
		t.Fatalf("error %v, want ErrSplit", err)
	}
	boom := errors.New("boom")
	failing := multiagent.CompilerFunc(func(compiler.Request) ([]compiler.Plan, error) { return nil, boom })
	if _, err := multiagent.Split(failing, compiler.Request{Agents: inventory("a")}); !errors.Is(err, boom) {
		t.Fatalf("error %v, want the compiler error", err)
	}
}
