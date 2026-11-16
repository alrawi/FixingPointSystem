// SPDX-License-Identifier: GPL-3.0-only

package multiagent

import (
	"errors"
	"fmt"
	"maps"
	"math/bits"
	"slices"
	"strings"
	"sync"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// ErrCounterOverflow reports that the sum of a counter over the agents does
// not fit in 64 bits.
var ErrCounterOverflow = errors.New("counter sum overflows 64 bits")

// Series is one counter or gauge of the run, summed over the agents.
type Series struct {
	// Metric is the catalog metric name.
	Metric string
	// Labels qualify the value, and are never nil.
	Labels map[string]string
	// Gauge is true for an instantaneous value, which is summed as a float.
	Gauge bool
	// Count is the exact sum of a counter over the agents.
	Count uint64
	// Value is the sum of a gauge over the agents. It is zero for a counter.
	Value float64
}

// seriesKey returns a key that is equal for equal metric and label sets.
func seriesKey(metric string, labels map[string]string) string {
	var b strings.Builder
	b.WriteString(metric)
	for _, k := range slices.Sorted(maps.Keys(labels)) {
		fmt.Fprintf(&b, "\x00%s\x01%s", k, labels[k])
	}
	return b.String()
}

// source is the newest state of one stream of snapshots, which is the
// aggregate stream of an agent or one of its workers.
type source struct {
	seq    uint64
	series map[string]Series
}

// Collector sums the counter snapshots of the agents of one run. Counters
// are cumulative from the start of the run, so the newest snapshot of a
// stream replaces the older ones, and a snapshot that omits a counter keeps
// the last value of that counter. The sum of counters is exact, in unsigned
// 64 bit integers, and does not pass through a float. A Collector is safe for
// concurrent use.
type Collector struct {
	planOf func(agentID string) string

	mu      sync.Mutex
	sources map[string]*source
	// final records the agents whose final snapshot arrived.
	final map[string]bool
}

// NewCollector returns a Collector. planOf names the plan id that each agent
// runs, and the collector ignores snapshots of any other plan, such as late
// snapshots of an earlier run. A nil planOf accepts every plan.
func NewCollector(planOf func(agentID string) string) *Collector {
	return &Collector{planOf: planOf, sources: map[string]*source{}, final: map[string]bool{}}
}

// Add takes a snapshot of an agent and reports whether it changed the state.
// It drops a snapshot of another plan, one that is not newer than the newest
// of its stream, and a nil snapshot. A stream is the aggregate snapshots of
// the agent, or the snapshots of one worker when the run asks for per worker
// snapshots.
func (c *Collector) Add(agentID string, snap *agentpb.CounterSnapshot) bool {
	if snap == nil {
		return false
	}
	if c.planOf != nil && snap.GetPlanId() != c.planOf(agentID) {
		return false
	}
	key := agentID
	if !snap.GetAggregate() && snap.GetWorkerId() != 0 {
		key = fmt.Sprintf("%s\x00worker%d", agentID, snap.GetWorkerId())
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	src := c.sources[key]
	if src == nil {
		src = &source{series: map[string]Series{}}
		c.sources[key] = src
	}
	if snap.GetSequence() <= src.seq {
		return false
	}
	src.seq = snap.GetSequence()
	for _, v := range snap.GetCounters() {
		labels := maps.Clone(v.GetLabels())
		if labels == nil {
			labels = map[string]string{}
		}
		s := Series{Metric: v.GetMetric(), Labels: labels}
		switch x := v.GetValue().(type) {
		case *agentpb.CounterValue_Count:
			s.Count = x.Count
		case *agentpb.CounterValue_Gauge:
			s.Gauge, s.Value = true, x.Gauge
		default:
			continue
		}
		src.series[seriesKey(s.Metric, labels)] = s
	}
	if snap.GetIsFinal() {
		c.final[agentID] = true
	}
	return true
}

// Final reports whether the final snapshot of agent id arrived.
func (c *Collector) Final(agentID string) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.final[agentID]
}

// Series returns the sum of every counter and gauge over all streams, ordered
// by metric and then by labels. It fails with ErrCounterOverflow when a
// counter sum does not fit in 64 bits.
func (c *Collector) Series() ([]Series, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	sums := map[string]*Series{}
	for _, id := range slices.Sorted(maps.Keys(c.sources)) {
		src := c.sources[id]
		for _, k := range slices.Sorted(maps.Keys(src.series)) {
			s := src.series[k]
			t := sums[k]
			if t == nil {
				cp := Series{Metric: s.Metric, Labels: maps.Clone(s.Labels), Gauge: s.Gauge}
				t = &cp
				sums[k] = t
			}
			if t.Gauge != s.Gauge {
				return nil, fmt.Errorf("metric %s is a counter on one agent and a gauge on another", s.Metric)
			}
			if s.Gauge {
				t.Value += s.Value
				continue
			}
			sum, carry := bits.Add64(t.Count, s.Count, 0)
			if carry != 0 {
				return nil, fmt.Errorf("metric %s: %w", s.Metric, ErrCounterOverflow)
			}
			t.Count = sum
		}
	}
	out := make([]Series, 0, len(sums))
	for _, k := range slices.Sorted(maps.Keys(sums)) {
		out = append(out, *sums[k])
	}
	return out, nil
}

// Totals returns the sum of every counter that has no labels, by metric name.
// It is the input of the criteria of a run. Counters with labels stay in
// Series, and their sum over the labels is not part of Totals, because the
// catalog gives some labeled metrics their own unlabeled totals.
func (c *Collector) Totals() (map[string]uint64, error) {
	all, err := c.Series()
	if err != nil {
		return nil, err
	}
	out := map[string]uint64{}
	for _, s := range all {
		if !s.Gauge && len(s.Labels) == 0 {
			out[s.Metric] = s.Count
		}
	}
	return out, nil
}
