// SPDX-License-Identifier: GPL-3.0-only

package multiagent

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strings"
	"time"
)

// MaxClockOffset is the largest absolute clock offset between an agent and the
// controller that a multi agent run accepts. Multi agent latency metrics need
// agents that share a time base, and the specification asks for NTP or PTP.
const MaxClockOffset = 10 * time.Millisecond

// Errors that callers test with errors.Is.
var (
	// ErrClockOffset reports that at least one agent clock differs from the
	// controller clock by more than the limit. The error is a *ClockError.
	ErrClockOffset = errors.New("agent clock offset above the limit")
	// ErrNoClockOffset reports that the controller has no measurement of an
	// agent clock. A run does not start on a guess.
	ErrNoClockOffset = errors.New("no clock offset measurement for the agent")
)

// ClockSource reports how far each agent clock is from the controller clock.
type ClockSource interface {
	// ClockOffset returns the agent clock minus the controller clock for the
	// agent id, so a positive value means that the agent runs ahead. It
	// returns an error that wraps ErrNoClockOffset when it has no
	// measurement.
	ClockOffset(ctx context.Context, id string) (time.Duration, error)
}

// ClockSourceFunc adapts a function to ClockSource.
type ClockSourceFunc func(ctx context.Context, id string) (time.Duration, error)

// ClockOffset calls f.
func (f ClockSourceFunc) ClockOffset(ctx context.Context, id string) (time.Duration, error) {
	return f(ctx, id)
}

// Offset is the measured clock offset of one agent.
type Offset struct {
	// AgentID names the agent.
	AgentID string
	// Offset is the agent clock minus the controller clock.
	Offset time.Duration
}

// ClockError is the error of CheckClocks when agents exceed the limit.
type ClockError struct {
	// Limit is the largest accepted absolute offset.
	Limit time.Duration
	// Offenders lists the agents above the limit, ordered by agent id.
	Offenders []Offset
}

// Error names every offender with its offset.
func (e *ClockError) Error() string {
	parts := make([]string, len(e.Offenders))
	for i, o := range e.Offenders {
		parts[i] = fmt.Sprintf("%s %s", o.AgentID, o.Offset)
	}
	return fmt.Sprintf("%s (limit %s): %s", ErrClockOffset, e.Limit, strings.Join(parts, ", "))
}

// Is makes errors.Is(err, ErrClockOffset) hold.
func (e *ClockError) Is(target error) bool { return target == ErrClockOffset }

// abs returns the absolute value of d, and saturates at the largest duration
// for the one value whose negation overflows.
func abs(d time.Duration) time.Duration {
	switch {
	case d >= 0:
		return d
	case d == time.Duration(-1<<63):
		return time.Duration(1<<63 - 1)
	}
	return -d
}

// CheckClocks reads the clock offset of every agent in ids and returns the
// offsets ordered by agent id. It fails with a *ClockError, which matches
// ErrClockOffset, when the absolute offset of any agent is above limit, and
// with an error that matches ErrNoClockOffset when a measurement is missing.
// An offset equal to the limit passes. A limit of zero or less means
// MaxClockOffset. CheckClocks sends nothing to an agent, so a caller runs it
// before it reserves or prepares anything.
func CheckClocks(ctx context.Context, src ClockSource, ids []string, limit time.Duration) ([]Offset, error) {
	if limit <= 0 {
		limit = MaxClockOffset
	}
	sorted := slices.Sorted(slices.Values(ids))
	sorted = slices.Compact(sorted)
	offsets := make([]Offset, 0, len(sorted))
	var bad []Offset
	for _, id := range sorted {
		d, err := src.ClockOffset(ctx, id)
		if err != nil {
			return nil, fmt.Errorf("clock check of agent %s: %w", id, err)
		}
		o := Offset{AgentID: id, Offset: d}
		offsets = append(offsets, o)
		if abs(d) > limit {
			bad = append(bad, o)
		}
	}
	if len(bad) > 0 {
		return offsets, &ClockError{Limit: limit, Offenders: bad}
	}
	return offsets, nil
}
