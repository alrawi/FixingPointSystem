// SPDX-License-Identifier: GPL-3.0-only

package mixcontrol_test

import (
	"errors"
	"math"
	"math/rand/v2"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/mixcontrol"
)

var t0 = time.Date(2026, 11, 9, 12, 0, 0, 0, time.UTC)

// A mix like the enterprise example: byte weights 60, 39.9, and 0.1 over three
// scenarios whose sessions carry 20 kB, 200 kB, and 200 B.
var (
	byteWeights = []float64{60, 39.9, 0.1}
	trueBytes   = []float64{20e3, 200e3, 200}
	ids         = []string{"web_browse", "web_get", "lookup"}
)

func entries(expected ...float64) []mixcontrol.Entry {
	out := make([]mixcontrol.Entry, len(ids))
	for i := range out {
		// The compiler normalizes the weights as session weights.
		out[i] = mixcontrol.Entry{ID: ids[i], ByteWeight: byteWeights[i], SessionWeight: byteWeights[i], ExpectedBytes: expected[i]}
	}
	return out
}

// shares returns the byte shares that session rates give when the entries
// carry the true bytes per session.
func shares(rates []float64) []float64 {
	var total float64
	b := make([]float64, len(rates))
	for i, r := range rates {
		b[i] = r * trueBytes[i]
		total += b[i]
	}
	for i := range b {
		b[i] /= total
	}
	return b
}

func maxErrorPP(rates []float64) float64 {
	var worst float64
	for i, s := range shares(rates) {
		worst = math.Max(worst, 100*math.Abs(s-byteWeights[i]/100))
	}
	return worst
}

// simulate runs the closed loop of one control window per second. The agents
// run totalRate sessions per second split by the compiled weights times the
// multipliers, each session carries its true bytes with relative noise, and the
// session counts of a window are noisy. It returns the error in points of byte
// share of every window, measured on the expected rates of that window.
func simulate(t *testing.T, cfg mixcontrol.Config, seed uint64, windows int) []float64 {
	t.Helper()
	st, err := mixcontrol.New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	rng := rand.New(rand.NewPCG(seed, 1)) //nolint:gosec // G404: seeded test data
	var sumW float64
	for _, e := range cfg.Entries {
		sumW += e.SessionWeight
	}
	const totalRate = 1000.0
	errs := make([]float64, 0, windows)
	for w := 0; w < windows; w++ {
		mult := st.Multipliers()
		rates := make([]float64, len(ids))
		samples := make([]mixcontrol.Sample, len(ids))
		for i, e := range cfg.Entries {
			rates[i] = totalRate * e.SessionWeight / sumW * mult[e.ID]
			n := math.Max(0, math.Round(rates[i]+rng.NormFloat64()*math.Sqrt(rates[i])))
			var b float64
			if n > 0 {
				// The mean of n sessions with 40 percent noise per session.
				b = n * trueBytes[i] * (1 + 0.4*rng.NormFloat64()/math.Sqrt(n))
			}
			samples[i] = mixcontrol.Sample{Entry: e.ID, Bytes: b, Sessions: n}
		}
		errs = append(errs, maxErrorPP(rates))
		next, _, err := st.Update(t0.Add(time.Duration(w)*time.Second), samples)
		if err != nil {
			t.Fatal(err)
		}
		st = next
	}
	return errs
}

// TestConvergesWithinThirtyWindowsAndStays is the unit level form of the CTL-12
// acceptance: with estimates that are off by a factor of four in both
// directions, and noisy sessions, the byte shares reach 2 points of the
// weights within 30 one second windows and stay there.
func TestConvergesWithinThirtyWindowsAndStays(t *testing.T) {
	for _, expected := range [][]float64{
		{20e3, 200e3, 200}, {5e3, 800e3, 800}, {80e3, 50e3, 50}, {1e3, 1e3, 1e3},
	} {
		for seed := uint64(1); seed <= 20; seed++ {
			cfg := mixcontrol.Config{Entries: entries(expected...)}
			errs := simulate(t, cfg, seed, 300)
			for w := 30; w < len(errs); w++ {
				if errs[w] > 2 {
					t.Fatalf("expected %v seed %d: window %d has %.2f points of error, want at most 2 from window 30 on", expected, seed, w, errs[w])
				}
			}
		}
	}
}

// TestSettlesWithFewMoves checks that noise around the settled multipliers does
// not make the loop send new rates every window.
func TestSettlesWithFewMoves(t *testing.T) {
	cfg := mixcontrol.Config{Entries: entries(5e3, 800e3, 800)}
	st, err := mixcontrol.New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	rng := rand.New(rand.NewPCG(9, 9)) //nolint:gosec // G404: seeded test data
	changes := 0
	for w := 0; w < 300; w++ {
		samples := make([]mixcontrol.Sample, len(ids))
		for i := range ids {
			n := 300.0 * byteWeights[i] / 100
			if i == 2 {
				n = 30
			}
			samples[i] = mixcontrol.Sample{Entry: ids[i], Sessions: n, Bytes: n * trueBytes[i] * (1 + 0.05*rng.NormFloat64())}
		}
		next, res, err := st.Update(t0.Add(time.Duration(w)*time.Second), samples)
		if err != nil {
			t.Fatal(err)
		}
		st = next
		if res.Changed {
			changes++
		}
	}
	if changes > 40 {
		t.Errorf("%d windows changed the multipliers out of 300, want a settled loop", changes)
	}
}

func feed(t *testing.T, st mixcontrol.State, windows int, per float64) (mixcontrol.State, mixcontrol.Result) {
	t.Helper()
	var res mixcontrol.Result
	for w := 0; w < windows; w++ {
		samples := make([]mixcontrol.Sample, len(ids))
		for i := range ids {
			samples[i] = mixcontrol.Sample{Entry: ids[i], Sessions: per, Bytes: per * trueBytes[i]}
		}
		var err error
		st, res, err = st.Update(t0.Add(time.Duration(w)*time.Second), samples)
		if err != nil {
			t.Fatal(err)
		}
	}
	return st, res
}

func TestPreserveBytesKeepsTheCompiledTotal(t *testing.T) {
	st, err := mixcontrol.New(mixcontrol.Config{Entries: entries(20e3, 200e3, 200)})
	if err != nil {
		t.Fatal(err)
	}
	st, res := feed(t, st, 60, 100)
	m := st.Multipliers()
	var compiled, applied float64
	for i, id := range ids {
		w := byteWeights[i] / 100
		compiled += w * trueBytes[i]
		applied += w * m[id] * trueBytes[i]
	}
	if math.Abs(applied/compiled-1) > 1e-6 {
		t.Errorf("total bytes per session moved from %v to %v", compiled, applied)
	}
	for _, e := range res.Entries {
		if !e.Measured {
			t.Errorf("entry %s still uses the estimate", e.ID)
		}
		if math.Abs(e.ByteShare-e.Weight) > 0.02 {
			t.Errorf("entry %s predicts byte share %v for weight %v", e.ID, e.ByteShare, e.Weight)
		}
	}
	if res.MaxErrorPP > 2 {
		t.Errorf("max error %v points", res.MaxErrorPP)
	}
}

func TestPreserveSessionsKeepsTheSessionRate(t *testing.T) {
	cfg := mixcontrol.Config{Entries: entries(20e3, 200e3, 200), Preserve: mixcontrol.PreserveSessions}
	st, err := mixcontrol.New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	st, res := feed(t, st, 60, 100)
	m := st.Multipliers()
	var sessions float64
	for i, id := range ids {
		sessions += byteWeights[i] / 100 * m[id]
	}
	if math.Abs(sessions-1) > 1e-6 {
		t.Errorf("session rate moved to %v times the compiled rate", sessions)
	}
	if res.MaxErrorPP > 2 {
		t.Errorf("max error %v points", res.MaxErrorPP)
	}
}

// TestEstimatesShapeTheMixBeforeMeasurements checks that an entry without
// enough sessions runs on its manifest estimate.
func TestEstimatesShapeTheMixBeforeMeasurements(t *testing.T) {
	st, err := mixcontrol.New(mixcontrol.Config{Entries: entries(20e3, 200e3, 200)})
	if err != nil {
		t.Fatal(err)
	}
	_, res, err := st.Update(t0, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !res.Changed {
		t.Fatal("the estimates call for other multipliers than one")
	}
	for _, e := range res.Entries {
		if e.Measured || e.BytesPerSession == 0 {
			t.Errorf("entry %s: %+v", e.ID, e)
		}
	}
	// The first step is bounded: no multiplier moves by more than a factor of two.
	for _, e := range res.Entries {
		if e.Multiplier > 2+1e-9 || e.Multiplier < 0.5-1e-9 {
			t.Errorf("entry %s moved to %v in one step", e.ID, e.Multiplier)
		}
	}
}

// TestDeadBandHoldsTheMultipliers feeds a mix whose compiled rates already give
// the byte shares, and expects no change.
func TestDeadBandHoldsTheMultipliers(t *testing.T) {
	// Session weights that are the byte weights over the bytes per session.
	es := entries(20e3, 200e3, 200)
	for i := range es {
		es[i].SessionWeight = byteWeights[i] / trueBytes[i]
	}
	st, err := mixcontrol.New(mixcontrol.Config{Entries: es})
	if err != nil {
		t.Fatal(err)
	}
	st, res := feed(t, st, 40, 100)
	if res.Changed {
		t.Error("a mix that is on target changed")
	}
	for id, m := range st.Multipliers() {
		if math.Abs(m-1) > 1e-9 {
			t.Errorf("multiplier of %s is %v, want 1", id, m)
		}
	}
	if !res.SettledSince.Equal(t0) {
		t.Errorf("settled since %v, want the first update %v", res.SettledSince, t0)
	}
}

func TestSettledSinceFollowsTheLastChange(t *testing.T) {
	st, err := mixcontrol.New(mixcontrol.Config{Entries: entries(5e3, 800e3, 800)})
	if err != nil {
		t.Fatal(err)
	}
	_, res := feed(t, st, 80, 100)
	if res.Changed || res.SettledSince.Equal(t0) || res.SettledSince.After(res.At) {
		t.Errorf("settled since %v at %v, changed %v", res.SettledSince, res.At, res.Changed)
	}
}

func TestUpdateDoesNotChangeTheReceiver(t *testing.T) {
	st, err := mixcontrol.New(mixcontrol.Config{Entries: entries(5e3, 800e3, 800)})
	if err != nil {
		t.Fatal(err)
	}
	before := st.Multipliers()
	_, _, err = st.Update(t0, []mixcontrol.Sample{{Entry: "web_get", Bytes: 1e6, Sessions: 5}})
	if err != nil {
		t.Fatal(err)
	}
	for id, m := range st.Multipliers() {
		if m != before[id] || m != 1 {
			t.Errorf("multiplier of %s changed to %v", id, m)
		}
	}
}

func TestDeterministic(t *testing.T) {
	cfg := mixcontrol.Config{Entries: entries(5e3, 800e3, 800)}
	a := simulate(t, cfg, 3, 60)
	b := simulate(t, cfg, 3, 60)
	for i := range a {
		if a[i] != b[i] {
			t.Fatalf("window %d differs: %v and %v", i, a[i], b[i])
		}
	}
}

func TestNewRejectsBadConfigs(t *testing.T) {
	good := entries(20e3, 200e3, 200)
	mutate := func(f func(*mixcontrol.Config)) mixcontrol.Config {
		c := mixcontrol.Config{Entries: append([]mixcontrol.Entry(nil), good...)}
		f(&c)
		return c
	}
	for name, cfg := range map[string]mixcontrol.Config{
		"no entries":      {},
		"empty id":        mutate(func(c *mixcontrol.Config) { c.Entries[0].ID = "" }),
		"duplicate id":    mutate(func(c *mixcontrol.Config) { c.Entries[1].ID = c.Entries[0].ID }),
		"zero byte w":     mutate(func(c *mixcontrol.Config) { c.Entries[0].ByteWeight = 0 }),
		"nan session w":   mutate(func(c *mixcontrol.Config) { c.Entries[0].SessionWeight = math.NaN() }),
		"no estimate":     mutate(func(c *mixcontrol.Config) { c.Entries[2].ExpectedBytes = 0 }),
		"inf estimate":    mutate(func(c *mixcontrol.Config) { c.Entries[2].ExpectedBytes = math.Inf(1) }),
		"bad preserve":    mutate(func(c *mixcontrol.Config) { c.Preserve = 7 }),
		"step too big":    mutate(func(c *mixcontrol.Config) { c.MaxStep = 11 }),
		"band over step":  mutate(func(c *mixcontrol.Config) { c.MaxStep, c.DeadBand = 0.1, 0.2 }),
		"negative sess":   mutate(func(c *mixcontrol.Config) { c.MinSessions = -1 }),
		"forget of one":   mutate(func(c *mixcontrol.Config) { c.Forget = 1 }),
		"negative forget": mutate(func(c *mixcontrol.Config) { c.Forget = -0.5 }),
	} {
		if _, err := mixcontrol.New(cfg); !errors.Is(err, mixcontrol.ErrConfig) {
			t.Errorf("%s: error %v, want ErrConfig", name, err)
		}
	}
}

func TestUpdateRejectsBadSamples(t *testing.T) {
	st, err := mixcontrol.New(mixcontrol.Config{Entries: entries(20e3, 200e3, 200)})
	if err != nil {
		t.Fatal(err)
	}
	for name, samples := range map[string][]mixcontrol.Sample{
		"unknown entry": {{Entry: "nope", Bytes: 1, Sessions: 1}},
		"twice":         {{Entry: "web_get", Sessions: 1, Bytes: 1}, {Entry: "web_get", Sessions: 1, Bytes: 1}},
		"negative":      {{Entry: "web_get", Sessions: -1}},
		"nan":           {{Entry: "web_get", Sessions: 1, Bytes: math.NaN()}},
		"bytes alone":   {{Entry: "web_get", Bytes: 5}},
	} {
		if _, _, err := st.Update(t0, samples); !errors.Is(err, mixcontrol.ErrMeasurement) {
			t.Errorf("%s: error %v, want ErrMeasurement", name, err)
		}
	}
	if _, _, err := (mixcontrol.State{}).Update(t0, nil); !errors.Is(err, mixcontrol.ErrConfig) {
		t.Errorf("zero state: error %v, want ErrConfig", err)
	}
}
