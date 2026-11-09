// SPDX-License-Identifier: GPL-3.0-only

// Package runs drives test runs. The orchestrator moves each run through the
// run state machine from pending to preparing, running, and stopping, and
// ends it as completed, failed, or aborted. It reserves the agents of a run,
// compiles one plan per agent, sends the prepare, start, and stop commands,
// and contains agent faults. An agent that fails while the run is under way
// ends only its own sessions. The orchestrator marks the run degraded, keeps
// the other agents running, and lets the pass and fail criteria decide the
// outcome when the run stops.
//
// The orchestrator reaches the configuration store, the compiler, the
// scenario catalog, and the agent manager through interfaces, so tests pass
// fakes or the real packages with fake agents.
package runs

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"
	"maps"
	"slices"
	"strings"
	"sync"
	"time"

	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/types/known/durationpb"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// Default timeouts of the agent commands.
const (
	DefaultCommandTimeout = 30 * time.Second
	DefaultDrainTimeout   = 10 * time.Second
)

// Errors that callers test with errors.Is.
var (
	// ErrRunNotFound reports that no run has the requested id.
	ErrRunNotFound = errors.New("run not found")
	// ErrRunFinished reports a stop of a run that already ended.
	ErrRunFinished = errors.New("run already finished")
	// ErrInvalidRequest reports a start request that names a configuration
	// or agents that cannot form a run.
	ErrInvalidRequest = errors.New("invalid run request")
	// ErrNoAgents reports a start request without agents when no agent is
	// reserved by the operator.
	ErrNoAgents = errors.New("no agents for the run")
	// ErrClosed reports that Close ran.
	ErrClosed = errors.New("run orchestrator closed")
)

// ConfigStore returns stored test configurations. *store.SQLite satisfies it.
type ConfigStore interface {
	Get(ctx context.Context, id string) (store.Record, error)
}

// Compiler compiles a run into one plan per agent.
type Compiler interface {
	Compile(req compiler.Request) ([]compiler.Plan, error)
}

// CompilerFunc adapts a function such as compiler.Compile to Compiler.
type CompilerFunc func(req compiler.Request) ([]compiler.Plan, error)

// Compile calls f.
func (f CompilerFunc) Compile(req compiler.Request) ([]compiler.Plan, error) { return f(req) }

// Catalog lists the scenarios of the installed modules.
type Catalog interface {
	Scenarios(ctx context.Context) ([]compiler.Scenario, error)
}

// AgentManager is the part of the agent manager that runs need.
// *agents.Manager satisfies it.
type AgentManager interface {
	List() []agents.Agent
	Get(id string) (agents.Agent, error)
	Reserve(id, holder string) (agents.Agent, error)
	Release(id, holder string) (agents.Agent, error)
	Prepare(ctx context.Context, id string, cmd *agentpb.PrepareCommand) (*agentpb.PrepareResult, error)
	Start(ctx context.Context, id string, cmd *agentpb.StartCommand) (*agentpb.StartResult, error)
	Stop(ctx context.Context, id string, cmd *agentpb.StopCommand) (*agentpb.StopResult, error)
}

var _ AgentManager = (*agents.Manager)(nil)

// Config configures an Orchestrator. Store and Agents are required, and
// zero fields of the others take the defaults that New names.
type Config struct {
	// Store returns the test configurations.
	Store ConfigStore
	// Agents is the agent manager.
	Agents AgentManager
	// Compiler defaults to compiler.Compile.
	Compiler Compiler
	// Catalog lists the installed scenarios. Nil means an empty catalog.
	Catalog Catalog
	// Evaluator decides the verdict of a stopped run. It defaults to
	// EvaluateTotals.
	Evaluator Evaluator
	// Clock tells the time. It defaults to agents.SystemClock.
	Clock agents.Clock
	// NewID returns a fresh run id. It defaults to RandomID.
	NewID func() (string, error)
	// Seed returns the seed that the compiler uses for a configuration
	// without timeline.seed. It defaults to nil, and then the compiler draws
	// a random nonzero seed. The run records the seed of its compiled plans
	// in either case.
	Seed func() uint64
	// CommandTimeout bounds every prepare, start, and stop command.
	CommandTimeout time.Duration
	// DrainTimeout bounds the ramp down of a graceful stop.
	DrainTimeout time.Duration
	// Calibration configures the correction of throughput_bps objectives
	// during a run. It needs an Agents value that implements RateUpdater.
	Calibration CalibrationConfig
	// Logger receives structured logs. The default discards them.
	Logger *slog.Logger
}

// StartRequest is the body of POST /v1/runs.
type StartRequest struct {
	// ConfigID names the stored test configuration.
	ConfigID string
	// Agents names the agents of the run. When empty, the run takes every
	// agent that the operator reserved.
	Agents []string
	// Labels annotate the run.
	Labels map[string]string
}

// Run is a copy of the state of one run.
type Run struct {
	ID       string
	ConfigID string
	State    State
	// Degraded is true when an agent failed during the run.
	Degraded bool
	// Phase is the load phase of the run, from the agent heartbeats.
	Phase     string
	Labels    map[string]string
	CreatedAt time.Time
	StartedAt time.Time
	EndedAt   time.Time
	Seed      uint64
	// EngineVersion and ModuleVersions come from the compiled plans.
	EngineVersion  string
	ModuleVersions map[string]string
	// Agents names the agents of the run, ordered by id, and FailedAgents
	// the ones that failed during the run.
	Agents       []string
	FailedAgents []string
	// Reason says in words why the run failed or was aborted, such as the
	// criteria that failed, the failure of every agent, or the stop request.
	// It is set for every run that ends as failed or aborted, and empty in
	// any other state.
	Reason string
	// Calibration lists the changes of the calibration factor of a run with
	// a throughput_bps objective, oldest first. It is empty for any other
	// objective and while the factor stays at one.
	Calibration []CalibrationStep
	// Totals, Series, and Verdict are set when the run stopped and its
	// criteria were evaluated. Series holds the counters of the agents with
	// their labels, which a report merges with the metrics aggregator.
	Totals  map[string]float64
	Series  []Series
	Verdict *Verdict
}

// stopRequest is the strongest stop request a run received.
type stopRequest int

const (
	stopNone stopRequest = iota
	stopGraceful
	stopForce
)

// agentStatus tracks one agent within one run.
type agentStatus int

const (
	agentActive agentStatus = iota
	agentFinished
	agentFailed
)

// runAgent is one agent of a run. The orchestrator lock guards it.
type runAgent struct {
	id          string
	plan        compiler.Plan
	priorHolder string
	status      agentStatus
	prepared    bool
	started     bool
	// seq is the sequence of the newest snapshot, and counters holds the
	// newest value of every unlabeled counter and gauge. series holds the
	// newest value of every counter by metric and labels. Counters are
	// cumulative, so a final snapshot without counters loses nothing.
	seq      uint64
	counters map[string]float64
	series   map[string]Series
	// The calibration of the run keeps its state of the agent here. calSent
	// says that an UpdateRates succeeded and calApplied holds the factor that
	// it set, calFailed holds the factor of the last failed attempt that the
	// log reported, and calSkip leaves the agent out of the calibration.
	calSent    bool
	calApplied float64
	calFailed  float64
	calSkip    bool
}

// absorb takes the counters of snap when it is newer than every earlier
// snapshot of a, and reports whether it was. The caller holds the lock.
func (a *runAgent) absorb(snap *agentpb.CounterSnapshot) bool {
	if snap.GetSequence() <= a.seq {
		return false
	}
	a.seq = snap.GetSequence()
	if a.counters == nil {
		a.counters = map[string]float64{}
		a.series = map[string]Series{}
	}
	for _, c := range snap.GetCounters() {
		switch c.GetValue().(type) {
		case *agentpb.CounterValue_Count:
			s := Series{Metric: c.GetMetric(), Labels: maps.Clone(c.GetLabels()), Value: float64(c.GetCount())}
			a.series[seriesKey(s.Metric, s.Labels)] = s
			if len(c.GetLabels()) == 0 {
				a.counters[c.GetMetric()] = s.Value
			}
		case *agentpb.CounterValue_Gauge:
			if len(c.GetLabels()) == 0 {
				a.counters[c.GetMetric()] = c.GetGauge()
			}
		}
	}
	return true
}

// run is the state of one run. The orchestrator lock guards every field
// except the channels and the cancel function, which are set at creation.
type run struct {
	Run
	criteria []Criterion
	agents   map[string]*runAgent
	stop     stopRequest
	// cal is the throughput calibration, or nil for another objective.
	cal *calibrator

	// force ends when a forced stop arrives or the orchestrator closes, and
	// cancels the commands that a forced stop skips.
	force       context.Context
	forceCancel context.CancelFunc
	wake        chan struct{}
	done        chan struct{}
}

// Orchestrator drives runs. It is safe for concurrent use.
type Orchestrator struct {
	cfg Config
	log *slog.Logger

	ctx    context.Context
	cancel context.CancelFunc
	wg     sync.WaitGroup

	mu      sync.Mutex
	runs    map[string]*run
	byAgent map[string]*run
	closed  bool
}

// New returns an Orchestrator. Call Observe with every event of the agent
// manager, for example through agents.Config.Observer, and Close at the end.
func New(cfg Config) (*Orchestrator, error) {
	if cfg.Store == nil || cfg.Agents == nil {
		return nil, errors.New("run orchestrator config: store and agents are required")
	}
	if cfg.Compiler == nil {
		cfg.Compiler = CompilerFunc(compiler.Compile)
	}
	if cfg.Evaluator == nil {
		cfg.Evaluator = EvaluateTotals
	}
	if cfg.Clock == nil {
		cfg.Clock = agents.SystemClock{}
	}
	if cfg.NewID == nil {
		cfg.NewID = RandomID
	}
	if cfg.CommandTimeout <= 0 {
		cfg.CommandTimeout = DefaultCommandTimeout
	}
	if cfg.DrainTimeout <= 0 {
		cfg.DrainTimeout = DefaultDrainTimeout
	}
	if cfg.Logger == nil {
		cfg.Logger = slog.New(slog.DiscardHandler)
	}
	ctx, cancel := context.WithCancel(context.Background())
	return &Orchestrator{
		cfg: cfg, log: cfg.Logger, ctx: ctx, cancel: cancel,
		runs: map[string]*run{}, byAgent: map[string]*run{},
	}, nil
}

// RunIDPrefix starts every run id that RandomID returns, so run ids differ
// from the cfg_ ids of configurations.
const RunIDPrefix = "run_"

// RandomID returns run_ and 24 random hex digits, which matches the contract
// id pattern ^[A-Za-z0-9_-]{1,64}$.
func RandomID() (string, error) {
	var b [12]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", fmt.Errorf("new run id: %w", err)
	}
	return RunIDPrefix + hex.EncodeToString(b[:]), nil
}

// Close aborts every run that has not ended and waits for them.
func (o *Orchestrator) Close() {
	o.mu.Lock()
	o.closed = true
	o.mu.Unlock()
	o.cancel()
	o.wg.Wait()
}

// Create loads the configuration, reserves the agents, compiles the plans,
// and starts a run in pending. The run proceeds in the background.
func (o *Orchestrator) Create(ctx context.Context, req StartRequest) (Run, error) {
	o.mu.Lock()
	closed := o.closed
	o.mu.Unlock()
	if closed {
		return Run{}, ErrClosed
	}
	if req.ConfigID == "" {
		return Run{}, fmt.Errorf("start run: %w: empty config_id", ErrInvalidRequest)
	}
	rec, err := o.cfg.Store.Get(ctx, req.ConfigID)
	if err != nil {
		return Run{}, fmt.Errorf("start run of config %s: %w", req.ConfigID, err)
	}
	cfg, err := compiler.ParseConfig(rec.Body)
	if err != nil {
		return Run{}, fmt.Errorf("start run of config %s: %w: %w", req.ConfigID, ErrInvalidRequest, err)
	}
	id, err := o.cfg.NewID()
	if err != nil {
		return Run{}, fmt.Errorf("start run of config %s: %w", req.ConfigID, err)
	}
	ids, err := o.pickAgents(req.Agents)
	if err != nil {
		return Run{}, fmt.Errorf("start run %s: %w", id, err)
	}
	reserved, err := o.reserve(id, ids)
	if err != nil {
		return Run{}, fmt.Errorf("start run %s: %w", id, err)
	}
	plans, scenarios, err := o.compile(ctx, id, cfg, reserved)
	if err != nil {
		o.release(id, reserved)
		return Run{}, fmt.Errorf("start run %s: %w", id, err)
	}
	r := o.newRun(id, req, cfg, reserved, plans)
	cal, err := newCalibrator(o.cfg.Calibration, plans, scenarios)
	if err != nil {
		// The run still meets its objective with the compiled rates.
		o.log.Warn("throughput calibration is off", "run_id", id, "error", err)
	}
	r.cal = cal

	o.mu.Lock()
	if o.closed {
		o.mu.Unlock()
		o.release(id, reserved)
		return Run{}, ErrClosed
	}
	o.runs[id] = r
	for aid := range r.agents {
		o.byAgent[aid] = r
	}
	o.wg.Add(1)
	out := r.copy()
	o.mu.Unlock()
	o.log.Info("run created", "run_id", id, "config_id", req.ConfigID, "agents", strings.Join(r.Agents, ","))
	go o.drive(r)
	return out, nil
}

// pickAgents returns the requested agent ids, sorted and without
// duplicates, or every agent that the operator reserved.
func (o *Orchestrator) pickAgents(requested []string) ([]string, error) {
	var ids []string
	if len(requested) > 0 {
		ids = slices.Clone(requested)
	} else {
		for _, ag := range o.cfg.Agents.List() {
			if ag.ReservedBy == agents.OperatorHolder {
				ids = append(ids, ag.ID)
			}
		}
	}
	slices.Sort(ids)
	ids = slices.Compact(ids)
	if len(ids) == 0 {
		return nil, ErrNoAgents
	}
	return ids, nil
}

// reserved is one agent that a run reserved, with the holder it had before.
type reserved struct {
	agent       agents.Agent
	priorHolder string
}

// reserve reserves every agent for the run id. An operator reservation
// passes to the run and returns to the operator when the run ends. On error
// it restores the agents that it already took.
func (o *Orchestrator) reserve(id string, ids []string) ([]reserved, error) {
	var out []reserved
	for _, aid := range ids {
		ag, err := o.cfg.Agents.Get(aid)
		if err != nil {
			o.release(id, out)
			return nil, fmt.Errorf("%w: %w", ErrInvalidRequest, err)
		}
		prior := ag.ReservedBy
		if prior == agents.OperatorHolder {
			if _, err := o.cfg.Agents.Release(aid, agents.OperatorHolder); err != nil {
				o.release(id, out)
				return nil, err
			}
		}
		ag, err = o.cfg.Agents.Reserve(aid, id)
		if err != nil {
			if prior == agents.OperatorHolder {
				o.restore(aid, prior)
			}
			o.release(id, out)
			return nil, err
		}
		out = append(out, reserved{agent: ag, priorHolder: prior})
	}
	return out, nil
}

// release ends the reservations of the run id and restores the operator
// reservations that the run took.
func (o *Orchestrator) release(id string, rs []reserved) {
	for _, r := range rs {
		if _, err := o.cfg.Agents.Release(r.agent.ID, id); err != nil {
			o.log.Warn("agent release failed", "run_id", id, "agent_id", r.agent.ID, "error", err)
		}
		if r.priorHolder == agents.OperatorHolder {
			o.restore(r.agent.ID, r.priorHolder)
		}
	}
}

func (o *Orchestrator) restore(agentID, holder string) {
	if _, err := o.cfg.Agents.Reserve(agentID, holder); err != nil {
		o.log.Warn("agent reservation not restored", "agent_id", agentID, "holder", holder, "error", err)
	}
}

// backendNames maps the proto backends to the configuration names.
var backendNames = map[agentpb.Backend]string{
	agentpb.Backend_BACKEND_KERNEL: "kernel",
	agentpb.Backend_BACKEND_AF_XDP: "af_xdp",
	agentpb.Backend_BACKEND_DPDK:   "dpdk",
}

// compile builds the compiler request from the reserved agents and the
// catalog and compiles it.
func (o *Orchestrator) compile(ctx context.Context, id string, cfg compiler.Config, rs []reserved) ([]compiler.Plan, []compiler.Scenario, error) {
	inv := make([]compiler.Agent, 0, len(rs))
	for _, r := range rs {
		c := r.agent.Capabilities
		ca := compiler.Agent{ID: r.agent.ID, Cores: c.GetCores()}
		for _, b := range c.GetBackends() {
			if name, ok := backendNames[b]; ok {
				ca.Backend = name
				break
			}
		}
		for _, p := range c.GetPorts() {
			ca.Interfaces = append(ca.Interfaces, p.GetName())
		}
		inv = append(inv, ca)
	}
	var scenarios []compiler.Scenario
	if o.cfg.Catalog != nil {
		var err error
		if scenarios, err = o.cfg.Catalog.Scenarios(ctx); err != nil {
			return nil, nil, fmt.Errorf("scenario catalog: %w", err)
		}
	}
	plans, err := o.cfg.Compiler.Compile(compiler.Request{RunID: id, Seed: o.requestSeed(), Config: cfg, Agents: inv, Scenarios: scenarios})
	if err != nil {
		return nil, nil, fmt.Errorf("%w: %w", ErrInvalidRequest, err)
	}
	byAgent := map[string]bool{}
	for _, p := range plans {
		// Every plan of a run carries the seed of the run, and the run
		// record takes it from the plans, so it is never 0.
		if p.Seed == 0 {
			return nil, nil, fmt.Errorf("compiler returned plan %s without a seed", p.PlanID)
		}
		byAgent[p.AgentID] = true
	}
	for _, r := range rs {
		if !byAgent[r.agent.ID] {
			return nil, nil, fmt.Errorf("compiler returned no plan for agent %s", r.agent.ID)
		}
	}
	return plans, scenarios, nil
}

// requestSeed returns the seed that the configuration of the orchestrator
// offers to the compiler, which is 0 when the compiler is to draw one.
func (o *Orchestrator) requestSeed() uint64 {
	if o.cfg.Seed == nil {
		return 0
	}
	return o.cfg.Seed()
}

func (o *Orchestrator) newRun(id string, req StartRequest, cfg compiler.Config, rs []reserved, plans []compiler.Plan) *run {
	force, cancel := context.WithCancel(o.ctx)
	r := &run{
		Run: Run{
			ID: id, ConfigID: req.ConfigID, State: Pending,
			Labels: maps.Clone(req.Labels), CreatedAt: o.cfg.Clock.Now().UTC(),
		},
		criteria:    criteriaOf(cfg.Criteria),
		agents:      map[string]*runAgent{},
		force:       force,
		forceCancel: cancel,
		wake:        make(chan struct{}, 1),
		done:        make(chan struct{}),
	}
	prior := map[string]string{}
	for _, x := range rs {
		prior[x.agent.ID] = x.priorHolder
	}
	for _, p := range plans {
		if _, ok := prior[p.AgentID]; !ok {
			continue
		}
		r.agents[p.AgentID] = &runAgent{id: p.AgentID, plan: p, priorHolder: prior[p.AgentID]}
		r.Agents = append(r.Agents, p.AgentID)
	}
	slices.Sort(r.Agents)
	if len(plans) > 0 {
		r.Seed = plans[0].Seed
		r.EngineVersion = plans[0].EngineAPI
		r.ModuleVersions = map[string]string{}
		for _, m := range plans[0].Modules {
			r.ModuleVersions[m.Name] = m.Version
		}
	}
	return r
}

// copy returns a deep copy of the public state. The caller holds the lock.
func (r *run) copy() Run {
	out := r.Run
	out.Labels = maps.Clone(r.Labels)
	out.ModuleVersions = maps.Clone(r.ModuleVersions)
	out.Agents = slices.Clone(r.Agents)
	out.FailedAgents = slices.Clone(r.FailedAgents)
	out.Calibration = slices.Clone(r.Calibration)
	out.Totals = maps.Clone(r.Totals)
	out.Series = slices.Clone(r.Series)
	if r.Verdict != nil {
		v := *r.Verdict
		v.Criteria = slices.Clone(r.Verdict.Criteria)
		out.Verdict = &v
	}
	return out
}

// Get returns the run with id.
func (o *Orchestrator) Get(id string) (Run, error) {
	o.mu.Lock()
	defer o.mu.Unlock()
	r := o.runs[id]
	if r == nil {
		return Run{}, fmt.Errorf("get run %s: %w", id, ErrRunNotFound)
	}
	return r.copy(), nil
}

// Wait blocks until the run with id ends or ctx ends, and returns the run.
func (o *Orchestrator) Wait(ctx context.Context, id string) (Run, error) {
	o.mu.Lock()
	r := o.runs[id]
	o.mu.Unlock()
	if r == nil {
		return Run{}, fmt.Errorf("wait for run %s: %w", id, ErrRunNotFound)
	}
	select {
	case <-r.done:
	case <-ctx.Done():
		return o.Get(id)
	}
	return o.Get(id)
}

// Stop asks the run with id to stop. A graceful stop ramps the load down and
// still evaluates the criteria. A forced stop aborts at once and ends the run
// as aborted. A stop before the run starts aborts it. Stop returns the run as
// it stands, and the run reaches its end state in the background.
func (o *Orchestrator) Stop(id string, force bool) (Run, error) {
	o.mu.Lock()
	defer o.mu.Unlock()
	r := o.runs[id]
	switch {
	case r == nil:
		return Run{}, fmt.Errorf("stop run %s: %w", id, ErrRunNotFound)
	case Final(r.State):
		return Run{}, fmt.Errorf("stop run %s in state %s: %w", id, r.State, ErrRunFinished)
	}
	if force {
		r.stop = stopForce
		r.forceCancel()
	} else if r.stop == stopNone {
		r.stop = stopGraceful
	}
	o.log.Info("run stop requested", "run_id", id, "force", force, "state", r.State)
	r.signal()
	return r.copy(), nil
}

// signal wakes the run goroutine without blocking.
func (r *run) signal() {
	select {
	case r.wake <- struct{}{}:
	default:
	}
}

// Observe takes one event of the agent manager. It marks an agent finished
// when it reports AGENT_STATE_STOPPED for its plan, and failed when it
// reports AGENT_STATE_FAILED, loses its plan, or goes offline, which
// marks the run degraded. It keeps the newest counter snapshot of every
// agent. Observe returns quickly, as agents.Config.Observer requires.
func (o *Orchestrator) Observe(ev agents.Event) {
	o.mu.Lock()
	defer o.mu.Unlock()
	r := o.byAgent[ev.AgentID]
	if r == nil || Final(r.State) {
		return
	}
	a := r.agents[ev.AgentID]
	if a == nil {
		return
	}
	switch ev.Kind {
	case agents.EventSnapshot:
		snap := ev.Snapshot
		if snap.GetPlanId() == a.plan.PlanID && a.absorb(snap) {
			if p := snap.GetPhase(); p != "" {
				r.Phase = p
			}
		}
	case agents.EventStateChanged, agents.EventRegistered:
		if a.status != agentActive || !a.prepared {
			break
		}
		switch ev.AgentState {
		case agentpb.AgentState_AGENT_STATE_FAILED:
			o.markFailed(r, a, "agent reported failed")
		case agentpb.AgentState_AGENT_STATE_STOPPED:
			if a.started && ev.PlanID == a.plan.PlanID {
				a.status = agentFinished
			}
		case agentpb.AgentState_AGENT_STATE_IDLE:
			if ev.PlanID != a.plan.PlanID {
				o.markFailed(r, a, "agent lost its plan")
			}
		default:
			if ev.PlanID != "" && ev.PlanID != a.plan.PlanID {
				o.markFailed(r, a, "agent runs another plan")
			}
		}
	case agents.EventOffline:
		if a.status == agentActive && r.State != Pending {
			o.markFailed(r, a, "agent offline: "+ev.Reason)
		}
	case agents.EventOnline:
	}
	r.signal()
}

// markFailed marks agent a of run r failed and the run degraded. The caller
// holds the lock.
func (o *Orchestrator) markFailed(r *run, a *runAgent, reason string) {
	if a.status == agentFailed {
		return
	}
	a.status = agentFailed
	r.Degraded = true
	r.FailedAgents = append(r.FailedAgents, a.id)
	slices.Sort(r.FailedAgents)
	o.log.Warn("run degraded", "run_id", r.ID, "agent_id", a.id, "reason", reason)
}

// transition moves run r to state to. The caller holds no lock.
func (o *Orchestrator) transition(r *run, to State) error {
	o.mu.Lock()
	from := r.State
	next, err := Transition(from, to)
	if err == nil {
		r.State = next
		if to == Running {
			r.StartedAt = o.cfg.Clock.Now().UTC()
		}
	}
	o.mu.Unlock()
	if err != nil {
		o.log.Error("run transition rejected", "run_id", r.ID, "from", from, "to", to, "error", err)
		return err
	}
	o.log.Info("run state changed", "run_id", r.ID, "from", from, "to", to)
	return nil
}

// stopState returns the stop request of r and whether the orchestrator closed.
func (o *Orchestrator) stopState(r *run) stopRequest {
	o.mu.Lock()
	defer o.mu.Unlock()
	if o.ctx.Err() != nil {
		return stopForce
	}
	return r.stop
}

// drive runs r from pending to an end state.
func (o *Orchestrator) drive(r *run) {
	defer o.wg.Done()
	if o.stopState(r) != stopNone {
		o.finish(r, Aborted, o.abortReason("stopped before preparing"))
		return
	}
	if o.transition(r, Preparing) != nil {
		return
	}
	err := o.forEach(r, everyAgent, o.prepareAgent)
	if o.stopState(r) != stopNone {
		o.abort(r, o.abortReason("stopped while preparing"))
		return
	}
	if err != nil {
		o.fail(r, "prepare: "+err.Error())
		return
	}
	if o.transition(r, Running) != nil {
		return
	}
	if err := o.forEach(r, everyAgent, o.startAgent); err != nil {
		if o.stopState(r) == stopForce {
			o.abort(r, o.abortReason("stopped while starting"))
			return
		}
		o.fail(r, "start: "+err.Error())
		return
	}
	// From here on, a forced stop ends the run with the reason "stopped by
	// request", and before it, with a reason that names the phase.
	o.log.Info("run started on every agent", "run_id", r.ID)
	// The calibration must end before the run leaves the running state,
	// because an agent accepts UpdateRates only while it runs.
	stopCalibration := o.startCalibration(r)
	for {
		stop, active, failed := o.progress(r)
		switch {
		case stop == stopForce:
			stopCalibration()
			o.abort(r, o.abortReason("stopped by request"))
			return
		case active == 0 && failed == len(r.agents):
			stopCalibration()
			o.finish(r, Failed, "every agent failed")
			return
		case stop == stopGraceful || active == 0:
			stopCalibration()
			o.stopping(r)
			return
		}
		select {
		case <-r.wake:
		case <-r.force.Done():
		}
	}
}

// everyAgent selects every agent of a run for forEach.
func everyAgent(*runAgent) bool { return true }

// abortReason names the cause of an abort, which is the shutdown of the
// orchestrator or else the stop request that reason describes.
func (o *Orchestrator) abortReason(reason string) string {
	if o.ctx.Err() != nil {
		return "controller shutdown"
	}
	return reason
}

// progress returns the stop request and the number of active and failed
// agents of r.
func (o *Orchestrator) progress(r *run) (stop stopRequest, active, failed int) {
	o.mu.Lock()
	defer o.mu.Unlock()
	for _, a := range r.agents {
		switch a.status {
		case agentActive:
			active++
		case agentFailed:
			failed++
		case agentFinished:
		}
	}
	stop = r.stop
	if o.ctx.Err() != nil {
		stop = stopForce
	}
	return stop, active, failed
}

// stopping ramps down the active agents of r, evaluates the criteria, and
// ends the run. A forced stop during the ramp down aborts the run.
func (o *Orchestrator) stopping(r *run) {
	if o.transition(r, Stopping) != nil {
		return
	}
	active := func(a *runAgent) bool { return a.status == agentActive && a.started }
	rampDown := func(ctx context.Context, a *runAgent) error {
		return o.stopAgent(ctx, a, r, agentpb.StopMode_STOP_MODE_RAMP_DOWN)
	}
	_ = o.forEach(r, active, rampDown)
	if o.stopState(r) == stopForce {
		o.abort(r, o.abortReason("stopped by request"))
		return
	}
	o.mu.Lock()
	counters := map[string]map[string]float64{}
	perAgent := map[string]map[string]Series{}
	failed := map[string]bool{}
	nFailed := 0
	for id, a := range r.agents {
		counters[id] = a.counters
		perAgent[id] = a.series
		if a.status == agentFailed {
			failed[id] = true
			nFailed++
		}
	}
	series := sumSeries(perAgent, failed)
	out := Outcome{
		RunID: r.ID, Criteria: slices.Clone(r.criteria), Series: series, Totals: totals(counters, series, failed),
		Agents: len(r.agents), FailedAgents: nFailed,
	}
	o.mu.Unlock()
	v := o.cfg.Evaluator(out)
	o.mu.Lock()
	r.Totals, r.Series = out.Totals, out.Series
	r.Verdict = &v
	o.mu.Unlock()
	if v.Passed {
		o.finish(r, Completed, "")
		return
	}
	o.finish(r, Failed, failureReason(v, out))
}

// maxReasonCriteria bounds the criteria that a failure reason names.
const maxReasonCriteria = 3

// failureReason words why a stopped run failed. It names the criteria that
// failed, each with its reason, which states the label filter. It falls back
// to the failure of every agent when no criterion failed.
func failureReason(v Verdict, o Outcome) string {
	var items []string
	for _, c := range v.Criteria {
		failing := c.Invalid || (c.Evaluated && !c.Passed)
		if !failing {
			continue
		}
		item := c.Metric
		if c.Reason != "" {
			item += ": " + c.Reason
		}
		items = append(items, item)
	}
	switch {
	case len(items) > maxReasonCriteria:
		more := len(items) - maxReasonCriteria
		return fmt.Sprintf("criteria failed: %s; and %d more", strings.Join(items[:maxReasonCriteria], "; "), more)
	case len(items) > 0:
		return "criteria failed: " + strings.Join(items, "; ")
	case o.Agents > 0 && o.FailedAgents == o.Agents:
		return "every agent failed"
	}
	return "criteria failed"
}

// abort sends an abort to every prepared agent of r that has not ended and
// ends the run as aborted.
func (o *Orchestrator) abort(r *run, reason string) {
	o.abortAgents(r)
	o.finish(r, Aborted, reason)
}

// fail sends an abort to every prepared agent of r and ends the run as failed.
func (o *Orchestrator) fail(r *run, reason string) {
	o.abortAgents(r)
	o.finish(r, Failed, reason)
}

func (o *Orchestrator) abortAgents(r *run) {
	live := func(a *runAgent) bool { return a.prepared && a.status == agentActive }
	abort := func(ctx context.Context, a *runAgent) error {
		return o.stopAgent(ctx, a, r, agentpb.StopMode_STOP_MODE_ABORT)
	}
	// An abort must reach the agents even after a forced stop or a
	// shutdown, so it runs outside the run context.
	_ = o.forEachIn(context.WithoutCancel(r.force), r, live, abort)
}

// forEach runs fn for every agent of r that keep selects, in parallel, each
// under the command timeout and the run context, and joins the errors.
func (o *Orchestrator) forEach(r *run, keep func(*runAgent) bool, fn func(context.Context, *runAgent) error) error {
	return o.forEachIn(r.force, r, keep, fn)
}

// forEachIn is forEach under the context base in place of the run context.
func (o *Orchestrator) forEachIn(base context.Context, r *run, keep func(*runAgent) bool, fn func(context.Context, *runAgent) error) error {
	o.mu.Lock()
	var sel []*runAgent
	for _, id := range r.Agents {
		if a := r.agents[id]; keep(a) {
			sel = append(sel, a)
		}
	}
	o.mu.Unlock()
	errs := make([]error, len(sel))
	var wg sync.WaitGroup
	for i, a := range sel {
		wg.Add(1)
		go func() {
			defer wg.Done()
			ctx, cancel := context.WithTimeout(base, o.cfg.CommandTimeout)
			defer cancel()
			errs[i] = fn(ctx, a)
		}()
	}
	wg.Wait()
	return errors.Join(errs...)
}

// prepareAgent sends the plan of a to its agent.
func (o *Orchestrator) prepareAgent(ctx context.Context, a *runAgent) error {
	data, err := a.plan.MarshalCanonical()
	if err != nil {
		return err
	}
	plan := &agentpb.Plan{}
	if err := protojson.Unmarshal(data, plan); err != nil {
		return fmt.Errorf("plan %s: proto mapping: %w", a.plan.PlanID, err)
	}
	o.mu.Lock()
	a.prepared = true
	o.mu.Unlock()
	if _, err := o.cfg.Agents.Prepare(ctx, a.id, &agentpb.PrepareCommand{Plan: plan}); err != nil {
		// An agent that answers with an error keeps its earlier state, so
		// it holds no plan to abort. Without an answer it may hold one.
		var cerr *agents.CommandError
		if errors.As(err, &cerr) {
			o.mu.Lock()
			a.prepared = false
			o.mu.Unlock()
		}
		return fmt.Errorf("agent %s: %w", a.id, err)
	}
	return nil
}

// startAgent starts the prepared plan of a.
func (o *Orchestrator) startAgent(ctx context.Context, a *runAgent) error {
	o.mu.Lock()
	a.started = true
	o.mu.Unlock()
	if _, err := o.cfg.Agents.Start(ctx, a.id, &agentpb.StartCommand{PlanId: a.plan.PlanID}); err != nil {
		return fmt.Errorf("agent %s: %w", a.id, err)
	}
	return nil
}

// stopAgent stops the plan of a in mode, keeps its final snapshot, and
// marks it finished. An agent that cannot answer is marked failed.
func (o *Orchestrator) stopAgent(ctx context.Context, a *runAgent, r *run, mode agentpb.StopMode) error {
	cmd := &agentpb.StopCommand{PlanId: a.plan.PlanID, Mode: mode}
	if mode == agentpb.StopMode_STOP_MODE_RAMP_DOWN {
		cmd.DrainTimeout = durationpb.New(o.cfg.DrainTimeout)
	}
	res, err := o.cfg.Agents.Stop(ctx, a.id, cmd)
	o.mu.Lock()
	defer o.mu.Unlock()
	if err != nil {
		if errors.Is(err, agents.ErrAgentOffline) || errors.Is(err, agents.ErrSessionLost) {
			o.markFailed(r, a, "stop: "+err.Error())
		} else {
			o.log.Warn("agent stop failed", "run_id", r.ID, "agent_id", a.id, "error", err)
		}
		return fmt.Errorf("agent %s: %w", a.id, err)
	}
	if snap := res.GetFinalSnapshot(); snap != nil {
		a.absorb(snap)
	}
	if a.status == agentActive {
		a.status = agentFinished
	}
	return nil
}

// finish ends r in state to, releases its agents, and wakes the waiters.
func (o *Orchestrator) finish(r *run, to State, reason string) {
	o.mu.Lock()
	from := r.State
	next, err := Transition(from, to)
	if err != nil {
		// The run must still end, so a rejected end state becomes failed.
		o.log.Error("run transition rejected", "run_id", r.ID, "from", from, "to", to, "error", err)
		next = Failed
		if reason != "" {
			reason = err.Error() + ": " + reason
		} else {
			reason = err.Error()
		}
	}
	switch {
	case next == Completed:
		reason = ""
	case reason == "" && next == Aborted:
		reason = "aborted"
	case reason == "":
		reason = "failed"
	}
	r.State = next
	r.Reason = reason
	r.EndedAt = o.cfg.Clock.Now().UTC()
	var rs []reserved
	for _, id := range r.Agents {
		a := r.agents[id]
		rs = append(rs, reserved{agent: agents.Agent{ID: id}, priorHolder: a.priorHolder})
		if o.byAgent[id] == r {
			delete(o.byAgent, id)
		}
	}
	degraded := r.Degraded
	o.mu.Unlock()
	o.release(r.ID, rs)
	o.log.Info("run ended", "run_id", r.ID, "from", from, "state", next, "degraded", degraded, "reason", reason)
	r.forceCancel()
	close(r.done)
}
