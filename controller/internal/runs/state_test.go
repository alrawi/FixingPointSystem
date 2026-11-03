// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"errors"
	"fmt"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// allowed lists the transitions of the run state machine in the
// specification. Every pair of states that the table leaves out is rejected.
var allowed = map[[2]runs.State]bool{
	{runs.Pending, runs.Preparing}:  true,
	{runs.Pending, runs.Failed}:     true,
	{runs.Pending, runs.Aborted}:    true,
	{runs.Preparing, runs.Running}:  true,
	{runs.Preparing, runs.Failed}:   true,
	{runs.Preparing, runs.Aborted}:  true,
	{runs.Running, runs.Stopping}:   true,
	{runs.Running, runs.Failed}:     true,
	{runs.Running, runs.Aborted}:    true,
	{runs.Stopping, runs.Completed}: true,
	{runs.Stopping, runs.Failed}:    true,
	{runs.Stopping, runs.Aborted}:   true,
}

func TestTransitionsCoverEveryPair(t *testing.T) {
	if len(runs.States) != 7 {
		t.Fatalf("runs.States has %d states, want the 7 of the contract", len(runs.States))
	}
	type tc struct {
		from, to runs.State
		ok       bool
	}
	var tests []tc
	for _, from := range runs.States {
		for _, to := range runs.States {
			tests = append(tests, tc{from, to, allowed[[2]runs.State{from, to}]})
		}
	}
	for _, tc := range tests {
		name := fmt.Sprintf("%s to %s", tc.from, tc.to)
		t.Run(name, func(t *testing.T) {
			if got := runs.CanTransition(tc.from, tc.to); got != tc.ok {
				t.Fatalf("CanTransition = %v, want %v", got, tc.ok)
			}
			next, err := runs.Transition(tc.from, tc.to)
			switch {
			case tc.ok && (err != nil || next != tc.to):
				t.Fatalf("Transition = %s, %v, want %s", next, err, tc.to)
			case !tc.ok && (!errors.Is(err, runs.ErrInvalidTransition) || next != tc.from):
				t.Fatalf("Transition = %s, %v, want %s and ErrInvalidTransition", next, err, tc.from)
			}
		})
	}
}

func TestTransitionRejectsUnknownStates(t *testing.T) {
	for _, tc := range []struct{ from, to runs.State }{
		{"", runs.Preparing},
		{runs.Pending, ""},
		{"paused", runs.Running},
		{runs.Running, "paused"},
	} {
		if _, err := runs.Transition(tc.from, tc.to); !errors.Is(err, runs.ErrInvalidTransition) {
			t.Errorf("Transition(%q, %q) = %v, want ErrInvalidTransition", tc.from, tc.to, err)
		}
	}
}

func TestFinalStates(t *testing.T) {
	for _, s := range runs.States {
		want := s == runs.Completed || s == runs.Failed || s == runs.Aborted
		if got := runs.Final(s); got != want {
			t.Errorf("Final(%s) = %v, want %v", s, got, want)
		}
		for _, to := range runs.States {
			if want && runs.CanTransition(s, to) {
				t.Errorf("final state %s may move to %s", s, to)
			}
		}
	}
}

func TestEvaluateTotals(t *testing.T) {
	tests := []struct {
		name string
		in   runs.Outcome
		want bool
	}{
		{"no criteria", runs.Outcome{Agents: 2}, true},
		{"ratio passes", runs.Outcome{Agents: 1, Criteria: crit("sessions.failed_ratio", "<", 0.1), Totals: map[string]float64{"sessions.failed_ratio": 0.05}}, true},
		{"ratio fails", runs.Outcome{Agents: 1, Criteria: crit("sessions.failed_ratio", "<", 0.1), Totals: map[string]float64{"sessions.failed_ratio": 0.1}}, false},
		{"unknown metric skipped", runs.Outcome{Agents: 1, Criteria: crit("mix.max_abs_error_pp", "<=", 2)}, true},
		{"unknown operator fails", runs.Outcome{Agents: 1, Criteria: crit("sessions.failed", "~", 0), Totals: map[string]float64{"sessions.failed": 0}}, false},
		{"every agent failed", runs.Outcome{Agents: 2, FailedAgents: 2}, false},
		{"one agent failed", runs.Outcome{Agents: 2, FailedAgents: 1}, true},
	}
	for _, op := range []struct {
		op   string
		want bool
	}{{"<", false}, {"<=", true}, {">", false}, {">=", true}, {"==", true}, {"!=", false}} {
		tests = append(tests, struct {
			name string
			in   runs.Outcome
			want bool
		}{"operator " + op.op, runs.Outcome{Agents: 1, Criteria: crit("sessions.failed", op.op, 3), Totals: map[string]float64{"sessions.failed": 3}}, op.want})
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			v := runs.EvaluateTotals(tc.in)
			if v.Passed != tc.want {
				t.Fatalf("Passed = %v, want %v: %+v", v.Passed, tc.want, v)
			}
			if len(v.Criteria) != len(tc.in.Criteria) {
				t.Fatalf("%d criterion results, want %d", len(v.Criteria), len(tc.in.Criteria))
			}
		})
	}
}
