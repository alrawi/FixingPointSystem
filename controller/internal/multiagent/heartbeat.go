// SPDX-License-Identifier: GPL-3.0-only

package multiagent

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
)

// AgentLookup reads one inventory entry. *agents.Manager implements it.
type AgentLookup interface {
	Get(id string) (agents.Agent, error)
}

// HeartbeatClocks is the production ClockSource. It reports the clock offset
// that the agent manager measures from the agent_time of the heartbeats of
// each agent: agent_time minus the controller time at which the heartbeat
// arrived, taken from the latest heartbeat of the open session.
//
// The estimate includes the one way delay of the heartbeat, which makes it
// smaller than the true offset by that delay: an agent with a true offset of
// zero reports about minus the delay, and an agent that runs ahead by 12 ms
// over a link with 3 ms of delay reports 9 ms. On the lab network of a multi
// agent run the delay is well below the 10 ms limit, so the estimate can
// accept an agent that is off by slightly more than the limit. A caller that
// needs a tighter bound lowers Config.MaxClockOffset. The source does not
// correct for the round trip, although HeartbeatAck.controller_time would
// allow a client side correction.
//
// An agent without an open session, an unknown agent, and an agent whose open
// session has sent no heartbeat with an agent_time have no measurement, and
// ClockOffset fails with an error that wraps ErrNoClockOffset, so a run does
// not start on a stale or guessed value.
type HeartbeatClocks struct {
	// Agents is the agent manager.
	Agents AgentLookup
}

var _ ClockSource = HeartbeatClocks{}

// ClockOffset returns the agent clock minus the controller clock for id.
func (h HeartbeatClocks) ClockOffset(_ context.Context, id string) (time.Duration, error) {
	a, err := h.Agents.Get(id)
	switch {
	case errors.Is(err, agents.ErrUnknownAgent):
		return 0, fmt.Errorf("agent %s: %w", id, ErrNoClockOffset)
	case err != nil:
		return 0, fmt.Errorf("agent %s: %w", id, err)
	case a.State == agents.StateOffline:
		return 0, fmt.Errorf("agent %s is offline: %w", id, ErrNoClockOffset)
	case !a.HasClockOffset:
		return 0, fmt.Errorf("agent %s has sent no agent_time: %w", id, ErrNoClockOffset)
	}
	return a.ClockOffset, nil
}
