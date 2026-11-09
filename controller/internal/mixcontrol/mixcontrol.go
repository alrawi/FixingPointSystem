// SPDX-License-Identifier: GPL-3.0-only

// Package mixcontrol keeps the byte share of every mix entry at its weight
// during a run (CTL-12). A test with profile.basis "bytes" gives each scenario
// a share of the bytes, but the engine starts sessions, and a session of one
// scenario carries a different number of bytes than a session of another. The
// compiler can only guess the bytes per session from the manifest estimate
// expected_bytes. This package measures the real bytes per session of each
// entry and returns one multiplier per entry. The controller multiplies the
// compiled session rate of the entry by its multiplier, and then the byte share
// of the entry converges to its weight.
//
// The package is pure. It reads no clock and no random source: a caller that
// wants a timestamp passes the time to Update, and the same configuration and
// measurements always give the same results. State is a value, and Update
// returns the next state without changing the receiver.
//
// # Composition with the throughput calibration
//
// A run has one loop that sends UpdateRates (CTL-24 in internal/runs). This
// package does not send anything. The loop of internal/runs feeds the samples
// of each window to this package as well, and multiplies the multipliers into
// the per entry totals that it sends, next to its own throughput factor. To let
// both corrections compose, the multipliers of a PreserveBytes configuration
// leave the total bytes per second of the plan unchanged: the weighted sum of
// the session weight, the multiplier, and the bytes per session stays what the
// compiled rates give. The throughput calibration then scales that total to
// the objective, and the multipliers only move bytes between the entries. A
// PreserveSessions configuration leaves the total session rate unchanged
// instead, for an objective in sessions per second.
//
// # Estimate and control law
//
// Each update decays the pooled byte and session sums of every entry by Forget
// and adds the window. An entry whose pooled session count reaches MinSessions
// replaces its manifest estimate with the pooled bytes divided by the pooled
// sessions. The target multiplier of an entry follows from its byte weight b,
// its compiled session weight w, and its bytes per session estimate e: the
// session share that gives the entry the byte share b is proportional to b/e,
// so the multiplier is proportional to b/(w*e). Like the estimate of the
// throughput calibration, this rests on a property of the traffic and not on
// the achieved byte shares, so the loop has no feedback delay and cannot
// oscillate with the ramp of the agents. The applied multipliers move toward
// the targets, and the ratio of one update is bounded by MaxStep for each
// entry, so a large correction takes a few windows and one noisy window cannot
// throw the mix off. A dead band holds the multipliers while
// every target stays within DeadBand of them.
package mixcontrol

import (
	"errors"
	"fmt"
	"math"
	"time"
)

// DefaultWindow is the length of one control window, as the catalog of work
// packages asks for CTL-12: the per scenario rates are corrected every second.
const DefaultWindow = time.Second

// Defaults apply to Config fields that are zero.
const (
	// DefaultMaxStep lets a multiplier change by at most a factor of two per
	// update, so a distance of 1000 takes ten windows.
	DefaultMaxStep = 1.0
	// DefaultDeadBand holds the multipliers while every target stays within
	// 1.5 percent. This costs under one point of byte share.
	DefaultDeadBand = 0.015
	// DefaultMinSessions is the pooled session count that an entry needs
	// before its measurement replaces the manifest estimate.
	DefaultMinSessions = 20
	// DefaultForget is the weight that older windows keep at each update. At
	// one window per second, the estimate looks back about 20 seconds.
	DefaultForget = 0.95
)

// Errors that New and Update return. Callers test them with errors.Is.
var (
	// ErrConfig reports an invalid configuration.
	ErrConfig = errors.New("invalid mix control config")
	// ErrMeasurement reports an invalid measurement.
	ErrMeasurement = errors.New("invalid mix control measurement")
)

// Preserve says which total the multipliers leave unchanged.
type Preserve int

const (
	// PreserveBytes keeps the total bytes per second of the compiled rates,
	// which suits a throughput objective that another correction scales.
	PreserveBytes Preserve = iota
	// PreserveSessions keeps the total session rate of the compiled rates,
	// which suits an objective in sessions per second.
	PreserveSessions
)

// Entry is one mix entry of the run.
type Entry struct {
	// ID is the mix entry id, as in the compiled worker rates.
	ID string
	// ByteWeight is the weight that the byte share of the entry converges to.
	// Weights need not sum to one, because the package normalizes them.
	ByteWeight float64
	// SessionWeight is the session weight that the compiled rates carry for
	// the entry. It is the weight of the entry in the compiled mix, which the
	// multiplier scales. Weights need not sum to one.
	SessionWeight float64
	// ExpectedBytes is the bytes per session estimate that the compiler used,
	// from the manifest field expected_bytes. It stands in for the entry until
	// the measurements replace it.
	ExpectedBytes float64
}

// Config configures the control. Zero valued tuning fields take the defaults.
type Config struct {
	// Entries lists the mix entries.
	Entries []Entry
	// Preserve selects the total that the multipliers leave unchanged.
	Preserve Preserve
	// MaxStep bounds the relative change of a multiplier per update. A value
	// of 1 lets it double or halve.
	MaxStep float64
	// DeadBand is the relative distance below which the multipliers stay
	// unchanged. It must stay below MaxStep.
	DeadBand float64
	// MinSessions is the pooled, decayed session count that an entry needs
	// before its measured bytes per session replaces the estimate.
	MinSessions float64
	// Forget in (0,1) is the weight that the pooled sums keep at each update.
	// The effective window is about 1/(1-Forget) updates.
	Forget float64
}

// Sample is the measurement of one mix entry over one window. Bytes counts the
// application payload bytes that the completed sessions moved in both
// directions, and Sessions counts these sessions. A window without completed
// sessions carries zeros for both.
type Sample struct {
	// Entry is the mix entry id.
	Entry string
	// Bytes is the payload bytes of the Sessions completed sessions.
	Bytes float64
	// Sessions is the number of sessions that completed in the window.
	Sessions float64
}

// EntryResult is the state of one mix entry after an update.
type EntryResult struct {
	// ID is the mix entry id.
	ID string
	// BytesPerSession is the estimate that the multipliers rest on.
	BytesPerSession float64
	// Measured reports whether BytesPerSession comes from measurements and
	// not from the manifest estimate.
	Measured bool
	// Multiplier scales the compiled session rate of the entry.
	Multiplier float64
	// Target is the multiplier that the estimates call for.
	Target float64
	// ByteShare is the byte share that the applied multipliers give under
	// the current estimates, between 0 and 1.
	ByteShare float64
	// Weight is the normalized byte weight that ByteShare approaches.
	Weight float64
}

// Result is the outcome of one update.
type Result struct {
	// At is the time that the caller passed to Update.
	At time.Time
	// Changed reports that a multiplier differs from the previous update, so
	// the caller must send new rates.
	Changed bool
	// MaxErrorPP is the largest distance between a predicted byte share and
	// its weight, in percentage points.
	MaxErrorPP float64
	// SettledSince is the time of the last update that changed a multiplier,
	// or the time of the first update when none did. The caller reads it as
	// the start of the current stable period.
	SettledSince time.Time
	// Entries holds the per entry state in configuration order.
	Entries []EntryResult
}

// State is the control state. The zero value is not usable, so build one with
// New. States are values, and Update never changes its receiver.
type State struct {
	cfg     Config
	mult    []float64 // applied multipliers
	bytes   []float64 // decayed byte sums per entry
	count   []float64 // decayed session sums per entry
	byteW   []float64 // normalized byte weights
	sessW   []float64 // normalized session weights
	settled time.Time
	started bool
}

// New validates cfg, applies the defaults, and returns the initial state, whose
// multipliers are all one and so leave the compiled rates unchanged.
func New(cfg Config) (State, error) {
	if len(cfg.Entries) == 0 {
		return State{}, fmt.Errorf("%w: no mix entries", ErrConfig)
	}
	cfg.Entries = append([]Entry(nil), cfg.Entries...)
	if cfg.MaxStep == 0 {
		cfg.MaxStep = DefaultMaxStep
	}
	if cfg.DeadBand == 0 {
		cfg.DeadBand = DefaultDeadBand
	}
	if cfg.MinSessions == 0 {
		cfg.MinSessions = DefaultMinSessions
	}
	if cfg.Forget == 0 {
		cfg.Forget = DefaultForget
	}
	switch {
	case cfg.Preserve != PreserveBytes && cfg.Preserve != PreserveSessions:
		return State{}, fmt.Errorf("%w: unknown preserve mode %d", ErrConfig, cfg.Preserve)
	case !(cfg.MaxStep > 0) || cfg.MaxStep > 10:
		return State{}, fmt.Errorf("%w: max step %v is outside (0, 10]", ErrConfig, cfg.MaxStep)
	case !(cfg.DeadBand >= 0) || cfg.DeadBand >= cfg.MaxStep:
		return State{}, fmt.Errorf("%w: dead band %v is outside [0, max step %v)", ErrConfig, cfg.DeadBand, cfg.MaxStep)
	case !(cfg.MinSessions > 0) || math.IsInf(cfg.MinSessions, 0):
		return State{}, fmt.Errorf("%w: min sessions %v is not positive and finite", ErrConfig, cfg.MinSessions)
	case !(cfg.Forget > 0 && cfg.Forget < 1):
		return State{}, fmt.Errorf("%w: forget %v is outside (0, 1)", ErrConfig, cfg.Forget)
	}
	seen := make(map[string]bool, len(cfg.Entries))
	var byteSum, sessSum float64
	for i, e := range cfg.Entries {
		switch {
		case e.ID == "" || seen[e.ID]:
			return State{}, fmt.Errorf("%w: entry %d has an empty or duplicate id %q", ErrConfig, i, e.ID)
		case !positive(e.ByteWeight):
			return State{}, fmt.Errorf("%w: entry %s has byte weight %v, which is not positive and finite", ErrConfig, e.ID, e.ByteWeight)
		case !positive(e.SessionWeight):
			return State{}, fmt.Errorf("%w: entry %s has session weight %v, which is not positive and finite", ErrConfig, e.ID, e.SessionWeight)
		case !positive(e.ExpectedBytes):
			return State{}, fmt.Errorf("%w: entry %s has no bytes per session estimate", ErrConfig, e.ID)
		}
		seen[e.ID] = true
		byteSum += e.ByteWeight
		sessSum += e.SessionWeight
	}
	n := len(cfg.Entries)
	s := State{
		cfg: cfg, mult: make([]float64, n), bytes: make([]float64, n), count: make([]float64, n),
		byteW: make([]float64, n), sessW: make([]float64, n),
	}
	for i, e := range cfg.Entries {
		s.mult[i] = 1
		s.byteW[i] = e.ByteWeight / byteSum
		s.sessW[i] = e.SessionWeight / sessSum
	}
	return s, nil
}

func positive(v float64) bool { return v > 0 && !math.IsInf(v, 0) }

// Multipliers returns the applied multiplier of every entry by id.
func (s State) Multipliers() map[string]float64 {
	out := make(map[string]float64, len(s.mult))
	for i, e := range s.cfg.Entries {
		out[e.ID] = s.mult[i]
	}
	return out
}

// estimate returns the bytes per session estimate of entry i.
func (s State) estimate(i int) (bps float64, measured bool) {
	if s.count[i] >= s.cfg.MinSessions && s.bytes[i] > 0 {
		return s.bytes[i] / s.count[i], true
	}
	return s.cfg.Entries[i].ExpectedBytes, false
}

// targets returns the multipliers that give every entry its byte weight when
// the entries carry the bytes per session in bps.
func (s State) targets(bps []float64) []float64 {
	t := make([]float64, len(bps))
	switch s.cfg.Preserve {
	case PreserveSessions:
		// The session share that gives entry i the byte share b_i is
		// proportional to b_i/e_i. The shares sum to one, as the compiled
		// session weights do.
		var sum float64
		for i := range t {
			sum += s.byteW[i] / bps[i]
		}
		for i := range t {
			t[i] = s.byteW[i] / bps[i] / sum / s.sessW[i]
		}
	default:
		// The total bytes per second stays the compiled one, which is the
		// weighted sum p of the session weight and the bytes per session.
		var p float64
		for i := range t {
			p += s.sessW[i] * bps[i]
		}
		for i := range t {
			t[i] = s.byteW[i] / (s.sessW[i] * bps[i]) * p
		}
	}
	return t
}

// Update folds one window of measurements into the state, stamped with now,
// and returns the next state and the result. Samples may omit entries and may
// list an entry once at most. An entry without completed sessions leaves its
// pooled sums decayed and otherwise unchanged. An invalid sample returns an
// error and the old state.
func (s State) Update(now time.Time, samples []Sample) (State, Result, error) {
	if len(s.bytes) == 0 {
		return s, Result{}, fmt.Errorf("%w: state was not built with New", ErrConfig)
	}
	idx := make(map[string]int, len(s.cfg.Entries))
	for i, e := range s.cfg.Entries {
		idx[e.ID] = i
	}
	add := make([]Sample, len(s.cfg.Entries))
	got := make([]bool, len(s.cfg.Entries))
	for _, m := range samples {
		i, ok := idx[m.Entry]
		switch {
		case !ok:
			return s, Result{}, fmt.Errorf("%w: unknown mix entry %q", ErrMeasurement, m.Entry)
		case got[i]:
			return s, Result{}, fmt.Errorf("%w: mix entry %q appears twice", ErrMeasurement, m.Entry)
		case !(m.Bytes >= 0) || !(m.Sessions >= 0) || math.IsInf(m.Bytes, 0) || math.IsInf(m.Sessions, 0):
			return s, Result{}, fmt.Errorf("%w: mix entry %q has bytes %v and sessions %v", ErrMeasurement, m.Entry, m.Bytes, m.Sessions)
		case m.Sessions == 0 && m.Bytes != 0:
			return s, Result{}, fmt.Errorf("%w: mix entry %q has %v bytes and no sessions", ErrMeasurement, m.Entry, m.Bytes)
		}
		got[i] = true
		add[i] = m
	}

	n := s
	n.bytes = make([]float64, len(s.bytes))
	n.count = make([]float64, len(s.count))
	for i := range s.bytes {
		n.bytes[i] = s.bytes[i]*s.cfg.Forget + add[i].Bytes
		n.count[i] = s.count[i]*s.cfg.Forget + add[i].Sessions
	}
	bps := make([]float64, len(s.bytes))
	measured := make([]bool, len(s.bytes))
	for i := range bps {
		bps[i], measured[i] = n.estimate(i)
	}
	target := n.targets(bps)

	// The dead band holds every multiplier while the largest relative distance
	// to a target stays small. Outside the band, each multiplier moves toward
	// its target by at most a factor of 1+MaxStep, so an entry with a small
	// share and a distant target, such as a scenario with tiny sessions, does
	// not hold back the entries that carry the bytes.
	var worst float64
	for i := range target {
		worst = math.Max(worst, math.Abs(target[i]/s.mult[i]-1))
	}
	n.mult = append([]float64(nil), s.mult...)
	changed := false
	if worst > s.cfg.DeadBand {
		for i := range n.mult {
			lo, hi := s.mult[i]/(1+s.cfg.MaxStep), s.mult[i]*(1+s.cfg.MaxStep)
			n.mult[i] = math.Min(math.Max(target[i], lo), hi)
			if n.mult[i] != s.mult[i] {
				changed = true
			}
		}
	}
	if !n.started || changed {
		n.settled = now
	}
	n.started = true

	res := Result{At: now, Changed: changed, SettledSince: n.settled, Entries: make([]EntryResult, len(bps))}
	var total float64
	for i := range bps {
		total += n.sessW[i] * n.mult[i] * bps[i]
	}
	for i, e := range s.cfg.Entries {
		share := n.sessW[i] * n.mult[i] * bps[i] / total
		res.Entries[i] = EntryResult{
			ID: e.ID, BytesPerSession: bps[i], Measured: measured[i], Multiplier: n.mult[i],
			Target: target[i], ByteShare: share, Weight: n.byteW[i],
		}
		res.MaxErrorPP = math.Max(res.MaxErrorPP, 100*math.Abs(share-n.byteW[i]))
	}
	return n, res, nil
}
