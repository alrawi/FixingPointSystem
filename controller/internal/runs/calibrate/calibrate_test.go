// SPDX-License-Identifier: GPL-3.0-only

package calibrate

import (
	"errors"
	"math"
	"math/rand/v2"
	"testing"
)

const (
	trueBytes = 17244.0 // the http1/web_get bytes per session of spike S4
	objective = 40e6    // bits per second
	window    = 10.0    // seconds per measurement window
)

// simulate runs a closed loop. Each window completes sessions at the rate that
// the current factor implies, every session moves a noisy number of bytes, and
// the window mean carries extra measurement noise. It returns the factor after
// every update and the achieved to objective ratio after every update.
func simulate(t *testing.T, seed uint64, estimate, sessionSD, windowSD float64, updates int) (factors, ratios []float64) {
	t.Helper()
	s, err := New(Config{ObjectiveBps: objective, Entries: []Entry{{ID: "web", Weight: 1, ExpectedBytes: estimate * trueBytes}}})
	if err != nil {
		t.Fatal(err)
	}
	rng := rand.New(rand.NewPCG(seed, 0x5eed)) //nolint:gosec // G404: a seeded generator keeps the simulation deterministic
	factor := 1.0
	for i := 0; i < updates; i++ {
		rate := s.BaseSessionsPerSecond() * factor
		n := math.Max(1, math.Round(rate*window*(1+0.1*rng.NormFloat64())))
		var bytes float64
		for j := 0; j < int(n); j++ {
			bytes += math.Max(1, trueBytes*(1+sessionSD*rng.NormFloat64()))
		}
		bytes *= 1 + windowSD*rng.NormFloat64()
		var res Result
		s, res, err = s.Update([]Sample{{Entry: "web", Bytes: bytes, Sessions: n}})
		if err != nil {
			t.Fatal(err)
		}
		factor = res.Factor
		factors = append(factors, factor)
		ratios = append(ratios, res.SessionsPerSecond*8*trueBytes/objective)
	}
	return factors, ratios
}

func TestConvergesFromWrongEstimates(t *testing.T) {
	const updates, settle = 60, 25
	for _, estimate := range []float64{0.25, 0.5, 1, 2, 4} {
		for seed := uint64(1); seed <= 40; seed++ {
			factors, ratios := simulate(t, seed, estimate, 0.4, 0.05, updates)
			prev := 1.0
			var lastDir float64
			var reversals int
			for i, f := range factors {
				// Every step stays within the configured bound.
				if f > prev*(1+DefaultMaxStep)*(1+1e-12) || f < prev/(1+DefaultMaxStep)*(1-1e-12) {
					t.Fatalf("estimate %vx seed %d update %d: step %v to %v exceeds the bound", estimate, seed, i, prev, f)
				}
				if d := f - prev; d != 0 {
					if lastDir != 0 && (d > 0) != (lastDir > 0) {
						reversals++
					}
					lastDir = d
					// Once settled, the noise moves the factor by a few
					// percent at most.
					if i >= settle && math.Abs(f/prev-1) > 0.05 {
						t.Fatalf("estimate %vx seed %d update %d: settled step %v to %v exceeds 5 percent", estimate, seed, i, prev, f)
					}
				}
				prev = f
			}
			for i := settle; i < updates; i++ {
				if math.Abs(ratios[i]-1) > 0.05 {
					t.Fatalf("estimate %vx seed %d update %d: achieved %.3f of the objective, want within 5 percent", estimate, seed, i, ratios[i])
				}
			}
			// The reverse dead band keeps jitter around the settled factor
			// to a few small moves.
			if reversals > 6 {
				t.Errorf("estimate %vx seed %d: %d direction reversals, want at most 6", estimate, seed, reversals)
			}
		}
	}
}

func TestFirstStepsAreBounded(t *testing.T) {
	// With exact measurements an estimate of 0.25x needs a factor of 0.25, which takes
	// three bounded steps of 1.5x, and the factor never overshoots.
	s, err := New(Config{ObjectiveBps: objective, Entries: []Entry{{ID: "web", Weight: 1, ExpectedBytes: 0.25 * trueBytes}}})
	if err != nil {
		t.Fatal(err)
	}
	var factors []float64
	for i := 0; i < 8; i++ {
		var res Result
		s, res, err = s.Update([]Sample{{Entry: "web", Bytes: 100 * trueBytes, Sessions: 100}})
		if err != nil {
			t.Fatal(err)
		}
		factors = append(factors, res.Factor)
	}
	// Estimate 0.25x means the compiled rate is 4x too high, so the target is 0.25.
	want := []float64{1 / 1.5, 1 / 2.25, 1 / 3.375, 0.25, 0.25, 0.25, 0.25, 0.25}
	for i := range want {
		if math.Abs(factors[i]-want[i]) > 1e-9 {
			t.Fatalf("factors = %v, want %v", factors, want)
		}
	}
}

func TestHoldsInsideDeadBandAndBeforeMinSessions(t *testing.T) {
	s, err := New(Config{ObjectiveBps: objective, Entries: []Entry{{ID: "a", Weight: 1, ExpectedBytes: 1000}}})
	if err != nil {
		t.Fatal(err)
	}
	// Fewer pooled sessions than MinSessions leave the estimate alone.
	s, res, err := s.Update([]Sample{{Entry: "a", Bytes: 5 * 4000, Sessions: 5}})
	if err != nil || res.Changed || res.Factor != 1 || res.Entries[0].Measured {
		t.Fatalf("early update = %+v, %v", res, err)
	}
	// A 1 percent difference lies inside the dead band.
	s, res, err = s.Update([]Sample{{Entry: "a", Bytes: 0, Sessions: 0}})
	if err != nil || res.Changed {
		t.Fatalf("empty window = %+v, %v", res, err)
	}
	s2, _ := New(Config{ObjectiveBps: objective, Entries: []Entry{{ID: "a", Weight: 1, ExpectedBytes: 1000}}})
	_, res, err = s2.Update([]Sample{{Entry: "a", Bytes: 1010 * 100, Sessions: 100}})
	if err != nil || res.Changed || !res.Entries[0].Measured {
		t.Fatalf("in band update = %+v, %v", res, err)
	}
	_ = s
}

func TestMixOfEntries(t *testing.T) {
	// Weights 3 and 1 are normalized. One entry is 2x off and the other is exact.
	s, err := New(Config{ObjectiveBps: 8e6, MaxStep: 10, Entries: []Entry{
		{ID: "web", Weight: 3, ExpectedBytes: 1000},
		{ID: "dns", Weight: 1, ExpectedBytes: 200},
	}})
	if err != nil {
		t.Fatal(err)
	}
	// Compiled: 8e6 / (8 * (0.75*1000 + 0.25*200)) = 1250 sessions per second.
	if got := s.BaseSessionsPerSecond(); math.Abs(got-1250) > 1e-9 {
		t.Fatalf("base rate = %v, want 1250", got)
	}
	_, res, err := s.Update([]Sample{{Entry: "web", Bytes: 2000 * 100, Sessions: 100}, {Entry: "dns", Bytes: 200 * 50, Sessions: 50}})
	if err != nil {
		t.Fatal(err)
	}
	// Measured mean: 0.75*2000 + 0.25*200 = 1550, so factor 800/1550.
	if want := 800.0 / 1550; math.Abs(res.Factor-want) > 1e-9 {
		t.Fatalf("factor = %v, want %v", res.Factor, want)
	}
	if got := res.SessionsPerSecond * 8 * 1550; math.Abs(got-8e6) > 1e-3 {
		t.Fatalf("implied throughput = %v, want 8e6", got)
	}
}

func TestFactorBound(t *testing.T) {
	s, err := New(Config{ObjectiveBps: objective, MaxStep: 10, MaxFactor: 8, Entries: []Entry{{ID: "a", Weight: 1, ExpectedBytes: 10}}})
	if err != nil {
		t.Fatal(err)
	}
	_, res, err := s.Update([]Sample{{Entry: "a", Bytes: 1e6 * 100, Sessions: 100}})
	if err != nil || res.Factor != 1.0/8 {
		t.Fatalf("factor = %v, %v, want 1/8", res.Factor, err)
	}
}

func TestDeterministicAndImmutable(t *testing.T) {
	a, _ := New(Config{ObjectiveBps: objective, Entries: []Entry{{ID: "a", Weight: 1, ExpectedBytes: 1000}}})
	in := []Sample{{Entry: "a", Bytes: 4000 * 50, Sessions: 50}}
	_, r1, _ := a.Update(in)
	_, r2, _ := a.Update(in) // the receiver is unchanged, so the result repeats
	if r1.Factor != r2.Factor || r1.Target != r2.Target || a.Factor() != 1 {
		t.Fatalf("results differ: %+v %+v", r1, r2)
	}
}

func TestInvalidInput(t *testing.T) {
	ok := Entry{ID: "a", Weight: 1, ExpectedBytes: 10}
	for name, cfg := range map[string]Config{
		"no objective":     {Entries: []Entry{ok}},
		"nan objective":    {ObjectiveBps: math.NaN(), Entries: []Entry{ok}},
		"no entries":       {ObjectiveBps: 1},
		"no estimate":      {ObjectiveBps: 1, Entries: []Entry{{ID: "a", Weight: 1}}},
		"zero weight":      {ObjectiveBps: 1, Entries: []Entry{{ID: "a", ExpectedBytes: 1}}},
		"duplicate":        {ObjectiveBps: 1, Entries: []Entry{ok, ok}},
		"band above step":  {ObjectiveBps: 1, Entries: []Entry{ok}, MaxStep: 0.1, DeadBand: 0.2},
		"forget one":       {ObjectiveBps: 1, Entries: []Entry{ok}, Forget: 1},
		"max factor one":   {ObjectiveBps: 1, Entries: []Entry{ok}, MaxFactor: 1},
		"negative min":     {ObjectiveBps: 1, Entries: []Entry{ok}, MinSessions: -1},
		"negative step":    {ObjectiveBps: 1, Entries: []Entry{ok}, MaxStep: -1},
		"negative band":    {ObjectiveBps: 1, Entries: []Entry{ok}, DeadBand: -1},
		"infinite weights": {ObjectiveBps: 1, Entries: []Entry{{ID: "a", Weight: math.Inf(1), ExpectedBytes: 1}}},
	} {
		if _, err := New(cfg); !errors.Is(err, ErrConfig) {
			t.Errorf("%s: err = %v, want ErrConfig", name, err)
		}
	}
	s, _ := New(Config{ObjectiveBps: 1, Entries: []Entry{ok}})
	for name, in := range map[string][]Sample{
		"unknown":         {{Entry: "b", Bytes: 1, Sessions: 1}},
		"twice":           {{Entry: "a"}, {Entry: "a"}},
		"negative":        {{Entry: "a", Bytes: -1, Sessions: 1}},
		"nan":             {{Entry: "a", Bytes: math.NaN(), Sessions: 1}},
		"bytes no sesion": {{Entry: "a", Bytes: 5}},
	} {
		if _, _, err := s.Update(in); !errors.Is(err, ErrMeasurement) {
			t.Errorf("%s: err = %v, want ErrMeasurement", name, err)
		}
	}
	if _, _, err := (State{}).Update(nil); !errors.Is(err, ErrConfig) {
		t.Errorf("zero state: err = %v, want ErrConfig", err)
	}
}

func TestNoiseFreeApproachIsMonotone(t *testing.T) {
	for _, estimate := range []float64{0.25, 0.5, 2, 4} {
		factors, ratios := simulate(t, 1, estimate, 0, 0, 40)
		// Without noise the factor moves in one direction and stops.
		down := estimate < 1
		prev := 1.0
		for i, f := range factors {
			if (down && f > prev) || (!down && f < prev) {
				t.Fatalf("estimate %vx update %d: factor moved from %v to %v against the approach", estimate, i, prev, f)
			}
			prev = f
		}
		if last := ratios[len(ratios)-1]; math.Abs(last-1) > 0.015 {
			t.Fatalf("estimate %vx: achieved %.4f of the objective, want within the dead band", estimate, last)
		}
	}
}
