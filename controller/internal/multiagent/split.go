// SPDX-License-Identifier: GPL-3.0-only

package multiagent

import (
	"errors"
	"fmt"
	"slices"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
)

// ErrSplit reports that the compiler did not return exactly one plan for
// each agent of the run.
var ErrSplit = errors.New("plan split does not match the agents")

// Compiler compiles a run into one plan per agent. compiler.Compile
// satisfies it through CompilerFunc.
type Compiler interface {
	Compile(req compiler.Request) ([]compiler.Plan, error)
}

// CompilerFunc adapts a function such as compiler.Compile to Compiler.
type CompilerFunc func(req compiler.Request) ([]compiler.Plan, error)

// Compile calls f.
func (f CompilerFunc) Compile(req compiler.Request) ([]compiler.Plan, error) { return f(req) }

// Split splits the test of req across req.Agents and returns one plan for each
// agent, ordered by agent id. The compiler divides every host pool and every
// objective by the cores of the agents (CTL-05 and CTL-18), and Split checks
// what a multi agent run relies on: that every agent has exactly one plan,
// that no plan names a stranger, that plan ids differ, and that every plan
// carries the same nonzero seed and run id. A violation fails with ErrSplit.
func Split(c Compiler, req compiler.Request) ([]compiler.Plan, error) {
	if len(req.Agents) == 0 {
		return nil, fmt.Errorf("%w: the request has no agents", ErrSplit)
	}
	plans, err := c.Compile(req)
	if err != nil {
		return nil, err
	}
	want := map[string]bool{}
	for _, a := range req.Agents {
		want[a.ID] = true
	}
	got := map[string]bool{}
	ids := map[string]bool{}
	for _, p := range plans {
		switch {
		case !want[p.AgentID]:
			return nil, fmt.Errorf("%w: plan %s names agent %q, which is not in the run", ErrSplit, p.PlanID, p.AgentID)
		case got[p.AgentID]:
			return nil, fmt.Errorf("%w: agent %s has more than one plan", ErrSplit, p.AgentID)
		case ids[p.PlanID]:
			return nil, fmt.Errorf("%w: plan id %s is used twice", ErrSplit, p.PlanID)
		case p.Seed == 0:
			return nil, fmt.Errorf("%w: plan %s has no seed", ErrSplit, p.PlanID)
		case p.Seed != plans[0].Seed:
			return nil, fmt.Errorf("%w: plan %s has seed %d, plan %s has %d", ErrSplit, p.PlanID, p.Seed, plans[0].PlanID, plans[0].Seed)
		case p.RunID != plans[0].RunID:
			return nil, fmt.Errorf("%w: plan %s belongs to run %q, plan %s to %q", ErrSplit, p.PlanID, p.RunID, plans[0].PlanID, plans[0].RunID)
		}
		got[p.AgentID], ids[p.PlanID] = true, true
	}
	var missing []string
	for id := range want {
		if !got[id] {
			missing = append(missing, id)
		}
	}
	if len(missing) > 0 {
		slices.Sort(missing)
		return nil, fmt.Errorf("%w: no plan for agent %s", ErrSplit, strings.Join(missing, ", "))
	}
	slices.SortFunc(plans, func(a, b compiler.Plan) int { return strings.Compare(a.AgentID, b.AgentID) })
	return plans, nil
}
