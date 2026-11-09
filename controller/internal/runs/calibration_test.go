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

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs/calibrate"
)

// The throughput tests run a 8 Mbit/s objective on a scenario that the
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

// simAgent drives a fake agent like an engine whose sessions carry trueBytes
// bytes each. It runs the rate of the plan in simulated time, reports the
// counters of the sessions that it completed, and takes new rates from
// UpdateRates, so the loop between the controller and the agent closes.
type simAgent struct {
	fake *fakeagent.Agent

	mu       sync.Mutex
	rates    map[string]float64 // sessions per second of the whole agent, by mix entry
	acc      float64            // sessions that are due but not yet whole
	sessions uint64
	bytes    uint64
	simTime  float64
	updates  []*agentpb.UpdateRatesCommand
	stop     chan struct{}
	done     chan struct{}
}

// startSim returns a simulated agent for fake, which must have a prepared plan,
// attaches it to rm for UpdateRates, and starts its traffic. The test ends it with Stop.
func startSim(t *testing.T, rm *rateManager, fake *fakeagent.Agent) *simAgent {
	t.Helper()
	s := &simAgent{fake: fake, rates: map[string]float64{}, stop: make(chan struct{}), done: make(chan struct{})}
	var plan *agentpb.Plan
	for _, c := range fake.Commands() {
		if p := c.GetPrepare(); p != nil {
			plan = p.GetPlan()
		}
	}
	if plan == nil {
		t.Fatalf("agent %s has no plan", fake.ID())
	}
	for _, w := range plan.GetWorkers() {
		for _, r := range w.GetRates() {
			s.rates[r.GetMixEntry()] += r.GetSessionsPerSecond()
		}
	}
	rm.attach(fake.ID(), s)
	go s.run()
	t.Cleanup(s.Stop)
	return s
}

// Stop ends the traffic. It may be called more than once.
func (s *simAgent) Stop() {
	s.mu.Lock()
	select {
	case <-s.stop:
	default:
		close(s.stop)
	}
	s.mu.Unlock()
	<-s.done
}

func (s *simAgent) run() {
	defer close(s.done)
	tick := time.NewTicker(simTick)
	defer tick.Stop()
	for {
		select {
		case <-s.stop:
			return
		case <-tick.C:
		}
		s.mu.Lock()
		dt := simTick.Seconds() * simSpeed
		for _, r := range s.rates {
			s.acc += r * dt
		}
		whole := uint64(s.acc)
		s.acc -= float64(whole)
		s.sessions += whole
		s.bytes += whole * trueBytes
		s.simTime += dt
		sessions, bytes := s.sessions, s.bytes
		s.mu.Unlock()
		client := scenario("client")
		_, _ = s.fake.Snapshot(
			labeled("sessions.attempted", sessions, client),
			labeled("sessions.succeeded", sessions, client),
			labeled("bytes.tx", bytes/2, client),
			labeled("bytes.rx", bytes-bytes/2, client),
		)
	}
}

// apply is the agent's handling of UpdateRates: all entries or none.
func (s *simAgent) apply(cmd *agentpb.UpdateRatesCommand) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, r := range cmd.GetRates() {
		if _, ok := s.rates[r.GetMixEntry()]; !ok {
			return fmt.Errorf("mix entry %q is not a mix entry of the plan", r.GetMixEntry())
		}
	}
	for _, r := range cmd.GetRates() {
		s.rates[r.GetMixEntry()] = r.GetSessionsPerSecond()
	}
	s.updates = append(s.updates, cmd)
	return nil
}

// total returns the session rate of the whole agent.
func (s *simAgent) total() float64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	var sum float64
	for _, r := range s.rates {
		sum += r
	}
	return sum
}

// mark returns the bytes and the simulated seconds so far.
func (s *simAgent) mark() (bytes uint64, simTime float64) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.bytes, s.simTime
}

func (s *simAgent) updateCommands() []*agentpb.UpdateRatesCommand {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]*agentpb.UpdateRatesCommand(nil), s.updates...)
}

// rateManager is an agent manager that also sends UpdateRates, which it hands
// to the simulated agent of the target. The agent manager of the controller
// does not send the command yet.
type rateManager struct {
	*agents.Manager
	mu    sync.Mutex
	sims  map[string]*simAgent
	calls map[string]int
}

func newRateManager(m *agents.Manager) *rateManager {
	return &rateManager{Manager: m, sims: map[string]*simAgent{}, calls: map[string]int{}}
}

func (m *rateManager) attach(id string, s *simAgent) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.sims[id] = s
}

func (m *rateManager) callsTo(id string) int {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.calls[id]
}

func (m *rateManager) UpdateRates(_ context.Context, id string, cmd *agentpb.UpdateRatesCommand) (*agentpb.UpdateRatesResult, error) {
	m.mu.Lock()
	m.calls[id]++
	s := m.sims[id]
	m.mu.Unlock()
	if s == nil {
		return nil, fmt.Errorf("no simulated agent %s", id)
	}
	if err := s.apply(cmd); err != nil {
		return nil, err
	}
	return &agentpb.UpdateRatesResult{}, nil
}

// engineCaps returns fake agent capabilities with the engine version.
func engineCaps(engine string) *agentpb.Capabilities {
	c := fakeagent.DefaultCapabilities()
	c.EngineApi = engine
	return c
}

// throughputHarness starts a harness whose configuration "thr" has a
// throughput objective, whose catalog carries the byte estimate, and whose
// calibration runs a window of 20 ms. It returns the rate manager and the
// recorder of its log.
func throughputHarness(t *testing.T, cfgBody string, cal runs.CalibrationConfig) (*harness, *rateManager, *logRecorder) {
	t.Helper()
	var rm *rateManager
	rec := &logRecorder{}
	h := newHarness(t, func(c *runs.Config) {
		c.Store = memStore{"thr": cfgBody}
		c.Catalog = byteCatalog(expectedBytes)
		c.Calibration = cal
		c.Logger = slog.New(rec)
		mgr, ok := c.Agents.(*agents.Manager)
		if !ok {
			t.Fatalf("harness agent manager is %T", c.Agents)
		}
		rm = newRateManager(mgr)
		c.Agents = rm
	})
	return h, rm, rec
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
	a, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: id, Dial: h.dial, Capabilities: engineCaps(engine)})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	return a
}

// measuredBps returns the throughput that the agents moved over about 200 ms
// of real time, which is 20 s of simulated traffic, in bits per second of
// simulated time.
func measuredBps(sims ...*simAgent) float64 {
	b0 := make([]uint64, len(sims))
	t0 := make([]float64, len(sims))
	for i, s := range sims {
		b0[i], t0[i] = s.mark()
	}
	time.Sleep(200 * time.Millisecond)
	var bits, secs float64
	for i, s := range sims {
		b1, t1 := s.mark()
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
	h, rm, rec := throughputHarness(t, throughputConfig, fastCalibration())
	a := startAgent(t, h, "agent-a", "0.2.1")
	run := h.startThroughputRun(t, a)
	sim := startSim(t, rm, a)

	// At the compiled rate the sessions move 4 times the objective.
	if got := sim.total() * trueBytes * 8; math.Abs(got/objectiveBps-4) > 1e-9 {
		t.Fatalf("compiled rate gives %v bit/s, want 4 times the objective", got)
	}
	waitFor(t, "the corrected rate", func() bool {
		return math.Abs(sim.total()*trueBytes*8/objectiveBps-1) <= 0.02
	})
	if got := measuredBps(sim); math.Abs(got/objectiveBps-1) > 0.05 {
		t.Errorf("throughput %.0f bit/s is not within 5%% of the objective %.0f bit/s", got, objectiveBps)
	}

	// The agent got whole agent totals, and the run records the steps.
	cmds := sim.updateCommands()
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
	sim.Stop()
	if err := a.Finish(); err != nil {
		t.Fatal(err)
	}
	end := h.wait(t, run.ID)
	if end.State != runs.Completed || len(end.Calibration) < len(steps) {
		t.Errorf("run ended %s with %d steps", end.State, len(end.Calibration))
	}
	calls := rm.callsTo(a.ID())
	time.Sleep(3 * calWin)
	if n := rm.callsTo(a.ID()); n != calls {
		t.Errorf("UpdateRates reached the agent after the run ended: %d calls, then %d", calls, n)
	}
}

// TestCalibrationSkipsAgentsWithoutUpdateRates checks that an agent with
// engine_api 0.1.0 never receives the command, that the orchestrator logs
// that once, and that the agent with 0.2.1 is corrected.
func TestCalibrationSkipsAgentsWithoutUpdateRates(t *testing.T) {
	h, rm, rec := throughputHarness(t, throughputConfig, fastCalibration())
	newer := startAgent(t, h, "agent-new", "0.2.1")
	older := startAgent(t, h, "agent-old", "0.1.0")
	run := h.startThroughputRun(t, newer, older)
	simNew, simOld := startSim(t, rm, newer), startSim(t, rm, older)
	baseOld := simOld.total()

	waitFor(t, "the corrected rate of the new agent", func() bool {
		r, _ := h.orch.Get(run.ID)
		return len(r.Calibration) >= 4
	})
	time.Sleep(5 * calWin)
	if n := rm.callsTo(older.ID()); n != 0 {
		t.Errorf("the old agent received %d UpdateRates commands", n)
	}
	if got := simOld.total(); got != baseOld {
		t.Errorf("the old agent runs %v sessions per second, compiled %v", got, baseOld)
	}
	if len(simNew.updateCommands()) == 0 {
		t.Error("the new agent received no UpdateRates")
	}
	if n := rec.count("agent has no UpdateRates, so its rates stay uncalibrated"); n != 1 {
		t.Errorf("logged %d times that the old agent has no UpdateRates, want once", n)
	}
	simNew.Stop()
	simOld.Stop()
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
			h, rm, rec := throughputHarness(t, tc.config, tc.cal)
			a := startAgent(t, h, "agent-a", "0.2.1")
			run := h.startThroughputRun(t, a)
			sim := startSim(t, rm, a)
			base := sim.total()
			time.Sleep(10 * calWin)
			r, err := h.orch.Get(run.ID)
			if err != nil {
				t.Fatal(err)
			}
			if n := rm.callsTo(a.ID()); n != 0 || len(r.Calibration) != 0 || sim.total() != base {
				t.Errorf("%d UpdateRates, %d steps, rate %v from %v", n, len(r.Calibration), sim.total(), base)
			}
			if n := rec.count("throughput calibration started"); n != 0 {
				t.Errorf("calibration started %d times", n)
			}
			sim.Stop()
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
