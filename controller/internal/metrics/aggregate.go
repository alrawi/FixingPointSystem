// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"math"
	"slices"
	"sort"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// Label is one label of a series.
type Label struct{ Name, Value string }

// Series is one merged series of a run: a catalog metric with one label set.
type Series struct {
	// Metric is the catalog entry.
	Metric *Metric
	// Labels holds the labels, ordered by name.
	Labels []Label
	// Count is the value of a counter.
	Count uint64
	// Gauge is the value of a gauge.
	Gauge float64
	// Hist is the distribution of a histogram.
	Hist Hist

	// n counts the gauge values that a non additive gauge averages.
	n int
	// key is seriesKey of the metric and labels.
	key string
}

// Label returns the value of label name, and whether the series carries it.
func (s *Series) Label(name string) (string, bool) {
	for _, l := range s.Labels {
		if l.Name == name {
			return l.Value, true
		}
	}
	return "", false
}

// LabelMap returns the labels as a map.
func (s *Series) LabelMap() map[string]string {
	out := make(map[string]string, len(s.Labels))
	for _, l := range s.Labels {
		out[l.Name] = l.Value
	}
	return out
}

// Value returns a counter or gauge as a float.
func (s *Series) Value() float64 {
	if s.Metric.Kind == Counter {
		return float64(s.Count)
	}
	return s.Gauge
}

func seriesKey(metric string, labels []Label) string {
	var b strings.Builder
	b.WriteString(metric)
	for _, l := range labels {
		b.WriteByte(0)
		b.WriteString(l.Name)
		b.WriteByte('=')
		b.WriteString(l.Value)
	}
	return b.String()
}

// sortedLabels converts a label map of a snapshot into ordered pairs and
// reports false when the catalog does not declare a label for m.
func sortedLabels(m *Metric, labels map[string]string) ([]Label, bool) {
	out := make([]Label, 0, len(labels))
	for k, v := range labels {
		if !m.HasLabel(k) {
			return nil, false
		}
		out = append(out, Label{Name: k, Value: v})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, true
}

// combine adds the value of o to s. Counters and additive gauges add, non
// additive gauges average, and histograms merge.
func (s *Series) combine(o *Series) {
	switch s.Metric.Kind {
	case Counter:
		s.Count = satAdd(s.Count, o.Count)
	case Gauge:
		if s.Metric.additive() {
			s.Gauge += o.Gauge
			return
		}
		total := s.n + o.n
		if total > 0 {
			s.Gauge = (s.Gauge*float64(s.n) + o.Gauge*float64(o.n)) / float64(total)
		}
		s.n = total
	case Histogram:
		s.Hist = Merge(s.Hist, o.Hist)
	}
}

// clone returns a copy of s with labels, which callers may own.
func (s *Series) clone(labels []Label) *Series {
	c := *s
	c.Labels = labels
	c.key = seriesKey(s.Metric.Name, labels)
	return &c
}

// group collects series under keys, combining the series of one metric.
type group struct {
	key    string
	series map[string]*Series // by metric name
}

// grouping is an ordered set of groups.
type grouping struct {
	order  []string
	groups map[string]*group
}

func newGrouping() *grouping { return &grouping{groups: map[string]*group{}} }

// add combines s into the group key under its metric, keeping labels.
func (g *grouping) add(key string, s *Series, labels []Label) {
	gr := g.groups[key]
	if gr == nil {
		gr = &group{key: key, series: map[string]*Series{}}
		g.groups[key] = gr
		g.order = append(g.order, key)
	}
	if cur := gr.series[s.Metric.Name]; cur != nil {
		cur.combine(s)
		return
	}
	gr.series[s.Metric.Name] = s.clone(labels)
}

// derive recomputes every formula gauge of the catalog that member admits
// from the counters and gauges of the group, so a ratio of a group is the
// ratio of its sums and never a mean of ratios. A formula whose denominator
// the group lacks, or whose value is undefined, leaves the gauge out.
func (gr *group) derive(c *Catalog, member func(*Metric) bool, labels []Label) {
	for _, m := range c.metrics {
		if m.formula == nil || !member(m) {
			continue
		}
		right := gr.series[m.formula.right]
		if right == nil {
			delete(gr.series, m.Name)
			continue
		}
		var l float64
		if left := gr.series[m.formula.left]; left != nil {
			l = left.Value()
		}
		v, ok := m.formula.eval(l, right.Value())
		if !ok {
			delete(gr.series, m.Name)
			continue
		}
		gr.series[m.Name] = &Series{Metric: m, Labels: labels, Gauge: v, n: 1, key: seriesKey(m.Name, labels)}
	}
}

// values renders the group as metric values keyed by catalog name, with
// histogram statistics under <metric>.<stat>.
func (gr *group) values() map[string]float64 {
	out := make(map[string]float64, len(gr.series))
	for name, s := range gr.series {
		if s.Metric.Kind == Histogram {
			for _, st := range Stats {
				out[name+"."+st.Name] = s.Hist.Value(st)
			}
			continue
		}
		// JSON numbers cannot hold NaN or an infinity, so such a gauge
		// leaves the sample.
		if v := s.Value(); !math.IsNaN(v) && !math.IsInf(v, 0) {
			out[name] = v
		}
	}
	return out
}

// sortSeries orders series by catalog position and then by labels.
func sortSeries(ss []*Series) {
	slices.SortFunc(ss, func(a, b *Series) int {
		if a.Metric.index != b.Metric.index {
			return a.Metric.index - b.Metric.index
		}
		return strings.Compare(a.key, b.key)
	})
}

// source holds the newest value of every series of one agent, or of one
// worker of an agent.
type source struct {
	agentID   string
	perWorker bool
	seq       uint64
	series    map[string]*Series
}

// absorb takes the series of snap, which is newer than every earlier
// snapshot of the source. A series that snap leaves out keeps its value,
// since the values are cumulative. It returns the number of entries that it
// dropped because the catalog does not admit them.
func (src *source) absorb(c *Catalog, snap *agentpb.CounterSnapshot, room int) (dropped int, added int) {
	src.seq = snap.GetSequence()
	put := func(s *Series) {
		if _, ok := src.series[s.key]; !ok {
			if room-added <= 0 {
				dropped++
				return
			}
			added++
		}
		src.series[s.key] = s
	}
	for _, cv := range snap.GetCounters() {
		m := c.Lookup(cv.GetMetric())
		if m == nil {
			dropped++
			continue
		}
		labels, ok := sortedLabels(m, cv.GetLabels())
		if !ok {
			dropped++
			continue
		}
		s := &Series{Metric: m, Labels: labels, key: seriesKey(m.Name, labels)}
		switch v := cv.GetValue().(type) {
		case *agentpb.CounterValue_Count:
			if m.Kind != Counter {
				dropped++
				continue
			}
			s.Count = v.Count
		case *agentpb.CounterValue_Gauge:
			if m.Kind != Gauge {
				dropped++
				continue
			}
			s.Gauge, s.n = v.Gauge, 1
		default:
			dropped++
			continue
		}
		put(s)
	}
	for _, hv := range snap.GetHistograms() {
		m := c.Lookup(hv.GetMetric())
		if m == nil || m.Kind != Histogram {
			dropped++
			continue
		}
		labels, ok := sortedLabels(m, hv.GetLabels())
		if !ok {
			dropped++
			continue
		}
		h, err := DecodeHist(hv.GetHdrV2())
		if err != nil {
			dropped++
			continue
		}
		put(&Series{Metric: m, Labels: labels, Hist: h, key: seriesKey(m.Name, labels)})
	}
	return dropped, added
}

// mergeSources merges the newest series of every source. An agent that sent
// per worker snapshots contributes only those, because per_worker replaces
// the aggregate snapshot. Formula gauges are recomputed over the merged
// counters for each label set of the gauge.
func mergeSources(c *Catalog, sources map[string]*source) []*Series {
	perWorker := map[string]bool{}
	for _, src := range sources {
		if src.perWorker {
			perWorker[src.agentID] = true
		}
	}
	keys := make([]string, 0, len(sources))
	for k := range sources {
		keys = append(keys, k)
	}
	sort.Strings(keys) // a fixed order keeps float sums reproducible
	merged := map[string]*Series{}
	for _, k := range keys {
		src := sources[k]
		if !src.perWorker && perWorker[src.agentID] {
			continue
		}
		for key, s := range src.series {
			if s.Metric.formula != nil {
				continue
			}
			if cur := merged[key]; cur != nil {
				cur.combine(s)
			} else {
				merged[key] = s.clone(s.Labels)
			}
		}
	}
	out := make([]*Series, 0, len(merged))
	for _, s := range merged {
		out = append(out, s)
	}
	out = append(out, deriveSeries(c, out)...)
	sortSeries(out)
	return out
}

// deriveSeries computes every formula gauge for each label set that its
// denominator projects to.
func deriveSeries(c *Catalog, merged []*Series) []*Series {
	var out []*Series
	for _, m := range c.metrics {
		if m.formula == nil {
			continue
		}
		g := newGrouping()
		labelsOf := map[string][]Label{}
		for _, s := range merged {
			if s.Metric.Name != m.formula.left && s.Metric.Name != m.formula.right {
				continue
			}
			proj := project(s.Labels, m.Labels)
			k := seriesKey("", proj)
			labelsOf[k] = proj
			g.add(k, s, proj)
		}
		for _, k := range g.order {
			gr := g.groups[k]
			gr.derive(c, func(x *Metric) bool { return x == m }, labelsOf[k])
			if s := gr.series[m.Name]; s != nil {
				out = append(out, s)
			}
		}
	}
	return out
}

// project keeps the labels whose names keep lists.
func project(labels []Label, keep []string) []Label {
	var out []Label
	for _, l := range labels {
		if slices.Contains(keep, l.Name) {
			out = append(out, l)
		}
	}
	return out
}
