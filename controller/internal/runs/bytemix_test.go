// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"context"
	"log/slog"
	"math"
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs/calibrate"
)

// The byte mix tests run the CTL-12 acceptance on the real agent manager and
// fake agents. The mix has three scenarios whose sessions carry 20 kB, 200 kB,
// and 200 B, a factor of 1000 apart, against manifest estimates that are off
// by factors of 2, 4, and 5. At the compiled session shares the heavy scenario
// takes 84 percent of the bytes, and the weights ask for 39.9.
const (
	mixObjectiveBps = 80e6
	// mixSimSpeed and mixWindow make one control window span one simulated
	// second: a fake agent runs 20 simulated seconds per real second, and the
	// window is 50 ms of real time.
	mixSimSpeed = 20
	mixWindow   = 50 * time.Millisecond
)

// The true bytes per session of each scenario, by scenario name.
var mixTrueBytes = map[string]float64{"web_browse": 20e3, "web_get": 200e3, "lookup": 200}

// The byte weights of the test configuration, as shares.
var mixWeights = map[string]float64{"web_browse": 0.6, "web_get": 0.399, "lookup": 0.001}

// mixConfig is a three scenario test with the given weight basis and objective.
func mixConfig(basis, objective string) string {
	s := strings.Replace(testConfig, "basis: sessions", "basis: "+basis, 1)
	s = strings.Replace(s, "- { scenario: http1/web_get, weight: 1 }",
		"- { scenario: http1/web_browse, weight: 60 }\n    - { scenario: http1/web_get, weight: 39.9 }\n    - { scenario: dns/lookup, weight: 0.1 }", 1)
	return strings.Replace(s, "objective: { type: sessions_per_second, value: 1000 }", "objective: "+objective, 1)
}

var mixThroughput = mixConfig("bytes", "{ type: throughput_bps, value: 80000000 }")

// mixCatalog serves the three scenarios with their manifest estimates, which
// are off by factors of 2, 4, and 5, or with the true bytes per session when
// exact is set.
type mixCatalog struct{ exact bool }

func (c mixCatalog) Scenarios(context.Context) ([]compiler.Scenario, error) {
	browse, get, lookup := 40e3, 50e3, 1000.0
	if c.exact {
		browse, get, lookup = mixTrueBytes["web_browse"], mixTrueBytes["web_get"], mixTrueBytes["lookup"]
	}
	digest := "sha256:" + strings.Repeat("0", 64)
	http1 := compiler.ModuleRef{Name: "http1", Version: "0.1.0", Digest: digest}
	dns := compiler.ModuleRef{Name: "dns", Version: "0.1.0", Digest: digest}
	return []compiler.Scenario{
		{Module: http1, Name: "web_browse", Source: "name: web_browse\n", BytesPerSession: browse},
		{Module: http1, Name: "web_get", Source: "name: web_get\n", BytesPerSession: get},
		{Module: dns, Name: "lookup", Source: "name: lookup\n", BytesPerSession: lookup},
	}, nil
}

func mixHarness(t *testing.T, cfgBody string, cal runs.CalibrationConfig, cat mixCatalog) (*harness, *logRecorder) {
	t.Helper()
	rec := &logRecorder{}
	h := newHarness(t, func(c *runs.Config) {
		c.Store = memStore{"thr": cfgBody}
		c.Catalog = cat
		c.Calibration = cal
		c.Logger = slog.New(rec)
	})
	return h, rec
}

func mixCalibration(off bool) runs.CalibrationConfig {
	return runs.CalibrationConfig{Window: mixWindow, Tuning: calibrate.Config{MinSessions: 20}, ByteMixOff: off}
}

func startMixAgent(t *testing.T, h *harness, id string) *fakeagent.Agent {
	t.Helper()
	a, err := fakeagent.Start(testContext(t), fakeagent.Options{
		ID: id, Dial: h.dial, Capabilities: engineCaps("0.2.1"),
		Traffic: &fakeagent.Traffic{BytesPerSession: 1000, BytesByScenario: mixTrueBytes, Speed: mixSimSpeed, Tick: simTick},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	return a
}

// shareSample is the worst distance between a byte share and its weight, in
// points, over an interval that ended at simulated time sim.
type shareSample struct{ sim, errPP, bps float64 }

// trackShares measures the byte share of every scenario over intervals of at
// least two simulated seconds, until the agent has run for until simulated
// seconds.
func trackShares(t *testing.T, a *fakeagent.Agent, until float64) []shareSample {
	t.Helper()
	prev := a.ScenarioBytes()
	prevBytes, prevSim := a.TrafficTotals()
	var out []shareSample
	deadline := time.Now().Add(60 * time.Second)
	for time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
		total, sim := a.TrafficTotals()
		if sim >= until {
			return out
		}
		if sim-prevSim < 2 {
			continue
		}
		cur := a.ScenarioBytes()
		var worst float64
		var sum uint64
		for name := range mixWeights {
			sum += cur[name] - prev[name]
		}
		if sum > 0 {
			for name, w := range mixWeights {
				share := float64(cur[name]-prev[name]) / float64(sum)
				worst = math.Max(worst, 100*math.Abs(share-w))
			}
			out = append(out, shareSample{sim: sim, errPP: worst, bps: float64(total-prevBytes) * 8 / (sim - prevSim)})
		}
		prev, prevBytes, prevSim = cur, total, sim
	}
	t.Fatalf("the agent reached only %v of %v simulated seconds", prevSim, until)
	return nil
}

// TestByteMixConvergesAndStays is the acceptance test of CTL-12 for wrong
// manifest estimates: the compiler converts the byte weights with them, so the
// run starts at the wrong shares, the byte shares reach 2 points of the
// weights within 30 simulated seconds and stay there, while the throughput
// stays on the objective.
func TestByteMixConvergesAndStays(t *testing.T) {
	h, rec := mixHarness(t, mixThroughput, mixCalibration(false), mixCatalog{})
	a := startMixAgent(t, h, "agent-a")
	run := h.startThroughputRun(t, a)

	samples := trackShares(t, a, 120)
	if len(samples) < 20 {
		t.Fatalf("%d share samples, want at least 20", len(samples))
	}
	if samples[0].errPP < 10 {
		t.Errorf("the first interval is already within %.1f points, so the test proves nothing", samples[0].errPP)
	}
	converged := 0.0
	for _, s := range samples {
		if s.sim >= 30 && s.errPP > 2 {
			t.Errorf("at %.0f simulated seconds the byte shares are %.2f points off, want at most 2 from 30 s on", s.sim, s.errPP)
		}
		if s.errPP <= 2 && converged == 0 {
			converged = s.sim
		}
	}
	t.Logf("first interval within 2 points ends at %.0f simulated seconds", converged)

	// The throughput calibration still reaches its objective with the
	// multipliers on top.
	last := samples[len(samples)-1]
	if math.Abs(last.bps/mixObjectiveBps-1) > 0.05 {
		t.Errorf("throughput %.0f bit/s is not within 5%% of the objective %.0f bit/s", last.bps, mixObjectiveBps)
	}

	// One loop sends the rates: every command carries the whole agent total of
	// every entry, and the run records the multipliers.
	cmds := updateCommands(a)
	if len(cmds) == 0 {
		t.Fatal("the agent received no UpdateRates")
	}
	for _, c := range cmds {
		if len(c.GetRates()) != 3 {
			t.Errorf("command carries %d rates, want one per mix entry", len(c.GetRates()))
		}
	}
	rec1, err := h.orch.Get(run.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(rec1.Calibration) == 0 {
		t.Fatal("the run recorded no calibration step")
	}
	m := rec1.Calibration[len(rec1.Calibration)-1].Multipliers
	// The compiled session shares follow the estimates, so the multiplier of an
	// entry is its estimate over its true bytes per session, up to a common
	// factor: (50 kB / 200 kB) / (40 kB / 20 kB) for web_get over web_browse.
	if got, want := m["web_get"]/m["web_browse"], (50e3/mixTrueBytes["web_get"])/(40e3/mixTrueBytes["web_browse"]); math.Abs(got/want-1) > 0.05 {
		t.Errorf("multiplier ratio %v, want %v", got, want)
	}
	if rec.count("byte mix step") == 0 {
		t.Error("no log line for a byte mix step")
	}

	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	if end := h.wait(t, run.ID); end.State != runs.Completed {
		t.Errorf("run ended %s", end.State)
	}
}

// TestByteMixOnlyWhenAsked checks that a session based profile and a switched
// off byte mix leave the mix at its compiled session weights.
func TestByteMixOnlyWhenAsked(t *testing.T) {
	for name, tc := range map[string]struct {
		cfg string
		off bool
		// ratio is the compiled session rate of web_get over web_browse: the
		// weights themselves for the sessions basis, and the byte weights over
		// the manifest estimates (50 kB and 40 kB) for the bytes basis.
		ratio float64
	}{
		"sessions basis": {mixConfig("sessions", "{ type: throughput_bps, value: 80000000 }"), false, 39.9 / 60},
		"byte mix off":   {mixThroughput, true, (39.9 / 50e3) / (60 / 40e3)},
	} {
		t.Run(name, func(t *testing.T) {
			h, rec := mixHarness(t, tc.cfg, mixCalibration(tc.off), mixCatalog{})
			a := startMixAgent(t, h, "agent-a")
			h.startThroughputRun(t, a)
			samples := trackShares(t, a, 40)
			if len(samples) == 0 {
				t.Fatal("no share samples")
			}
			for _, s := range samples {
				if s.errPP < 10 {
					t.Fatalf("at %.0f simulated seconds the shares are within %.1f points without byte mix control", s.sim, s.errPP)
				}
			}
			if n := rec.count("byte mix step"); n != 0 {
				t.Errorf("%d byte mix steps", n)
			}
			for _, c := range updateCommands(a) {
				// The throughput calibration alone keeps the session ratios.
				var browse, get float64
				for _, r := range c.GetRates() {
					switch r.GetMixEntry() {
					case "web_browse":
						browse = r.GetSessionsPerSecond()
					case "web_get":
						get = r.GetSessionsPerSecond()
					}
				}
				if math.Abs(get/browse-tc.ratio) > 1e-6 {
					t.Errorf("session rates %v and %v leave the compiled ratio", browse, get)
				}
			}
		})
	}
}

// TestByteMixNeedsThroughputObjective checks that a byte basis with another
// objective keeps the compiled rates and says why.
func TestByteMixNeedsThroughputObjective(t *testing.T) {
	h, rec := mixHarness(t, mixConfig("bytes", "{ type: sessions_per_second, value: 1000 }"), mixCalibration(false), mixCatalog{})
	a := startMixAgent(t, h, "agent-a")
	h.startThroughputRun(t, a)
	time.Sleep(10 * mixWindow)
	if n := rec.count("byte weighted mix control is off: it needs a throughput_bps objective and the calibration loop, so the mix keeps its compiled session weights"); n != 1 {
		t.Errorf("%d log lines about the missing objective, want 1", n)
	}
	if n := len(updateCommands(a)); n != 0 {
		t.Errorf("%d UpdateRates commands, want none", n)
	}
}

// TestByteMixExactEstimatesStartOnTarget is the acceptance test of CTL-34: when
// the manifest estimates are exact, the compiler turns the byte weights into
// session shares that already give the byte weights, so the byte shares are
// within 2 points from the first window on, the first rates are the compiled
// ones, and the throughput stays on the objective.
func TestByteMixExactEstimatesStartOnTarget(t *testing.T) {
	h, _ := mixHarness(t, mixThroughput, mixCalibration(false), mixCatalog{exact: true})
	a := startMixAgent(t, h, "agent-a")
	h.startThroughputRun(t, a)

	samples := trackShares(t, a, 60)
	if len(samples) < 10 {
		t.Fatalf("%d share samples, want at least 10", len(samples))
	}
	for _, s := range samples {
		if s.errPP > 2 {
			t.Errorf("at %.0f simulated seconds the byte shares are %.2f points off, want at most 2 from the first window on", s.sim, s.errPP)
		}
	}
	last := samples[len(samples)-1]
	if math.Abs(last.bps/mixObjectiveBps-1) > 0.05 {
		t.Errorf("throughput %.0f bit/s is not within 5%% of the objective %.0f bit/s", last.bps, mixObjectiveBps)
	}
	// The first command keeps the compiled session ratio, the byte weight over
	// the estimate, because the multipliers start at 1.
	cmds := updateCommands(a)
	if len(cmds) == 0 {
		t.Fatal("the agent received no UpdateRates")
	}
	var browse, get float64
	for _, r := range cmds[0].GetRates() {
		switch r.GetMixEntry() {
		case "web_browse":
			browse = r.GetSessionsPerSecond()
		case "web_get":
			get = r.GetSessionsPerSecond()
		}
	}
	if want := (39.9 / mixTrueBytes["web_get"]) / (60 / mixTrueBytes["web_browse"]); math.Abs(get/browse/want-1) > 0.1 {
		t.Errorf("first session rates %v and %v are not at the compiled ratio %v", browse, get, want)
	}
}
