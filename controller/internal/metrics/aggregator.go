// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"errors"
	"fmt"
	"log/slog"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// Defaults of Config.
const (
	DefaultMaxRuns        = 64
	DefaultMaxPoints      = 24 * 60 * 60
	DefaultMaxSeries      = 100_000
	DefaultStreamInterval = time.Second
)

// ErrRunUnknown reports a run that the aggregator holds no metrics for.
var ErrRunUnknown = errors.New("run has no metrics")

// Config configures an Aggregator. Every field is optional.
type Config struct {
	// Catalog is the metric catalog. It defaults to DefaultCatalog.
	Catalog *Catalog
	// Clock tells the time of Add calls without a time and of the stream.
	// It defaults to agents.SystemClock.
	Clock agents.Clock
	// Logger receives structured logs. The default discards them.
	Logger *slog.Logger
	// RunOf maps the agent and plan id of a snapshot to its run id. The
	// default strips the suffix /<agent id> that the compiler appends.
	RunOf func(agentID, planID string) string
	// MaxRuns bounds the runs held in memory. The least recently updated
	// run goes first. It defaults to DefaultMaxRuns.
	MaxRuns int
	// MaxPoints bounds the one second points of each run. It defaults to
	// DefaultMaxPoints, one day.
	MaxPoints int
	// MaxSeries bounds the distinct series of each run. It defaults to
	// DefaultMaxSeries.
	MaxSeries int
	// StreamInterval is the event period of the metrics stream. It
	// defaults to DefaultStreamInterval.
	StreamInterval time.Duration
}

// point is the state of the views at the end of one second.
type point struct {
	ts    time.Time
	views map[api.MetricView][]Sample
}

// runMetrics is the metric state of one run.
type runMetrics struct {
	id      string
	sources map[string]*source
	series  int
	dropped int
	points  []point
	dirty   bool
	updated time.Time
	// merged caches mergeSources until the next snapshot.
	merged []*Series
}

// Aggregator merges the counter snapshots of every agent of a run into run
// metrics. It keeps the newest snapshot of every agent and worker, so a
// repeated or late snapshot changes nothing, and records the views once per
// second for windowed queries. It is safe for concurrent use.
type Aggregator struct {
	cfg Config
	log *slog.Logger

	mu   sync.Mutex
	runs map[string]*runMetrics
}

// New returns an Aggregator. Pass Observe as, or call it from,
// agents.Config.Observer.
func New(cfg Config) (*Aggregator, error) {
	if cfg.Catalog == nil {
		c, err := DefaultCatalog()
		if err != nil {
			return nil, err
		}
		cfg.Catalog = c
	}
	if cfg.Clock == nil {
		cfg.Clock = agents.SystemClock{}
	}
	if cfg.RunOf == nil {
		cfg.RunOf = RunOfPlan
	}
	if cfg.MaxRuns <= 0 {
		cfg.MaxRuns = DefaultMaxRuns
	}
	if cfg.MaxPoints <= 0 {
		cfg.MaxPoints = DefaultMaxPoints
	}
	if cfg.MaxSeries <= 0 {
		cfg.MaxSeries = DefaultMaxSeries
	}
	if cfg.StreamInterval <= 0 {
		cfg.StreamInterval = DefaultStreamInterval
	}
	log := cfg.Logger
	if log == nil {
		log = slog.New(slog.DiscardHandler)
	}
	return &Aggregator{cfg: cfg, log: log, runs: map[string]*runMetrics{}}, nil
}

// RunOfPlan returns the run id of a plan id of the form <run id>/<agent id>,
// as the compiler names plans, or the part before the first slash.
func RunOfPlan(agentID, planID string) string {
	if run, ok := strings.CutSuffix(planID, "/"+agentID); ok && run != "" {
		return run
	}
	run, _, _ := strings.Cut(planID, "/")
	return run
}

// Catalog returns the metric catalog.
func (a *Aggregator) Catalog() *Catalog { return a.cfg.Catalog }

// Observe takes one event of the agent manager and adds the snapshot of an
// EventSnapshot. It returns quickly, as agents.Config.Observer requires.
func (a *Aggregator) Observe(ev agents.Event) {
	if ev.Kind == agents.EventSnapshot && ev.Snapshot != nil {
		a.Add(ev.AgentID, ev.Time, ev.Snapshot)
	}
}

// Add takes one counter snapshot of agent agentID at controller time at, for
// example the final snapshot of a StopResult, and reports whether it was
// newer than every earlier snapshot of the same agent and worker. A zero at
// takes the time from Config.Clock.
func (a *Aggregator) Add(agentID string, at time.Time, snap *agentpb.CounterSnapshot) bool {
	if snap == nil || agentID == "" {
		return false
	}
	runID := a.cfg.RunOf(agentID, snap.GetPlanId())
	if runID == "" {
		a.log.Debug("snapshot without run dropped", "agent_id", agentID, "plan_id", snap.GetPlanId())
		return false
	}
	if at.IsZero() {
		at = a.cfg.Clock.Now()
	}
	sec := at.UTC().Truncate(time.Second)

	a.mu.Lock()
	defer a.mu.Unlock()
	r := a.runs[runID]
	if r == nil {
		r = &runMetrics{id: runID, sources: map[string]*source{}, updated: sec}
		a.runs[runID] = r
		a.evict(r)
	}
	srcKey := agentID + "\x00aggregate"
	if !snap.GetAggregate() {
		srcKey = agentID + "\x00" + strconv.FormatUint(uint64(snap.GetWorkerId()), 10)
	}
	src := r.sources[srcKey]
	if src != nil && snap.GetSequence() <= src.seq {
		return false
	}
	// A new second closes the previous point with the state before this
	// snapshot.
	if n := len(r.points); n > 0 && sec.After(r.points[n-1].ts) && r.dirty {
		a.finalize(r)
	}
	if src == nil {
		src = &source{agentID: agentID, perWorker: !snap.GetAggregate(), series: map[string]*Series{}}
		r.sources[srcKey] = src
	}
	dropped, added := src.absorb(a.cfg.Catalog, snap, a.cfg.MaxSeries-r.series)
	r.series += added
	if dropped > 0 {
		r.dropped += dropped
		a.log.Debug("snapshot entries dropped", "run_id", runID, "agent_id", agentID, "sequence", snap.GetSequence(), "dropped", dropped)
	}
	r.merged = nil
	r.updated = sec
	if n := len(r.points); n == 0 || sec.After(r.points[n-1].ts) {
		r.points = append(r.points, point{ts: sec})
		if len(r.points) > a.cfg.MaxPoints {
			r.points = append(r.points[:0:0], r.points[len(r.points)-a.cfg.MaxPoints:]...)
		}
	}
	r.dirty = true
	return true
}

// evict drops the least recently updated runs beyond MaxRuns, never keep.
// The caller holds the lock.
func (a *Aggregator) evict(keep *runMetrics) {
	for len(a.runs) > a.cfg.MaxRuns {
		var oldest *runMetrics
		for _, r := range a.runs {
			if r == keep {
				continue
			}
			if oldest == nil || r.updated.Before(oldest.updated) || (r.updated.Equal(oldest.updated) && r.id < oldest.id) {
				oldest = r
			}
		}
		delete(a.runs, oldest.id)
		a.log.Info("run metrics evicted", "run_id", oldest.id)
	}
}

// mergedLocked returns the merged series of r. The caller holds the lock.
func (a *Aggregator) mergedLocked(r *runMetrics) []*Series {
	if r.merged == nil {
		r.merged = mergeSources(a.cfg.Catalog, r.sources)
	}
	return r.merged
}

// finalize computes the views of the newest point of r. The caller holds
// the lock.
func (a *Aggregator) finalize(r *runMetrics) {
	if !r.dirty || len(r.points) == 0 {
		return
	}
	p := &r.points[len(r.points)-1]
	merged := a.mergedLocked(r)
	p.views = make(map[api.MetricView][]Sample, len(Views))
	for _, v := range Views {
		p.views[v] = computeView(a.cfg.Catalog, merged, v, p.ts)
	}
	r.dirty = false
}

// Forget drops the metrics of a run.
func (a *Aggregator) Forget(runID string) {
	a.mu.Lock()
	defer a.mu.Unlock()
	delete(a.runs, runID)
}

// Runs returns the ids of the runs that the aggregator holds, in order.
func (a *Aggregator) Runs() []string {
	a.mu.Lock()
	defer a.mu.Unlock()
	out := make([]string, 0, len(a.runs))
	for id := range a.runs {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

// Has reports whether the aggregator holds metrics of a run.
func (a *Aggregator) Has(runID string) bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.runs[runID] != nil
}

// Series returns the merged series of a run, ordered by catalog position and
// labels, with derived gauges. Callers must not modify them.
func (a *Aggregator) Series(runID string) ([]*Series, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	r := a.runs[runID]
	if r == nil {
		return nil, fmt.Errorf("series of run %s: %w", runID, ErrRunUnknown)
	}
	return a.mergedLocked(r), nil
}

// Totals returns the metrics of a run aggregated over every label, keyed by
// catalog name, with histogram statistics under <metric>.<stat> as metric
// references in test criteria name them.
func (a *Aggregator) Totals(runID string) (map[string]float64, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	r := a.runs[runID]
	if r == nil {
		return nil, fmt.Errorf("totals of run %s: %w", runID, ErrRunUnknown)
	}
	return totals(a.cfg.Catalog, a.mergedLocked(r)), nil
}

// Window selects the points of a metrics query.
type Window struct {
	// From and To bound the point times, inclusive. Zero means open.
	From, To time.Time
	// Step keeps the newest point of every step, counted from From or from
	// the first point. Zero or one second keeps every point.
	Step time.Duration
}

// Samples returns the samples of view v of a run over the window, in time
// order, then in view key order.
func (a *Aggregator) Samples(runID string, v api.MetricView, w Window) ([]Sample, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	r := a.runs[runID]
	if r == nil {
		return nil, fmt.Errorf("samples of run %s: %w", runID, ErrRunUnknown)
	}
	a.finalize(r)
	var base time.Time
	if !w.From.IsZero() {
		base = w.From
	}
	var picked []*point
	lastBucket := int64(-1)
	for i := range r.points {
		p := &r.points[i]
		if (!w.From.IsZero() && p.ts.Before(w.From)) || (!w.To.IsZero() && p.ts.After(w.To)) {
			continue
		}
		if base.IsZero() {
			base = p.ts
		}
		if w.Step <= time.Second {
			picked = append(picked, p)
			continue
		}
		b := int64(p.ts.Sub(base) / w.Step)
		if b == lastBucket {
			picked[len(picked)-1] = p
		} else {
			picked = append(picked, p)
			lastBucket = b
		}
	}
	out := []Sample{}
	for _, p := range picked {
		out = append(out, p.views[v]...)
	}
	return out, nil
}

// Latest returns the current samples of view v of a run.
func (a *Aggregator) Latest(runID string, v api.MetricView) ([]Sample, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	r := a.runs[runID]
	if r == nil {
		return nil, fmt.Errorf("latest samples of run %s: %w", runID, ErrRunUnknown)
	}
	a.finalize(r)
	if len(r.points) == 0 {
		return nil, nil
	}
	return r.points[len(r.points)-1].views[v], nil
}

// Dropped returns the number of snapshot entries of a run that the
// aggregator dropped because the catalog does not admit them, their
// histogram encoding was malformed, or the run reached MaxSeries.
func (a *Aggregator) Dropped(runID string) int {
	a.mu.Lock()
	defer a.mu.Unlock()
	if r := a.runs[runID]; r != nil {
		return r.dropped
	}
	return 0
}
