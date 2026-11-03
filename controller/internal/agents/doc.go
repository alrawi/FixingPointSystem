// SPDX-License-Identifier: GPL-3.0-only

// Package agents is the agent manager of the controller. It serves the
// controller side of ControllerService from
// contracts/proto/fps/agent/v1/agent.proto, keeps the agent inventory, and
// serves the /v1/agents operations of the REST contract.
//
// Agents dial the controller. An agent calls Register, which checks its
// identity, enrollment, and engine contract version and returns a session
// token, and then holds one Session stream open. The stream carries
// heartbeats and their acknowledgements, commands with their results, and
// counter snapshots. Manager.Sweep marks an agent offline when no heartbeat
// arrives within the heartbeat timeout and ends its stream, and Manager.Run
// calls Sweep once per heartbeat interval.
//
// The inventory gives each agent one of the four REST states. An agent is
// offline without an open session, busy while it holds a plan, reserved
// while a holder such as a run reserves it, and online otherwise. Reserve
// and Release change the reservation, and Prepare, Start, and Stop send
// commands down the stream and wait for their results.
//
// The package reads time only from Config.Clock, so tests drive heartbeat
// timeouts with a fake clock. The package fakeagent provides agents that dial
// a Manager in tests.
package agents
