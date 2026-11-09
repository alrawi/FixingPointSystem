// SPDX-License-Identifier: GPL-3.0-only

package agents

import (
	"errors"
	"fmt"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// ErrPlanUnsupported reports a plan that the agent would apply differently
// from what the plan says, because its engine_api is older than the plan
// needs. The manager refuses to send it, instead of letting the agent skip
// the part it does not know.
var ErrPlanUnsupported = errors.New("agent engine api is too old for the plan")

// labelsEngine is the first engine_api version that applies Criterion.labels
// (CON-15). An agent of 0.2.2 or earlier skips the field, counts every series
// of the metric, and gives another verdict.
var labelsEngine = [3]int{0, 2, 3}

// requirePlanEngine fails with ErrPlanUnsupported when plan holds a criterion
// with labels and agent id reports an engine_api below 0.2.3. The run
// compiler refuses such a plan when it knows the versions, so this check
// catches an agent whose version changed since, or an inventory without
// versions. An unknown agent passes, so that the command reports the usual
// error.
func (m *Manager) requirePlanEngine(id string, plan *agentpb.Plan) error {
	for i, c := range plan.GetCriteria() {
		if len(c.GetLabels()) == 0 {
			continue
		}
		m.mu.Lock()
		r := m.agents[id]
		var engine string
		known := r != nil && r.caps != nil
		if known {
			engine = r.caps.GetEngineApi()
		}
		m.mu.Unlock()
		if known && !engineAtLeast(engine, labelsEngine) {
			return fmt.Errorf("prepare plan %s on agent %s: criteria[%d] (%s) has labels, which need engine_api %d.%d.%d or later, and the agent reports %q: %w",
				plan.GetPlanId(), id, i, c.GetMetric(), labelsEngine[0], labelsEngine[1], labelsEngine[2], engine, ErrPlanUnsupported)
		}
		return nil
	}
	return nil
}
