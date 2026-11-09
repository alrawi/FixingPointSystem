// SPDX-License-Identifier: GPL-3.0-only

package runs

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs/calibrate"
)

// DefaultCalibrationWindow is the measurement window of the throughput
// calibration. Decision D12 item 10 asks for 5 to 10 seconds.
const DefaultCalibrationWindow = 5 * time.Second

// maxCalibrationSteps bounds the factor history of a run. A run that changes
// its factor more often keeps the newest steps.
const maxCalibrationSteps = 256

// objectiveThroughput is the plan name of the throughput_bps objective.
const objectiveThroughput = "LOAD_OBJECTIVE_THROUGHPUT_BPS"

// labelScenario is the catalog label that names the scenario of a counter.
const labelScenario = "scenario"

// The first engine contract version that has the UpdateRates command (CON-05).
// An agent announces its version in Capabilities.engine_api.
const (
	updateRatesMajor = 0
	updateRatesMinor = 2
)

// RateUpdater sends UpdateRates (CON-05) to one agent and waits for the
// result. An AgentManager that also implements it lets the orchestrator
// calibrate throughput objectives. Without it, a run keeps its compiled rates
// and the orchestrator logs that once per run.
type RateUpdater interface {
	UpdateRates(ctx context.Context, id string, cmd *agentpb.UpdateRatesCommand) (*agentpb.UpdateRatesResult, error)
}

// CalibrationConfig configures the throughput calibration of the orchestrator.
type CalibrationConfig struct {
	// Window is the length of one measurement window. Zero means
	// DefaultCalibrationWindow, and a negative value turns the calibration
	// off. The window is real time, because the calibration runs beside the
	// agents, and the clock of the orchestrator only stamps the history.
	Window time.Duration
	// Tuning sets the tuning fields MaxStep, DeadBand, MinSessions, Forget,
	// and MaxFactor of calibrate.Config, and zero fields take the defaults of
	// that package. The orchestrator fills the objective and the entries.
	Tuning calibrate.Config
}

// CalibrationStep records one change of the calibration factor.
type CalibrationStep struct {
	// Time is the orchestrator clock time of the step.
	Time time.Time
	// Factor scales every compiled session rate of the run from this step on.
	Factor float64
	// Target is the factor that the estimates called for before the bounds
	// of the step limited the move.
	Target float64
	// SessionsPerSecond is the total session rate that Factor implies across
	// the mix.
	SessionsPerSecond float64
	// Entries holds the bytes per session estimate of each mix entry.
	Entries []calibrate.EntryResult
}

// calEntry is one mix entry of a calibration.
type calEntry struct {
	id       string
	scenario string
	weight   float64
}

// calTotals are the cumulative client side bytes and completed sessions of
// one scenario.
type calTotals struct {
	bytes    float64
	sessions float64
}

// calibrator is the calibration of one run. Only the calibration goroutine of
// the run touches it after the run starts.
type calibrator struct {
	state     calibrate.State
	window    time.Duration
	updater   RateUpdater
	entries   []calEntry
	scenarios map[string]bool
	// last holds the cumulative totals of the windows that were counted, by
	// scenario name.
	last map[string]calTotals
}

// newCalibrator returns the calibration of a run, or nil when the run needs
// none: when calibration is off, or when the objective is not throughput_bps.
// It reads the objective from the plans and the bytes per session estimates
// that the compiler used from the catalog.
func newCalibrator(cfg CalibrationConfig, plans []compiler.Plan, scenarios []compiler.Scenario) (*calibrator, error) {
	if cfg.Window < 0 || len(plans) == 0 || plans[0].Load == nil || plans[0].Load.Objective != objectiveThroughput {
		return nil, nil
	}
	if cfg.Window == 0 {
		cfg.Window = DefaultCalibrationWindow
	}
	var objective float64
	for _, p := range plans {
		if p.Load != nil {
			objective += p.Load.ObjectiveValue
		}
	}
	estimate := map[string]float64{}
	for _, s := range scenarios {
		estimate[s.Module.Name+"/"+s.Name] = s.BytesPerSession
	}
	tuned := cfg.Tuning
	tuned.ObjectiveBps = objective
	tuned.Entries = nil
	c := &calibrator{window: cfg.Window, scenarios: map[string]bool{}, last: map[string]calTotals{}}
	for _, m := range plans[0].Mix {
		bytes, ok := estimate[m.Scenario]
		if !ok {
			return nil, fmt.Errorf("mix entry %s: scenario %s is not in the catalog", m.ID, m.Scenario)
		}
		name := m.Scenario[strings.LastIndex(m.Scenario, "/")+1:]
		c.entries = append(c.entries, calEntry{id: m.ID, scenario: name, weight: m.Weight})
		c.scenarios[name] = true
		tuned.Entries = append(tuned.Entries, calibrate.Entry{ID: m.ID, Weight: m.Weight, ExpectedBytes: bytes})
	}
	state, err := calibrate.New(tuned)
	if err != nil {
		return nil, err
	}
	c.state = state
	return c, nil
}

// sample turns the cumulative client side totals into the samples of the
// window that ended now. Two mix entries of one scenario share its counters,
// so the window splits by their weights, which keeps the bytes per session of
// each. A scenario without a completed session leaves its bytes uncounted
// until a window that has one, so no byte is lost and none counts twice. A
// total that fell, which a restarted agent causes, starts the count again.
func (c *calibrator) sample(now map[string]calTotals) []calibrate.Sample {
	share := map[string]float64{}
	for _, e := range c.entries {
		share[e.scenario] += e.weight
	}
	delta := map[string]calTotals{}
	for name := range share {
		cur, prev := now[name], c.last[name]
		ds, db := cur.sessions-prev.sessions, cur.bytes-prev.bytes
		switch {
		case ds < 0 || db < 0:
			c.last[name] = cur
		case ds > 0:
			delta[name] = calTotals{bytes: db, sessions: ds}
			c.last[name] = cur
		}
	}
	out := make([]calibrate.Sample, 0, len(c.entries))
	for _, e := range c.entries {
		f := e.weight / share[e.scenario]
		d := delta[e.scenario]
		out = append(out, calibrate.Sample{Entry: e.id, Bytes: d.bytes * f, Sessions: d.sessions * f})
	}
	return out
}

// engineAtLeast reports whether the engine contract version v, such as 0.2.1,
// is at least major.minor.0.
func engineAtLeast(v string, major, minor int) bool {
	parts := strings.SplitN(v, ".", 3)
	if len(parts) < 2 {
		return false
	}
	ma, err := strconv.Atoi(parts[0])
	if err != nil {
		return false
	}
	mi, err := strconv.Atoi(parts[1])
	if err != nil {
		return false
	}
	return ma > major || (ma == major && mi >= minor)
}

// ratesCommand returns the UpdateRates command that sets the session rate of
// every mix entry of agent a to its compiled rate times factor. The command
// carries the whole agent total of each entry, as CON-05 asks, which is the sum
// of the compiled rates of its workers. It returns nil for an agent without a
// session rate, for example an agent that runs the server side only.
func ratesCommand(a *runAgent, entries []calEntry, factor float64) *agentpb.UpdateRatesCommand {
	sum := map[string]float64{}
	for _, w := range a.plan.Workers {
		for _, wr := range w.Rates {
			if wr.SessionsPerSecond != nil {
				sum[wr.MixEntry] += *wr.SessionsPerSecond
			}
		}
	}
	var rates []*agentpb.RateUpdate
	for _, e := range entries {
		if s := sum[e.id]; s > 0 {
			rates = append(rates, &agentpb.RateUpdate{
				MixEntry: e.id,
				Target:   &agentpb.RateUpdate_SessionsPerSecond{SessionsPerSecond: s * factor},
			})
		}
	}
	if len(rates) == 0 {
		return nil
	}
	return &agentpb.UpdateRatesCommand{PlanId: a.plan.PlanID, Rates: rates}
}

// appliedFactor returns the factor that agent a runs with, which is 1 until
// an UpdateRates succeeded. The caller holds the orchestrator lock.
func (a *runAgent) appliedFactor() float64 {
	if a.calSent {
		return a.calApplied
	}
	return 1
}

// startCalibration starts the calibration loop of r and returns the function
// that stops it and waits for it. The function may be called more than once.
// A run without a calibrator, or an agent manager that cannot send
// UpdateRates, gets no loop.
func (o *Orchestrator) startCalibration(r *run) (stop func()) {
	c := r.cal
	if c == nil {
		return func() {}
	}
	updater, ok := o.cfg.Agents.(RateUpdater)
	if !ok {
		o.log.Warn("throughput calibration is off: the agent manager cannot send UpdateRates", "run_id", r.ID)
		return func() {}
	}
	c.updater = updater
	ctx, cancel := context.WithCancel(r.force)
	done := make(chan struct{})
	go func() {
		defer close(done)
		t := time.NewTicker(c.window)
		defer t.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-t.C:
				o.calibrateOnce(ctx, r, c)
			}
		}
	}()
	o.log.Info("throughput calibration started", "run_id", r.ID, "window", c.window.String())
	return func() {
		cancel()
		<-done
	}
}

// clientTotals returns the cumulative client side bytes (sent plus received)
// and completed sessions of the named scenarios, summed over the agents of r.
// The client side counts every payload byte once, as CTL-21 asks.
func (o *Orchestrator) clientTotals(r *run, scenarios map[string]bool) map[string]calTotals {
	o.mu.Lock()
	defer o.mu.Unlock()
	out := map[string]calTotals{}
	for _, a := range r.agents {
		for _, s := range a.series {
			name := s.Labels[labelScenario]
			if s.Labels[LabelSide] != SideClient || !scenarios[name] {
				continue
			}
			t := out[name]
			switch s.Metric {
			case MetricBytesTx, MetricBytesRx:
				t.bytes += s.Value
			case MetricSessionsSucceeded:
				t.sessions += s.Value
			default:
				continue
			}
			out[name] = t
		}
	}
	return out
}

// calibrateOnce folds the window that just ended into the calibration, records
// a change of the factor, and brings every agent to the factor.
func (o *Orchestrator) calibrateOnce(ctx context.Context, r *run, c *calibrator) {
	next, res, err := c.state.Update(c.sample(o.clientTotals(r, c.scenarios)))
	if err != nil {
		o.log.Warn("throughput calibration skipped a window", "run_id", r.ID, "error", err)
		return
	}
	c.state = next
	if res.Changed {
		step := CalibrationStep{
			Time: o.cfg.Clock.Now().UTC(), Factor: res.Factor, Target: res.Target,
			SessionsPerSecond: res.SessionsPerSecond, Entries: slices.Clone(res.Entries),
		}
		o.mu.Lock()
		r.Calibration = append(r.Calibration, step)
		if over := len(r.Calibration) - maxCalibrationSteps; over > 0 {
			r.Calibration = slices.Delete(r.Calibration, 0, over)
		}
		o.mu.Unlock()
		o.log.Info("throughput calibration step", "run_id", r.ID, "factor", res.Factor, "target", res.Target,
			"sessions_per_second", res.SessionsPerSecond, "bytes_per_session", bytesPerSession(res.Entries))
	}
	o.pushRates(ctx, r, c, res.Factor)
}

// bytesPerSession words the estimates of the mix entries for a log line.
func bytesPerSession(es []calibrate.EntryResult) string {
	parts := make([]string, len(es))
	for i, e := range es {
		parts[i] = e.ID + "=" + strconv.FormatFloat(e.BytesPerSession, 'f', 1, 64)
	}
	return strings.Join(parts, ",")
}

// pushRates sends UpdateRates to every running agent that does not run with
// factor yet. An agent that failed to apply the rates is tried again in the
// next window, so a lost command costs one window and no more.
func (o *Orchestrator) pushRates(ctx context.Context, r *run, c *calibrator, factor float64) {
	// forEachIn calls keep with the orchestrator lock held.
	stale := func(a *runAgent) bool {
		return a.status == agentActive && a.started && !a.calSkip && a.appliedFactor() != factor
	}
	send := func(ctx context.Context, a *runAgent) error { return o.sendRates(ctx, r, c, a, factor) }
	_ = o.forEachIn(ctx, r, stale, send)
}

// sendRates brings agent a to factor. An agent with an engine contract older
// than 0.2.0 never receives the command, and the orchestrator logs that once.
func (o *Orchestrator) sendRates(ctx context.Context, r *run, c *calibrator, a *runAgent, factor float64) error {
	cmd := ratesCommand(a, c.entries, factor)
	if cmd == nil {
		o.skipCalibration(a)
		return nil
	}
	ag, err := o.cfg.Agents.Get(a.id)
	if err != nil {
		return fmt.Errorf("agent %s: %w", a.id, err)
	}
	if engine := ag.Capabilities.GetEngineApi(); !engineAtLeast(engine, updateRatesMajor, updateRatesMinor) {
		o.skipCalibration(a)
		o.log.Warn("agent has no UpdateRates, so its rates stay uncalibrated",
			"run_id", r.ID, "agent_id", a.id, "engine_api", engine)
		return nil
	}
	if _, err := c.updater.UpdateRates(ctx, a.id, cmd); err != nil {
		o.mu.Lock()
		first := a.calFailed != factor
		a.calFailed = factor
		o.mu.Unlock()
		if first && ctx.Err() == nil && !errors.Is(err, context.Canceled) {
			o.log.Warn("update rates failed, will retry", "run_id", r.ID, "agent_id", a.id, "factor", factor, "error", err)
		}
		return fmt.Errorf("agent %s: %w", a.id, err)
	}
	o.mu.Lock()
	a.calSent, a.calApplied, a.calFailed = true, factor, 0
	o.mu.Unlock()
	o.log.Info("rates updated", "run_id", r.ID, "agent_id", a.id, "factor", factor)
	return nil
}

// skipCalibration leaves agent a out of the calibration for the rest of the run.
func (o *Orchestrator) skipCalibration(a *runAgent) {
	o.mu.Lock()
	a.calSkip = true
	o.mu.Unlock()
}
