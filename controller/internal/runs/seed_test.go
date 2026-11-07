// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"errors"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// noTimelineSeed is the test configuration without timeline.seed.
var noTimelineSeed = strings.Replace(testConfig, "timeline: { seed: 7 }\n", "", 1)

// TestRunSeedComesFromThePlans checks that the run record carries the seed of
// the compiled plans, which is never 0: the seed of the configuration first,
// then the one of the orchestrator, and else a random one that the compiler
// draws.
func TestRunSeedComesFromThePlans(t *testing.T) {
	cases := []struct {
		name   string
		config string
		seed   func() uint64
		want   uint64 // 0 means any nonzero seed
	}{
		{"timeline seed", testConfig, func() uint64 { return 1 }, 7},
		{"orchestrator seed", noTimelineSeed, func() uint64 { return 42 }, 42},
		{"seed drawn by the compiler", noTimelineSeed, nil, 0},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			h := newHarness(t, func(cfg *runs.Config) {
				cfg.Store = memStore{"cfg-1": c.config}
				cfg.Seed = c.seed
			})
			a := h.agent(t, "agent-a", nil)
			run := h.start(t, a)
			if run.Seed == 0 || (c.want != 0 && run.Seed != c.want) {
				t.Fatalf("created run has seed %d, want %d (0 means any nonzero seed)", run.Seed, c.want)
			}
			if err := a.Finish(); err != nil {
				t.Fatal(err)
			}
			if end := h.wait(t, run.ID); end.Seed != run.Seed {
				t.Fatalf("ended run has seed %d, want the seed %d of its plans", end.Seed, run.Seed)
			}
		})
	}
}

// TestPlansWithoutSeedAreRejected checks that a compiler that leaves the seed
// out cannot give a run the seed 0.
func TestPlansWithoutSeedAreRejected(t *testing.T) {
	h := newHarness(t, func(cfg *runs.Config) {
		cfg.Compiler = runs.CompilerFunc(func(req compiler.Request) ([]compiler.Plan, error) {
			plans, err := compiler.Compile(req)
			for i := range plans {
				plans[i].Seed = 0
			}
			return plans, err
		})
	})
	a := h.agent(t, "agent-a", nil)
	_, err := h.orch.Create(testContext(t), runs.StartRequest{ConfigID: "cfg-1", Agents: []string{a.ID()}})
	if err == nil || errors.Is(err, runs.ErrInvalidRequest) || !strings.Contains(err.Error(), "without a seed") {
		t.Fatalf("Create returned %v, want the error of a plan without a seed", err)
	}
}
