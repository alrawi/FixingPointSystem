// SPDX-License-Identifier: GPL-3.0-only

package runs

import (
	"errors"
	"fmt"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// State is the state of a run, as the RunState schema of the REST contract
// names it.
type State = api.RunState

// The seven run states.
const (
	Pending   = api.RunStatePending
	Preparing = api.RunStatePreparing
	Running   = api.RunStateRunning
	Stopping  = api.RunStateStopping
	Completed = api.RunStateCompleted
	Failed    = api.RunStateFailed
	Aborted   = api.RunStateAborted
)

// States lists every run state in the order of the contract.
var States = []State{Pending, Preparing, Running, Stopping, Completed, Failed, Aborted}

// ErrInvalidTransition reports a state change that the run state machine
// does not allow.
var ErrInvalidTransition = errors.New("invalid run state transition")

// transitions holds the allowed successors of every state. A run moves from
// pending to preparing, running, and stopping, then ends as completed,
// failed, or aborted. Every state before the end may fail or abort, and only
// stopping may complete, because the controller collects the final counters
// and evaluates the criteria while it stops.
var transitions = map[State][]State{
	Pending:   {Preparing, Failed, Aborted},
	Preparing: {Running, Failed, Aborted},
	Running:   {Stopping, Failed, Aborted},
	Stopping:  {Completed, Failed, Aborted},
	Completed: nil,
	Failed:    nil,
	Aborted:   nil,
}

// Final reports whether s is one of the three end states.
func Final(s State) bool {
	return s == Completed || s == Failed || s == Aborted
}

// CanTransition reports whether a run may move from state from to state to.
func CanTransition(from, to State) bool {
	for _, s := range transitions[from] {
		if s == to {
			return true
		}
	}
	return false
}

// Transition returns to when the state machine allows the move from from,
// and an error that wraps ErrInvalidTransition otherwise.
func Transition(from, to State) (State, error) {
	if !CanTransition(from, to) {
		return from, fmt.Errorf("run state %s to %s: %w", from, to, ErrInvalidTransition)
	}
	return to, nil
}
