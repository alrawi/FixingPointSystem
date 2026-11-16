// SPDX-License-Identifier: GPL-3.0-only

// Package multiagent runs one test on several agents at once. A Coordinator
// refuses to start when the clock of an agent differs from the controller
// clock by more than MaxClockOffset, splits the test into one plan per agent
// with the compiler, prepares every agent, and starts all of them at one
// instant that it converts to the clock of each agent. A Collector sums the
// counter snapshots of the agents exactly, in unsigned 64 bit integers.
//
// The package reaches the agent manager, the compiler, and the clock
// measurements through interfaces. *agents.Manager satisfies AgentManager.
// ClockSource has no implementation in the controller yet: the agent manager
// does not keep Heartbeat.agent_time, which the contract provides for exactly
// this estimate, so the controller wiring that adds one belongs to a package
// that owns internal/agents.
//
//	c, _ := multiagent.New(multiagent.Config{Agents: mgr, Clocks: clocks})
//	run, err := c.Start(ctx, multiagent.Request{RunID: id, Config: cfg, Agents: ids})
//	// feed every agents.Event to run.Observe
//	res, err := run.Stop(ctx, agentpb.StopMode_STOP_MODE_RAMP_DOWN)
package multiagent
