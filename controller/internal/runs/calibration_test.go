// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"context"
	"fmt"
	"log/slog"
	"math"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs/calibrate"
)

// The throughput tests run on the real agent manager and fake agents that
// simulate traffic (fakeagent.Traffic). They run a 8 Mbit/s objective on a scenario that the
// manifest estimates at 10 kB per session. The compiled rate is therefore 100
// sessions per second, and sessions that really carry four times as much move
// four times the objective until the calibration corrects the rate to 25.
const (
	objectiveBps  = 8e6
	expectedBytes = 10000
	trueBytes     = 4 * expectedBytes
	// simSpeed makes one real second of a simulated agent count as 100
	// seconds of traffic, so that a window of 20 ms holds hundreds of
	// sessions and the calibration converges within a second.
	simSpeed = 100
	simTick  = 5 * time.Millisecond
	calWin   = 20 * time.Millisecond
)

// throughputConfig is testConfig with a throughput objective.
var throughputConfig = strings.Replace(testConfig,
	"objective: { type: sessions_per_second, value: 1000 }",
	fmt.Sprintf("objective: { type: throughput_bps, value: %d }", int(objectiveBps)), 1)

// byteCatalog serves the http1/web_get scenario with a bytes per session
// estimate, the manifest field expected_bytes.
type byteCatalog float64

func (b byteCatalog) Scenarios(context.Context) ([]compiler.Scenario, error) {
	return []compiler.Scenario{{
		Module:          compiler.ModuleRef{Name: "http1", Version: "0.1.0", Digest: "sha256:" + strings.Repeat("0", 64)},
		Name:            "web_get",
		Source:          "name: web_get\n",
		BytesPerSession: float64(b),
	}}, nil
}

// trafficOptions makes a fake agent simulate sessions that carry trueBytes
// bytes each. The agent runs the rate of its plan in simulated time, reports
// the counters of the sessions that it completed, and takes new rates from
// UpdateRates, so the loop between the controller and the agent closes on the
// real agent manager.
func trafficOptions() *fakeagent.Traffic {
	return &fakeagent.Traffic{BytesPerSession: trueBytes, Speed: simSpeed, Tick: simTick}
}

// totalRate returns the session rate that the agent runs, over every mix entry.
func totalRate(a *fakeagent.Agent) float64 {
	var sum float64
	for _, r := range a.Rates() {
		sum += r
	}
	return sum
}

// compiledRate returns the session rate of the plan that the agent prepared.
func compiledRate(a *fakeagent.Agent) float64 {
	var sum float64
	for _, c := range a.Commands() {
		for _, w := range c.GetPrepare().GetPlan().GetWorkers() {
			for _, r := range w.GetRates() {
				sum += r.GetSessionsPerSecond()
			}
		}
	}
	return sum
}

// updateCommands returns the UpdateRates commands that the agent received.
func updateCommands(a *fakeagent.Agent) []*agentpb.UpdateRatesCommand {
	var out []*agentpb.UpdateRatesCommand
	for _, c := range a.Commands() {
		if u := c.GetUpdateRates(); u != nil {
			out = append(out, u)
		}
	}
	return out
}

// engineCaps returns fake agent capabilities with the engine version.
func engineCaps(engine string) *agentpb.Capabilities {
	c := fakeagent.DefaultCapabilities()
	c.EngineApi = engine
	return c
}

// throughputHarness starts a harness whose configuration "thr" has a
// throughput objective, whose catalog carries the byte estimate, and whose
// calibration runs a window of 20 ms. The orchestrator runs on the real agent
// manager. It returns the recorder of its log.
func throughputHarness(t *testing.T, cfgBody string, cal runs.CalibrationConfig) (*harness, *logRecorder) {
	t.Helper()
	rec := &logRecorder{}
	h := newHarness(t, func(c *runs.Config) {
		c.Store = memStore{"thr": cfgBody}
		c.Catalog = byteCatalog(expectedBytes)
		c.Calibration = cal
		c.Logger = slog.New(rec)
	})
	return h, rec
}

// fastCalibration is the calibration of the tests: short windows, and the
// tuning that the defaults of calibrate give.
func fastCalibration() runs.CalibrationConfig {
	return runs.CalibrationConfig{Window: calWin, Tuning: calibrate.Config{MinSessions: 20}}
}

// logRecorder is a slog.Handler that keeps the messages.
type logRecorder struct {
	mu   sync.Mutex
	msgs []string
}

func (*logRecorder) Enabled(context.Context, slog.Level) bool { return true }

func (l *logRecorder) Handle(_ context.Context, r slog.Record) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.msgs = append(l.msgs, r.Message)
	return nil
}

func (l *logRecorder) WithAttrs([]slog.Attr) slog.Handler { return l }
func (l *logRecorder) WithGroup(string) slog.Handler      { return l }

func (l *logRecorder) count(message string) int {
	l.mu.Lock()
	defer l.mu.Unlock()
	n := 0
	for _, m := range l.msgs {
		if m == message {
			n++
		}
	}
	return n
}

// startThroughputRun creates the run "thr" on the agents and waits until it runs.
func (h *harness) startThroughputRun(t *testing.T, fakes ...*fakeagent.Agent) runs.Run {
	t.Helper()
	ids := make([]string, len(fakes))
	for i, f := range fakes {
		ids[i] = f.ID()
	}
	run, err := h.orch.Create(testContext(t), runs.StartRequest{ConfigID: "thr", Agents: ids})
	if err != nil {
		t.Fatal(err)
	}
	h.waitRunning(t, run.ID, fakes...)
	return run
}

func startAgent(t *testing.T, h *harness, id, engine string) *fakeagent.Agent {
	t.Helper()
	a, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: id, Dial: h.dial, Capabilities: engineCaps(engine), Traffic: trafficOptions()})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	return a
}

// measuredBps returns the throughput that the agents moved over about 200 ms
// of real time, which is 20 s of simulated traffic, in bits per second of
// simulated time.
func measuredBps(agents ...*fakeagent.Agent) float64 {
	b0 := make([]uint64, len(agents))
	t0 := make([]float64, len(agents))
	for i, a := range agents {
		b0[i], t0[i] = a.TrafficTotals()
	}
	time.Sleep(200 * time.Millisecond)
	var bits, secs float64
	for i, a := range agents {
		b1, t1 := a.TrafficTotals()
		bits += float64(b1 - b0[i])
		secs = math.Max(secs, t1-t0[i])
	}
	return bits * 8 / secs
}

// TestThroughputCalibrationReachesObjective is the acceptance test of CTL-24:
// sessions that carry four times the expected bytes move four times the
// objective at the compiled rate, and the calibration brings the realized
// throughput within 5 percent of the objective.
func TestThroughputCalibrationReachesObjective(t *testing.T) {
	h, rec := throughputHarness(t, throughputConfig, fastCalibration())
	a := startAgent(t, h, "agent-a", "0.2.1")
	run := h.startThroughputRun(t, a)
	// At the compiled rate of the plan the sessions move 4 times the objective.
	if got := compiledRate(a) * trueBytes * 8; math.Abs(got/objectiveBps-4) > 1e-9 {
		t.Fatalf("compiled rate gives %v bit/s, want 4 times the objective", got)
	}
	waitFor(t, "the corrected rate", func() bool {
		return math.Abs(totalRate(a)*trueBytes*8/objectiveBps-1) <= 0.02
	})
	if got := measuredBps(a); math.Abs(got/objectiveBps-1) > 0.05 {
		t.Errorf("throughput %.0f bit/s is not within 5%% of the objective %.0f bit/s", got, objectiveBps)
	}

	// The agent got whole agent totals, and the run records the steps.
	cmds := updateCommands(a)
	if len(cmds) == 0 {
		t.Fatal("the agent received no UpdateRates")
	}
	for _, c := range cmds {
		if c.GetPlanId() != run.ID+"/"+a.ID() {
			t.Errorf("command plan id %q", c.GetPlanId())
		}
		if len(c.GetRates()) != 1 || c.GetRates()[0].GetMixEntry() != "web_get" {
			t.Errorf("command rates %v, want one for web_get", c.GetRates())
		}
	}
	rec1, err := h.orch.Get(run.ID)
	if err != nil {
		t.Fatal(err)
	}
	steps := rec1.Calibration
	if len(steps) < 3 {
		t.Fatalf("%d calibration steps, want the 4x correction to take several", len(steps))
	}
	prev := 1.0
	for i, s := range steps {
		if s.Factor >= prev || s.Factor < prev/1.5-1e-9 {
			t.Errorf("step %d moved the factor from %v to %v, want a decrease of at most 1.5x", i, prev, s.Factor)
		}
		prev = s.Factor
	}
	if math.Abs(prev-0.25) > 0.01 {
		t.Errorf("final factor %v, want 0.25", prev)
	}
	if n := rec.count("throughput calibration step"); n != len(steps) {
		t.Errorf("%d log lines for %d steps", n, len(steps))
	}

	// The calibration ends with the run, and the run completes as usual.
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)
	if end.State != runs.Completed || len(end.Calibration) < len(steps) {
		t.Errorf("run ended %s with %d steps", end.State, len(end.Calibration))
	}
	calls := len(updateCommands(a))
	time.Sleep(3 * calWin)
	if n := len(updateCommands(a)); n != calls {
		t.Errorf("UpdateRates reached the agent after the run ended: %d calls, then %d", calls, n)
	}
}

// TestCalibrationSkipsAgentsWithoutUpdateRates checks that an agent with
// engine_api 0.1.0 never receives the command, that the orchestrator logs
// that once, and that the agent with 0.2.1 is corrected.
func TestCalibrationSkipsAgentsWithoutUpdateRates(t *testing.T) {
	h, rec := throughputHarness(t, throughputConfig, fastCalibration())
	newer := startAgent(t, h, "agent-new", "0.2.1")
	older := startAgent(t, h, "agent-old", "0.1.0")
	run := h.startThroughputRun(t, newer, older)
	baseOld := totalRate(older)

	waitFor(t, "the corrected rate of the new agent", func() bool {
		r, _ := h.orch.Get(run.ID)
		return len(r.Calibration) >= 4
	})
	time.Sleep(5 * calWin)
	if n := len(updateCommands(older)); n != 0 {
		t.Errorf("the old agent received %d UpdateRates commands", n)
	}
	if got := totalRate(older); got != baseOld {
		t.Errorf("the old agent runs %v sessions per second, compiled %v", got, baseOld)
	}
	if len(updateCommands(newer)) == 0 {
		t.Error("the new agent received no UpdateRates")
	}
	if n := rec.count("agent has no UpdateRates, so its rates stay uncalibrated"); n != 1 {
		t.Errorf("logged %d times that the old agent has no UpdateRates, want once", n)
	}
	if _, err := h.orch.Stop(run.ID, true); err != nil {
		t.Fatal(err)
	}
	h.wait(t, run.ID)
}

// TestCalibrationLeavesOtherObjectivesAlone checks that a sessions_per_second
// objective gets no calibration, and that neither does a throughput objective
// with the calibration turned off.
func TestCalibrationLeavesOtherObjectivesAlone(t *testing.T) {
	for name, tc := range map[string]struct {
		config string
		cal    runs.CalibrationConfig
	}{
		"sessions per second": {testConfig, fastCalibration()},
		"calibration off":     {throughputConfig, runs.CalibrationConfig{Window: -1}},
	} {
		t.Run(name, func(t *testing.T) {
			h, rec := throughputHarness(t, tc.config, tc.cal)
			a := startAgent(t, h, "agent-a", "0.2.1")
			run := h.startThroughputRun(t, a)
			base := totalRate(a)
			time.Sleep(10 * calWin)
			r, err := h.orch.Get(run.ID)
			if err != nil {
				t.Fatal(err)
			}
			if n := len(updateCommands(a)); n != 0 || len(r.Calibration) != 0 || totalRate(a) != base {
				t.Errorf("%d UpdateRates, %d steps, rate %v from %v", n, len(r.Calibration), totalRate(a), base)
			}
			if n := rec.count("throughput calibration started"); n != 0 {
				t.Errorf("calibration started %d times", n)
			}
			if _, err := h.orch.Stop(run.ID, true); err != nil {
				t.Fatal(err)
			}
			h.wait(t, run.ID)
		})
	}
}

// TestCalibrationNeedsARateUpdater checks that a run on an agent manager that
// cannot send UpdateRates keeps its compiled rates, says so in the log, and
// ends as usual.
func TestCalibrationNeedsARateUpdater(t *testing.T) {
	seen := make(chan struct{})
	h := newHarness(t, func(c *runs.Config) {
		c.Store = memStore{"thr": throughputConfig}
		c.Catalog = byteCatalog(expectedBytes)
		c.Calibration = fastCalibration()
		// Hide any UpdateRates method of the manager behind the interface.
		c.Agents = struct{ runs.AgentManager }{c.Agents}
		c.Logger = slog.New(&messageHandler{message: "throughput calibration is off: the agent manager cannot send UpdateRates", seen: seen})
	})
	a := startAgent(t, h, "agent-a", "0.2.1")
	run := h.startThroughputRun(t, a)
	select {
	case <-seen:
	case <-testContext(t).Done():
		t.Fatal("the orchestrator did not say that the calibration is off")
	}
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	if end := h.wait(t, run.ID); end.State != runs.Completed || len(end.Calibration) != 0 {
		t.Fatalf("run ended %s with %d steps", end.State, len(end.Calibration))
	}
}
