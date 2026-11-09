// SPDX-License-Identifier: GPL-3.0-only

package agents

import (
	"context"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// UpdateRates sends an update rates command to agent id and waits for its
// result. The agent applies the command completely or not at all (CON-05), and
// only in AGENT_STATE_RUNNING. The caller sends it only to an agent whose
// engine_api is 0.2.0 or later. A failure of the agent comes back as a
// *CommandError.
//
// Manager implements runs.RateUpdater with this method, which switches the
// throughput calibration of the run orchestrator on.
func (m *Manager) UpdateRates(ctx context.Context, id string, cmd *agentpb.UpdateRatesCommand) (*agentpb.UpdateRatesResult, error) {
	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_UpdateRates{UpdateRates: cmd}}, false)
	if err != nil {
		return nil, err
	}
	return res.GetUpdateRates(), nil
}
