// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"fmt"
	"maps"
	"math"
	"slices"
	"strconv"
	"strings"
	"sync"

	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// Status is the result of one criterion.
type Status string

// The four results of a criterion. Only Fail and Invalid fail the run.
const (
	// Pass means that the observed value satisfies the rule.
	Pass Status = "pass"
	// Fail means that the observed value breaks the rule.
	Fail Status = "fail"
	// Missing means that the run has no value for a catalog metric, for
	// example a ratio whose divisor is zero or a gauge that no component
	// computed. Such a criterion does not decide the verdict.
	Missing Status = "missing"
	// Invalid means that the criterion names no catalog metric or statistic,
	// an unknown operator, or a label that its metric lacks. Such a
	// criterion fails the run, because the rule cannot hold.
	Invalid Status = "invalid"
)

// Operators lists the criterion operators of
// contracts/schemas/test-config.schema.json in schema order.
var Operators = []string{"<", "<=", ">", ">=", "==", "!="}

// Result is the result of one criterion.
type Result struct {
	runs.Criterion
	// Status is the result.
	Status Status
	// Observed is the value of the metric. It is meaningful when Status is
	// Pass or Fail.
	Observed float64
	// Reason explains the status in one lowercase sentence.
	Reason string
}

// Evaluated reports whether the criterion decided by comparing a value.
func (r Result) Evaluated() bool { return r.Status == Pass || r.Status == Fail }

// Evaluation is the result of the criteria of a run.
type Evaluation struct {
	// Passed is true when no criterion failed or was invalid.
	Passed bool
	// Results holds one result per criterion, in configuration order.
	Results []Result
}

// SeriesSource returns the merged series of a run, with their labels.
// *metrics.Aggregator satisfies it, and an error that matches
// metrics.ErrRunUnknown means that it holds no metrics of the run.
type SeriesSource interface {
	Series(runID string) ([]*metrics.Series, error)
}

var _ SeriesSource = (*metrics.Aggregator)(nil)

// Data holds the metrics of a run that criteria are evaluated over.
//
// With Labeled false, Totals holds the metrics aggregated over every label,
// keyed by catalog name with histogram statistics under <metric>.<stat>, and
// a criterion cannot select labels. With Labeled true, Series holds every
// series of the run and each criterion selects among them by its where
// filter. Without a side in the filter, a criterion on a metric that has the
// side label counts the client side only, because a total over both sides
// counts every session twice.
type Data struct {
	Totals  map[string]float64
	Series  []*metrics.Series
	Labeled bool
}

// Evaluator evaluates criteria with the metric names of a catalog.
type Evaluator struct {
	cat *metrics.Catalog
	// formulas holds the parsed formulas of the derived gauges, in catalog
	// order, so a formula over another derived gauge sees its new value.
	formulas []derived
	stats    map[string]bool

	mu  sync.RWMutex
	src SeriesSource
}

// derived is one derived gauge of the catalog.
type derived struct {
	name, left, right string
	op                byte
}

// NewEvaluator returns an Evaluator over cat. A nil cat means
// metrics.DefaultCatalog.
func NewEvaluator(cat *metrics.Catalog) (*Evaluator, error) {
	if cat == nil {
		c, err := metrics.DefaultCatalog()
		if err != nil {
			return nil, fmt.Errorf("criteria evaluator: %w", err)
		}
		cat = c
	}
	e := &Evaluator{cat: cat, stats: map[string]bool{}}
	for _, s := range metrics.Stats {
		e.stats[s.Name] = true
	}
	for _, m := range cat.Metrics() {
		if m.Formula == "" {
			continue
		}
		f := strings.Fields(m.Formula)
		if len(f) != 3 || len(f[1]) != 1 || !strings.Contains("+-*/", f[1]) {
			return nil, fmt.Errorf("criteria evaluator: metric %s: formula %q is not of the form <metric> <op> <metric>", m.Name, m.Formula)
		}
		e.formulas = append(e.formulas, derived{name: m.Name, left: f[0], op: f[1][0], right: f[2]})
	}
	return e, nil
}

// UseSeries sets the source that RunEvaluator reads by run id when a run
// stops, so that histogram statistics and labeled metrics decide the state
// of the run as they decide its report. NewAPI calls it with the totals
// source of the report when that source offers series. A nil src clears it.
func (e *Evaluator) UseSeries(src SeriesSource) {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.src = src
}

func (e *Evaluator) source() SeriesSource {
	e.mu.RLock()
	defer e.mu.RUnlock()
	return e.src
}

// Catalog returns the metric catalog of e.
func (e *Evaluator) Catalog() *metrics.Catalog { return e.cat }

// Derive returns a copy of totals in which every derived gauge of the
// catalog, such as sessions.failed_ratio, is recomputed from the merged
// counters with its catalog formula. A derived gauge whose divisor is zero or
// whose divisor is absent leaves the result, because its value is undefined.
// A missing dividend counts as zero, as in the metrics aggregator.
func (e *Evaluator) Derive(totals map[string]float64) map[string]float64 {
	out := maps.Clone(totals)
	if out == nil {
		out = map[string]float64{}
	}
	for _, f := range e.formulas {
		r, ok := out[f.right]
		if !ok {
			delete(out, f.name)
			continue
		}
		l := out[f.left]
		v, ok := apply(f.op, l, r)
		if !ok {
			delete(out, f.name)
			continue
		}
		out[f.name] = v
	}
	return out
}

func apply(op byte, l, r float64) (float64, bool) {
	switch op {
	case '+':
		return l + r, true
	case '-':
		return l - r, true
	case '*':
		return l * r, true
	case '/':
		if r == 0 {
			return 0, false
		}
		return l / r, true
	}
	return 0, false
}

// Evaluate evaluates criteria over totals, which hold run metrics keyed by
// catalog name with histogram statistics under <metric>.<stat>. It derives
// the catalog formulas first, so a criterion on a ratio sees the ratio of the
// merged counters and never a mean of per agent ratios. Totals carry no
// labels, so a criterion with a where filter has no result.
func (e *Evaluator) Evaluate(criteria []runs.Criterion, totals map[string]float64) Evaluation {
	return e.EvaluateData(criteria, Data{Totals: totals})
}

// EvaluateData evaluates criteria over d.
func (e *Evaluator) EvaluateData(criteria []runs.Criterion, d Data) Evaluation {
	var values map[string]float64
	if !d.Labeled {
		values = e.Derive(d.Totals)
	}
	ev := Evaluation{Passed: true, Results: make([]Result, 0, len(criteria))}
	for _, c := range criteria {
		res := e.one(c, d, values)
		if res.Status == Fail || res.Status == Invalid {
			ev.Passed = false
		}
		ev.Results = append(ev.Results, res)
	}
	return ev
}

// one evaluates one criterion. values holds the derived totals of unlabeled
// data.
func (e *Evaluator) one(c runs.Criterion, d Data, values map[string]float64) Result {
	res := Result{Criterion: c}
	if !slices.Contains(Operators, c.Op) {
		res.Status, res.Reason = Invalid, fmt.Sprintf("unknown operator %q", c.Op)
		return res
	}
	m, stat, reason := e.resolve(c.Metric)
	if reason == "" {
		reason = checkWhere(m, c.Where)
	}
	if reason != "" {
		res.Status, res.Reason = Invalid, reason
		return res
	}
	var obs float64
	var ok bool
	suffix := ""
	switch {
	case d.Labeled:
		obs, ok = e.seriesValue(d.Series, m, stat, c.Where)
		if text := filterText(m, c.Where); text != "" {
			suffix = " for " + text
		}
	case len(c.Where) > 0:
		res.Status, res.Reason = Missing, "the run has no labeled metrics to select from"
		return res
	default:
		obs, ok = values[c.Metric]
	}
	if !ok || math.IsNaN(obs) || math.IsInf(obs, 0) {
		res.Status = Missing
		if d.Labeled {
			res.Reason = e.missingSeriesReason(d.Series, m, c.Where) + suffix
		} else {
			res.Reason = e.missingReason(c.Metric, values)
		}
		return res
	}
	res.Observed = obs
	if compare(obs, c.Op, c.Value) {
		res.Status = Pass
		res.Reason = fmt.Sprintf("observed %s %s %s holds%s", formatNumber(obs), c.Op, formatNumber(c.Value), suffix)
	} else {
		res.Status = Fail
		res.Reason = fmt.Sprintf("observed %s %s %s does not hold%s", formatNumber(obs), c.Op, formatNumber(c.Value), suffix)
	}
	return res
}

// resolve returns the catalog metric that a criterion reference names, with
// the histogram statistic when the reference has one, or why name is no
// metric reference of the catalog. A histogram needs a statistic suffix, and
// only a histogram takes one.
func (e *Evaluator) resolve(name string) (m *metrics.Metric, stat, reason string) {
	if m := e.cat.Lookup(name); m != nil {
		if m.Kind == metrics.Histogram {
			return nil, "", fmt.Sprintf("histogram %s needs a statistic suffix such as .p99", name)
		}
		return m, "", ""
	}
	base, st, ok := cutLast(name)
	if ok && e.stats[st] {
		m := e.cat.Lookup(base)
		switch {
		case m == nil:
		case m.Kind == metrics.Histogram:
			return m, st, ""
		default:
			return nil, "", fmt.Sprintf("statistic %s applies only to histograms, and %s is a %s", st, base, m.Kind)
		}
	}
	return nil, "", fmt.Sprintf("metric %s is not in the metric catalog", name)
}

// checkWhere returns why where does not fit m, or "". Every name of the
// filter must be a label of the metric.
func checkWhere(m *metrics.Metric, where map[string]string) string {
	for _, k := range slices.Sorted(maps.Keys(where)) {
		if !m.HasLabel(k) {
			return fmt.Sprintf("metric %s has no label %s", m.Name, k)
		}
	}
	return ""
}

// defaultsToClient reports whether a criterion on m with the filter where
// counts the client side only, because m has the side label and the filter
// names no side.
func defaultsToClient(m *metrics.Metric, where map[string]string) bool {
	_, named := where[runs.LabelSide]
	return m.HasLabel(runs.LabelSide) && !named
}

// filterText words the label filter that a criterion applies, with the side
// default, as name=value pairs in name order, or "" for no filter.
func filterText(m *metrics.Metric, where map[string]string) string {
	f := maps.Clone(where)
	if f == nil {
		f = map[string]string{}
	}
	if defaultsToClient(m, where) {
		f[runs.LabelSide] = runs.SideClient
	}
	pairs := make([]string, 0, len(f))
	for _, k := range slices.Sorted(maps.Keys(f)) {
		pairs = append(pairs, k+"="+f[k])
	}
	return strings.Join(pairs, ",")
}

// pick returns the series of m that a criterion counts. A series must carry
// every label value of where. When m has the side label and where names no
// side, a series that carries another side than the client is left out. A
// series without any side label stays, because it predates the label. A
// series without any label is a run total that repeats the labeled series of
// its metric, so it counts only when the run has no labeled series of m.
func pick(series []*metrics.Series, m *metrics.Metric, where map[string]string) []*metrics.Series {
	client := defaultsToClient(m, where)
	labeled := false
	for _, s := range series {
		if s.Metric.Name == m.Name && len(s.Labels) > 0 {
			labeled = true
			break
		}
	}
	var out []*metrics.Series
next:
	for _, s := range series {
		if s.Metric.Name != m.Name || (labeled && len(s.Labels) == 0) {
			continue
		}
		for k, want := range where {
			if got, ok := s.Label(k); !ok || got != want {
				continue next
			}
		}
		if got, ok := s.Label(runs.LabelSide); client && ok && got != runs.SideClient {
			continue
		}
		out = append(out, s)
	}
	return out
}

// additive reports whether gauge values add up, as the metrics aggregator
// decides it. Ratios and percentages combine by their mean instead.
func additive(m *metrics.Metric) bool {
	switch m.Unit {
	case "ratio", "percent", "percentage_points":
		return false
	}
	return true
}

// formulaOf returns the formula of the derived gauge name, or nil.
func (e *Evaluator) formulaOf(name string) *derived {
	for i := range e.formulas {
		if e.formulas[i].name == name {
			return &e.formulas[i]
		}
	}
	return nil
}

// seriesValue returns the value of metric m, or of its histogram statistic
// stat, over the series that where selects. Counters and additive gauges add,
// other gauges average, and histograms merge. A derived gauge divides the
// sums of its components, never a mean of ratios. It reports false when the
// selection holds no series or the value is undefined.
func (e *Evaluator) seriesValue(series []*metrics.Series, m *metrics.Metric, stat string, where map[string]string) (float64, bool) {
	if f := e.formulaOf(m.Name); f != nil {
		lm, rm := e.cat.Lookup(f.left), e.cat.Lookup(f.right)
		if lm == nil || rm == nil {
			return 0, false
		}
		r, ok := e.seriesValue(series, rm, "", where)
		if !ok {
			return 0, false
		}
		l, _ := e.seriesValue(series, lm, "", where)
		return apply(f.op, l, r)
	}
	sel := pick(series, m, where)
	if len(sel) == 0 {
		return 0, false
	}
	switch m.Kind {
	case metrics.Counter:
		var sum float64
		for _, s := range sel {
			sum += float64(s.Count)
		}
		return sum, true
	case metrics.Gauge:
		var sum float64
		for _, s := range sel {
			sum += s.Gauge
		}
		if !additive(m) {
			sum /= float64(len(sel))
		}
		return sum, true
	case metrics.Histogram:
		hs := make([]metrics.Hist, len(sel))
		for i, s := range sel {
			hs[i] = s.Hist
		}
		h := metrics.Merge(hs...)
		for _, st := range metrics.Stats {
			if st.Name == stat {
				return h.Value(st), true
			}
		}
	}
	return 0, false
}

// missingSeriesReason explains why metric m has no value over the series that
// where selects.
func (e *Evaluator) missingSeriesReason(series []*metrics.Series, m *metrics.Metric, where map[string]string) string {
	if f := e.formulaOf(m.Name); f != nil {
		if rm := e.cat.Lookup(f.right); rm != nil {
			if r, ok := e.seriesValue(series, rm, "", where); ok && r == 0 && f.op == '/' {
				return fmt.Sprintf("undefined because %s is 0", f.right)
			}
		}
		return fmt.Sprintf("undefined because the run reported no %s", f.right)
	}
	return "the run reported no value"
}

// seriesTotals returns the run metrics of series as totals, keyed by catalog
// name with histogram statistics under <metric>.<stat>. Every metric counts
// the client side only where it has the side label, as a criterion without a
// side does, so the totals count each session once.
func (e *Evaluator) seriesTotals(series []*metrics.Series) map[string]float64 {
	out := map[string]float64{}
	put := func(name string, m *metrics.Metric, stat string) {
		if v, ok := e.seriesValue(series, m, stat, nil); ok && !math.IsNaN(v) && !math.IsInf(v, 0) {
			out[name] = v
		}
	}
	for _, m := range e.cat.Metrics() {
		if m.Kind != metrics.Histogram {
			put(m.Name, m, "")
			continue
		}
		for _, st := range metrics.Stats {
			put(m.Name+"."+st.Name, m, st.Name)
		}
	}
	return out
}

// seriesKey identifies a series by metric and labels.
func seriesKey(metric string, labels map[string]string) string {
	var b strings.Builder
	b.WriteString(metric)
	for _, k := range slices.Sorted(maps.Keys(labels)) {
		b.WriteByte(0)
		b.WriteString(k)
		b.WriteByte('=')
		b.WriteString(labels[k])
	}
	return b.String()
}

// mergeSeries returns the series of the aggregator with the values that the
// orchestrator holds. The counters of orch take precedence, because the
// orchestrator counts the open sessions of a failed agent as failed and sees
// the final snapshot of every agent. A gauge that only the orchestrator
// reports, in gauges, joins as a series without labels. Histograms and every
// other gauge come from the aggregator.
func (e *Evaluator) mergeSeries(agg []*metrics.Series, orch []runs.Series, gauges map[string]float64) []*metrics.Series {
	out := slices.Clone(agg)
	index := make(map[string]int, len(out))
	have := map[string]bool{}
	for i, s := range out {
		index[seriesKey(s.Metric.Name, s.LabelMap())] = i
		have[s.Metric.Name] = true
	}
	for _, o := range orch {
		m := e.cat.Lookup(o.Metric)
		if m == nil || m.Kind != metrics.Counter || o.Value < 0 || o.Value >= 1<<63 {
			continue
		}
		labels, ok := catalogLabels(m, o.Labels)
		if !ok {
			continue
		}
		count := uint64(o.Value)
		key := seriesKey(o.Metric, o.Labels)
		if i, found := index[key]; found {
			c := *out[i]
			c.Count = count
			out[i] = &c
			continue
		}
		index[key] = len(out)
		have[m.Name] = true
		out = append(out, &metrics.Series{Metric: m, Labels: labels, Count: count})
	}
	for _, name := range slices.Sorted(maps.Keys(gauges)) {
		m := e.cat.Lookup(name)
		if m == nil || m.Kind != metrics.Gauge || m.Formula != "" || have[name] {
			continue
		}
		out = append(out, &metrics.Series{Metric: m, Gauge: gauges[name]})
	}
	return out
}

// catalogLabels returns the labels in name order, and false when the catalog
// does not declare one of them for m.
func catalogLabels(m *metrics.Metric, labels map[string]string) ([]metrics.Label, bool) {
	out := make([]metrics.Label, 0, len(labels))
	for _, k := range slices.Sorted(maps.Keys(labels)) {
		if !m.HasLabel(k) {
			return nil, false
		}
		out = append(out, metrics.Label{Name: k, Value: labels[k]})
	}
	return out, true
}

// missingReason explains why a valid metric has no value.
func (e *Evaluator) missingReason(name string, values map[string]float64) string {
	for _, f := range e.formulas {
		if f.name != name {
			continue
		}
		if r, ok := values[f.right]; ok && r == 0 && f.op == '/' {
			return fmt.Sprintf("undefined because %s is 0", f.right)
		}
		return fmt.Sprintf("undefined because the run reported no %s", f.right)
	}
	return "the run reported no value"
}

func cutLast(name string) (base, last string, ok bool) {
	i := strings.LastIndexByte(name, '.')
	if i < 0 {
		return "", "", false
	}
	return name[:i], name[i+1:], true
}

// compare applies the criterion operator op.
func compare(observed float64, op string, value float64) bool {
	switch op {
	case "<":
		return observed < value
	case "<=":
		return observed <= value
	case ">":
		return observed > value
	case ">=":
		return observed >= value
	case "==":
		return observed == value
	case "!=":
		return observed != value
	}
	return false
}

// formatNumber renders v in plain decimal notation, which spreadsheets read
// as a number, with the fewest digits that parse back to v.
func formatNumber(v float64) string { return strconv.FormatFloat(v, 'f', -1, 64) }
