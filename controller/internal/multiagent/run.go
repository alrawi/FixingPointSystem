// SPDX-License-Identifier: GPL-3.0-only

package multiagent

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"maps"
	"slices"
	"sync"
	"time"

	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/types/known/durationpb"
	"google.golang.org/protobuf/types/known/timestamppb"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
)

// Defaults of Config.
const (
	// DefaultStartLead is the time between the moment that every agent is
	// prepared and the common start instant.
	DefaultStartLead = 2 * time.Second
	// DefaultCommandTimeout bounds every prepare, start, and stop command.
	DefaultCommandTimeout = 30 * time.Second
	// DefaultDrainTimeout bounds the ramp down of a graceful stop.
	DefaultDrainTimeout = 10 * time.Second
)

// Errors that callers test with errors.Is.
var (
	// ErrNoAgents reports a request without agents.
	ErrNoAgents = errors.New("no agents for the run")
	// ErrStopped reports a second call of Run.Stop.
	ErrStopped = errors.New("run already stopped")
)

// AgentManager is the part of the agent manager that a multi agent run uses.
// *agents.Manager satisfies it.
type AgentManager interface {
	Get(id string) (agents.Agent, error)
	Reserve(id, holder string) (agents.Agent, error)
	Release(id, holder string) (agents.Agent, error)
	Prepare(ctx context.Context, id string, cmd *agentpb.PrepareCommand) (*agentpb.PrepareResult, error)
	Start(ctx context.Context, id string, cmd *agentpb.StartCommand) (*agentpb.StartResult, error)
	Stop(ctx context.Context, id string, cmd *agentpb.StopCommand) (*agentpb.StopResult, error)
}

var _ AgentManager = (*agents.Manager)(nil)

// Config configures a Coordinator. Agents and Clocks are required.
type Config struct {
	// Agents is the agent manager.
	Agents AgentManager
	// Clocks measures the clock offsets of the agents.
	Clocks ClockSource
	// Compiler defaults to compiler.Compile.
	Compiler Compiler
	// Clock tells the controller time. It defaults to agents.SystemClock.
	Clock agents.Clock
	// MaxClockOffset is the largest accepted absolute clock offset. It
	// defaults to MaxClockOffset.
	MaxClockOffset time.Duration
	// StartLead is the delay from the last prepared agent to the common start
	// instant, and it must cover the time that a start command needs to reach
	// every agent. It defaults to DefaultStartLead.
	StartLead time.Duration
	// CommandTimeout and DrainTimeout default to the constants of the same
	// names.
	CommandTimeout time.Duration
	DrainTimeout   time.Duration
	// Logger receives structured logs. The default discards them.
	Logger *slog.Logger
}

// Coordinator starts multi agent runs. It is safe for concurrent use.
type Coordinator struct {
	cfg Config
	log *slog.Logger
}

// New returns a Coordinator.
func New(cfg Config) (*Coordinator, error) {
	if cfg.Agents == nil || cfg.Clocks == nil {
		return nil, errors.New("multi agent config: agents and clocks are required")
	}
	if cfg.Compiler == nil {
		cfg.Compiler = CompilerFunc(compiler.Compile)
	}
	if cfg.Clock == nil {
		cfg.Clock = agents.SystemClock{}
	}
	if cfg.MaxClockOffset <= 0 {
		cfg.MaxClockOffset = MaxClockOffset
	}
	if cfg.StartLead <= 0 {
		cfg.StartLead = DefaultStartLead
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
	return &Coordinator{cfg: cfg, log: cfg.Logger}, nil
}

// Request describes one multi agent run.
type Request struct {
	// RunID names the run. It is also the holder of the agent reservations.
	RunID string
	// Config is the test configuration.
	Config compiler.Config
	// Agents names the agents that share the run.
	Agents []string
	// Scenarios is the catalog of installed scenarios.
	Scenarios []compiler.Scenario
	// Seed is the run seed when the configuration sets none. Zero lets the
	// compiler draw one.
	Seed uint64
}

// backendNames maps the proto backends to the configuration names.
var backendNames = map[agentpb.Backend]string{
	agentpb.Backend_BACKEND_KERNEL: "kernel",
	agentpb.Backend_BACKEND_AF_XDP: "af_xdp",
	agentpb.Backend_BACKEND_DPDK:   "dpdk",
}

// Start checks the clocks, splits the plan, and starts the run on every agent
// at one instant. It works in this order, so that a refusal leaves no trace on
// any agent:
//
//  1. Check the clock offset of every agent and refuse with ErrClockOffset.
//  2. Read the capabilities of the agents and split the test into one plan each.
//  3. Reserve the agents for the run.
//  4. Prepare every agent.
//  5. Send every agent the same start instant, converted to the clock of that
//     agent with its measured offset.
//
// When a step from 3 on fails, Start aborts the agents that hold a plan,
// releases the reservations, and returns the error. Counter snapshots reach
// the returned Run through Run.Observe.
func (c *Coordinator) Start(ctx context.Context, req Request) (*Run, error) {
	ids := slices.Compact(slices.Sorted(slices.Values(req.Agents)))
	if len(ids) == 0 {
		return nil, ErrNoAgents
	}
	if req.RunID == "" {
		return nil, errors.New("multi agent run: empty run id")
	}
	offsets, err := CheckClocks(ctx, c.cfg.Clocks, ids, c.cfg.MaxClockOffset)
	if err != nil {
		c.log.Warn("multi agent run refused", "run_id", req.RunID, "error", err)
		return nil, fmt.Errorf("start run %s: %w", req.RunID, err)
	}
	inv := make([]compiler.Agent, 0, len(ids))
	for _, id := range ids {
		a, err := c.cfg.Agents.Get(id)
		if err != nil {
			return nil, fmt.Errorf("start run %s: %w", req.RunID, err)
		}
		ca := compiler.Agent{ID: id, Cores: a.Capabilities.GetCores(), EngineAPI: a.Capabilities.GetEngineApi()}
		for _, b := range a.Capabilities.GetBackends() {
			if name, ok := backendNames[b]; ok {
				ca.Backend = name
				break
			}
		}
		for _, p := range a.Capabilities.GetPorts() {
			ca.Interfaces = append(ca.Interfaces, p.GetName())
		}
		inv = append(inv, ca)
	}
	plans, err := Split(c.cfg.Compiler, compiler.Request{
		RunID: req.RunID, Seed: req.Seed, Config: req.Config, Agents: inv, Scenarios: req.Scenarios,
	})
	if err != nil {
		return nil, fmt.Errorf("start run %s: %w", req.RunID, err)
	}
	planOf := map[string]string{}
	pb := map[string]*agentpb.Plan{}
	for _, p := range plans {
		data, err := p.MarshalCanonical()
		if err != nil {
			return nil, fmt.Errorf("start run %s: plan %s: %w", req.RunID, p.PlanID, err)
		}
		m := &agentpb.Plan{}
		if err := protojson.Unmarshal(data, m); err != nil {
			return nil, fmt.Errorf("start run %s: plan %s: proto mapping: %w", req.RunID, p.PlanID, err)
		}
		planOf[p.AgentID], pb[p.AgentID] = p.PlanID, m
	}
	r := &Run{
		c: c, id: req.RunID, ids: ids, plans: plans, offsets: offsets, planOf: planOf,
		coll: NewCollector(func(id string) string { return planOf[id] }),
	}
	if err := r.begin(ctx, pb); err != nil {
		return nil, err
	}
	return r, nil
}

// Run is a multi agent run that has started.
type Run struct {
	c       *Coordinator
	id      string
	ids     []string
	plans   []compiler.Plan
	offsets []Offset
	planOf  map[string]string
	coll    *Collector

	startAt time.Time

	mu      sync.Mutex
	stopped bool
}

// ID returns the run id.
func (r *Run) ID() string { return r.id }

// Plans returns the plans of the run, ordered by agent id.
func (r *Run) Plans() []compiler.Plan { return slices.Clone(r.plans) }

// Offsets returns the clock offsets that the start check measured, ordered by
// agent id.
func (r *Run) Offsets() []Offset { return slices.Clone(r.offsets) }

// StartAt returns the common start instant on the controller clock.
func (r *Run) StartAt() time.Time { return r.startAt }

// Collector returns the collector that sums the snapshots of the run.
func (r *Run) Collector() *Collector { return r.coll }

// Observe takes one event of the agent manager. It adds the counter snapshots
// of the agents of the run and ignores everything else. Wire it into
// agents.Config.Observer.
func (r *Run) Observe(ev agents.Event) {
	if ev.Kind != agents.EventSnapshot || ev.Snapshot == nil {
		return
	}
	if _, ok := r.planOf[ev.AgentID]; !ok {
		return
	}
	r.coll.Add(ev.AgentID, ev.Snapshot)
}

// each runs fn for every agent id in parallel and returns the errors by id.
func (r *Run) each(ids []string, fn func(id string) error) map[string]error {
	errs := make([]error, len(ids))
	var wg sync.WaitGroup
	for i, id := range ids {
		wg.Add(1)
		go func() {
			defer wg.Done()
			errs[i] = fn(id)
		}()
	}
	wg.Wait()
	out := map[string]error{}
	for i, e := range errs {
		if e != nil {
			out[ids[i]] = e
		}
	}
	return out
}

func joinByAgent(errs map[string]error) error {
	var all []error
	for _, id := range slices.Sorted(maps.Keys(errs)) {
		all = append(all, fmt.Errorf("agent %s: %w", id, errs[id]))
	}
	return errors.Join(all...)
}

// begin reserves, prepares, and starts every agent.
func (r *Run) begin(ctx context.Context, pb map[string]*agentpb.Plan) error {
	c := r.c
	cmdCtx := func() (context.Context, context.CancelFunc) {
		return context.WithTimeout(ctx, c.cfg.CommandTimeout)
	}
	var reserved []string
	fail := func(err error, prepared []string) error {
		// The caller context may be the reason for the failure, so cleanup
		// runs on a fresh one.
		cctx, cancel := context.WithTimeout(context.WithoutCancel(ctx), c.cfg.CommandTimeout)
		defer cancel()
		r.each(prepared, func(id string) error {
			_, e := c.cfg.Agents.Stop(cctx, id, &agentpb.StopCommand{PlanId: r.planOf[id], Mode: agentpb.StopMode_STOP_MODE_ABORT})
			return e
		})
		for _, id := range reserved {
			if _, e := c.cfg.Agents.Release(id, r.id); e != nil {
				c.log.Warn("release after failed start", "run_id", r.id, "agent_id", id, "error", e)
			}
		}
		return fmt.Errorf("start run %s: %w", r.id, err)
	}
	for _, id := range r.ids {
		if _, err := c.cfg.Agents.Reserve(id, r.id); err != nil {
			return fail(err, nil)
		}
		reserved = append(reserved, id)
	}
	errs := r.each(r.ids, func(id string) error {
		pctx, cancel := cmdCtx()
		defer cancel()
		_, err := c.cfg.Agents.Prepare(pctx, id, &agentpb.PrepareCommand{Plan: pb[id]})
		return err
	})
	if len(errs) > 0 {
		// An agent that answered with an error holds no plan, and one that
		// did not answer may hold one, so abort every agent. Abort on an
		// agent without a plan fails harmlessly.
		return fail(joinByAgent(errs), r.ids)
	}
	r.startAt = c.cfg.Clock.Now().Add(c.cfg.StartLead)
	byAgent := map[string]time.Duration{}
	for _, o := range r.offsets {
		byAgent[o.AgentID] = o.Offset
	}
	errs = r.each(r.ids, func(id string) error {
		sctx, cancel := cmdCtx()
		defer cancel()
		at := r.startAt.Add(byAgent[id])
		_, err := c.cfg.Agents.Start(sctx, id, &agentpb.StartCommand{PlanId: r.planOf[id], StartAt: timestamppb.New(at)})
		return err
	})
	if len(errs) > 0 {
		return fail(joinByAgent(errs), r.ids)
	}
	c.log.Info("multi agent run started", "run_id", r.id, "agents", len(r.ids), "start_at", r.startAt)
	return nil
}

// Result is the outcome of a stopped run.
type Result struct {
	// Series holds every counter and gauge summed over the agents.
	Series []Series
	// Totals holds the exact sum of every counter without labels.
	Totals map[string]uint64
	// Final lists the agents whose final snapshot arrived, ordered by id.
	Final []string
}

// Stop stops every agent, adds the final snapshot of each StopResult, releases
// the reservations, and returns the aggregate. A graceful stop waits up to
// the drain timeout, and StopMode_STOP_MODE_ABORT resets the open sessions at
// once. Stop returns the result together with an error when an agent did not
// stop or an aggregate sum failed, so the counters that arrived are not lost.
func (r *Run) Stop(ctx context.Context, mode agentpb.StopMode) (Result, error) {
	r.mu.Lock()
	if r.stopped {
		r.mu.Unlock()
		return Result{}, ErrStopped
	}
	r.stopped = true
	r.mu.Unlock()
	c := r.c
	cmdTimeout := c.cfg.CommandTimeout
	if mode != agentpb.StopMode_STOP_MODE_ABORT {
		cmdTimeout += c.cfg.DrainTimeout
	}
	errs := r.each(r.ids, func(id string) error {
		sctx, cancel := context.WithTimeout(ctx, cmdTimeout)
		defer cancel()
		res, err := c.cfg.Agents.Stop(sctx, id, &agentpb.StopCommand{
			PlanId: r.planOf[id], Mode: mode, DrainTimeout: durationpb.New(c.cfg.DrainTimeout),
		})
		if err != nil {
			return err
		}
		r.coll.Add(id, res.GetFinalSnapshot())
		return nil
	})
	for _, id := range r.ids {
		if _, err := c.cfg.Agents.Release(id, r.id); err != nil {
			c.log.Warn("release after stop", "run_id", r.id, "agent_id", id, "error", err)
		}
	}
	out := Result{}
	for _, id := range r.ids {
		if r.coll.Final(id) {
			out.Final = append(out.Final, id)
		}
	}
	var sumErr error
	if out.Series, sumErr = r.coll.Series(); sumErr == nil {
		out.Totals, sumErr = r.coll.Totals()
	}
	return out, errors.Join(joinByAgent(errs), sumErr)
}
