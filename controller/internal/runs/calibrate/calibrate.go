// SPDX-License-Identifier: GPL-3.0-only

// Package calibrate corrects the session rate of a throughput_bps objective
// during a run (decision D12 item 10).
//
// The compiler turns the objective into session rates with an estimate of the
// bytes per session, the manifest field expected_bytes. When the estimate is
// wrong, the run misses the objective by the same ratio. This package measures
// the real bytes per session from the completed sessions of each mix entry,
// pools the measurements over several windows to damp noise, and returns a
// Factor that scales every compiled session rate of the run.
//
// The package is pure. It reads no clock and no random source, and the same
// configuration and measurements always give the same results. State is a
// value, and Update returns the next state without changing the receiver.
//
// The correction never reacts to the achieved throughput. It estimates a
// property of the traffic, the bytes per session, so it does not depend on how
// fast the agents ramp and it has no feedback delay that could oscillate. The
// remaining safeguards are a bound on the relative step of the factor, a bound
// on the factor itself, a dead band around the current factor, and a minimum
// number of pooled sessions before a measurement counts.
package calibrate

import (
	"errors"
	"fmt"
	"math"
)

// Defaults apply to Config fields that are zero.
const (
	// DefaultMaxStep lets the factor change by at most 50 percent per update.
	DefaultMaxStep = 0.5
	// DefaultDeadBand holds the factor while the target stays within 1.5 percent.
	DefaultDeadBand = 0.015
	// DefaultMinSessions is the pooled session count that an entry needs
	// before its measurement replaces the manifest estimate.
	DefaultMinSessions = 20
	// DefaultForget is the weight that older windows keep at each update.
	DefaultForget = 0.97
	// DefaultMaxFactor bounds the factor to [1/64, 64].
	DefaultMaxFactor = 64
)

// reverseBandRatio widens the dead band for a move against the last move.
const reverseBandRatio = 1.5

// Errors that New and Update return. Callers test them with errors.Is.
var (
	// ErrConfig reports an invalid configuration.
	ErrConfig = errors.New("invalid calibration config")
	// ErrMeasurement reports an invalid measurement.
	ErrMeasurement = errors.New("invalid calibration measurement")
)

// Entry is one mix entry of the run.
type Entry struct {
	// ID is the mix entry id, as in the compiled worker rates.
	ID string
	// Weight is the session weight of the entry in the mix. Weights need not
	// sum to one, because the package normalizes them.
	Weight float64
	// ExpectedBytes is the bytes per session estimate that the compiler used,
	// from the manifest field expected_bytes.
	ExpectedBytes float64
}

// Config configures a calibration. Zero valued tuning fields take defaults.
type Config struct {
	// ObjectiveBps is the throughput objective in bits per second.
	ObjectiveBps float64
	// Entries lists the mix entries with their compile time estimates.
	Entries []Entry
	// MaxStep bounds the relative change of the factor per update. A value of
	// 0.5 keeps the next factor between factor/1.5 and factor*1.5.
	MaxStep float64
	// DeadBand is the relative distance between the target and the current
	// factor below which the factor stays unchanged. A change against the
	// direction of the previous change needs 1.5 times that distance. It must
	// stay below MaxStep, and it should stay well below the accuracy that the
	// objective needs, which leaves room for the noise of the estimate.
	DeadBand float64
	// MinSessions is the pooled, decayed session count that an entry needs
	// before its measured bytes per session replaces the estimate.
	MinSessions float64
	// Forget in (0,1) is the weight that the pooled sums keep at each update.
	// The effective window is about 1/(1-Forget) updates.
	Forget float64
	// MaxFactor bounds the factor to [1/MaxFactor, MaxFactor].
	MaxFactor float64
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

// EntryResult is the calibration state of one mix entry after an update.
type EntryResult struct {
	// ID is the mix entry id.
	ID string
	// BytesPerSession is the estimate that the next rates rest on.
	BytesPerSession float64
	// Measured reports whether BytesPerSession comes from measurements and
	// not from the manifest estimate.
	Measured bool
}

// Result is the outcome of one update.
type Result struct {
	// Factor scales every compiled session rate of the run. The compiled
	// rates correspond to a factor of one.
	Factor float64
	// Changed reports that Factor differs from the factor of the previous
	// update, so the caller must send new rates.
	Changed bool
	// Target is the unbounded factor that the current estimates call for,
	// limited only by MaxFactor.
	Target float64
	// SessionsPerSecond is the total session rate that Factor implies across
	// the mix.
	SessionsPerSecond float64
	// Entries holds the per entry estimates in configuration order.
	Entries []EntryResult
}

// State is the calibration state. The zero value is not usable, so build one
// with New. States are values, and Update never changes its receiver.
type State struct {
	cfg    Config
	factor float64
	dir    int       // direction of the last change, -1, 0, or +1
	bytes  []float64 // decayed byte sums per entry
	count  []float64 // decayed session sums per entry
	weight []float64 // normalized weights
}

// New validates cfg, applies the defaults, and returns the initial state with
// a factor of one, which leaves the compiled rates unchanged.
func New(cfg Config) (State, error) {
	if !(cfg.ObjectiveBps > 0) || math.IsInf(cfg.ObjectiveBps, 0) {
		return State{}, fmt.Errorf("%w: objective %v bit/s is not positive and finite", ErrConfig, cfg.ObjectiveBps)
	}
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
	if cfg.MaxFactor == 0 {
		cfg.MaxFactor = DefaultMaxFactor
	}
	switch {
	case !(cfg.MaxStep > 0) || cfg.MaxStep > 10:
		return State{}, fmt.Errorf("%w: max step %v is outside (0, 10]", ErrConfig, cfg.MaxStep)
	case !(cfg.DeadBand >= 0) || cfg.DeadBand >= cfg.MaxStep:
		return State{}, fmt.Errorf("%w: dead band %v is outside [0, max step %v)", ErrConfig, cfg.DeadBand, cfg.MaxStep)
	case !(cfg.MinSessions > 0) || math.IsInf(cfg.MinSessions, 0):
		return State{}, fmt.Errorf("%w: min sessions %v is not positive and finite", ErrConfig, cfg.MinSessions)
	case !(cfg.Forget > 0 && cfg.Forget < 1):
		return State{}, fmt.Errorf("%w: forget %v is outside (0, 1)", ErrConfig, cfg.Forget)
	case !(cfg.MaxFactor > 1) || math.IsInf(cfg.MaxFactor, 0):
		return State{}, fmt.Errorf("%w: max factor %v is not above 1 and finite", ErrConfig, cfg.MaxFactor)
	}
	seen := make(map[string]bool, len(cfg.Entries))
	var total float64
	for i, e := range cfg.Entries {
		switch {
		case e.ID == "" || seen[e.ID]:
			return State{}, fmt.Errorf("%w: entry %d has an empty or duplicate id %q", ErrConfig, i, e.ID)
		case !(e.Weight > 0) || math.IsInf(e.Weight, 0):
			return State{}, fmt.Errorf("%w: entry %s has weight %v, which is not positive and finite", ErrConfig, e.ID, e.Weight)
		case !(e.ExpectedBytes > 0) || math.IsInf(e.ExpectedBytes, 0):
			return State{}, fmt.Errorf("%w: entry %s has no bytes per session estimate", ErrConfig, e.ID)
		}
		seen[e.ID] = true
		total += e.Weight
	}
	n := len(cfg.Entries)
	s := State{cfg: cfg, factor: 1, bytes: make([]float64, n), count: make([]float64, n), weight: make([]float64, n)}
	for i, e := range cfg.Entries {
		s.weight[i] = e.Weight / total
	}
	return s, nil
}

// Factor returns the current factor.
func (s State) Factor() float64 { return s.factor }

// BaseSessionsPerSecond returns the total session rate that the compiler
// derives from the objective and the manifest estimates, which is a factor of
// one.
func (s State) BaseSessionsPerSecond() float64 {
	var b float64
	for i, e := range s.cfg.Entries {
		b += s.weight[i] * e.ExpectedBytes
	}
	return s.cfg.ObjectiveBps / (8 * b)
}

// estimate returns the bytes per session estimate of entry i.
func (s State) estimate(i int) (bps float64, measured bool) {
	if s.count[i] >= s.cfg.MinSessions && s.bytes[i] > 0 {
		return s.bytes[i] / s.count[i], true
	}
	return s.cfg.Entries[i].ExpectedBytes, false
}

// Update folds one window of measurements into the state and returns the next
// state and the result. Samples may omit entries and may list an entry once at
// most. An entry without completed sessions leaves its pooled sums decayed and
// otherwise unchanged. An invalid sample returns an error and the old state.
func (s State) Update(samples []Sample) (State, Result, error) {
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

	// The weighted mean bytes per session gives the total session rate that
	// meets the objective. The ratio to the compiled rate is the target.
	var est, base float64
	res := Result{Entries: make([]EntryResult, len(s.cfg.Entries))}
	for i, e := range s.cfg.Entries {
		b, measured := n.estimate(i)
		est += s.weight[i] * b
		base += s.weight[i] * e.ExpectedBytes
		res.Entries[i] = EntryResult{ID: e.ID, BytesPerSession: b, Measured: measured}
	}
	target := math.Min(math.Max(base/est, 1/s.cfg.MaxFactor), s.cfg.MaxFactor)
	res.Target = target

	// The dead band holds the factor against noise. A move against the
	// direction of the last change needs a target outside a band that is
	// half as wide again, so noise around a settled factor cannot make it
	// zigzag.
	next := s.factor
	dev := target/s.factor - 1
	band := s.cfg.DeadBand
	if dev*float64(s.dir) < 0 {
		band *= reverseBandRatio
	}
	if math.Abs(dev) > band {
		lo, hi := s.factor/(1+s.cfg.MaxStep), s.factor*(1+s.cfg.MaxStep)
		next = math.Min(math.Max(target, lo), hi)
	}
	n.factor = next
	switch {
	case next > s.factor:
		n.dir = 1
	case next < s.factor:
		n.dir = -1
	}
	res.Factor = next
	res.Changed = next != s.factor
	res.SessionsPerSecond = s.BaseSessionsPerSecond() * next
	return n, res, nil
}
