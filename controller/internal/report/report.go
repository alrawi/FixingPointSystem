// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	"encoding/csv"
	"encoding/json"
	"fmt"
	"maps"
	"math"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// Report is the report of one run.
type Report struct {
	// Run is the run as the orchestrator holds it. Its Totals and Verdict
	// are replaced by the values of the report.
	Run runs.Run
	// Totals holds the run metrics that the report shows, keyed by catalog
	// name with histogram statistics under <metric>.<stat>, after the
	// catalog formulas are derived.
	Totals map[string]float64
	// Evaluation is the result of the criteria, or nil when the run never
	// reached the criteria, as for a run that is still under way or aborted.
	Evaluation *Evaluation
	// Passed is the verdict of the report. It holds when the evaluation
	// passed and at least one agent of the run survived.
	Passed bool

	cat *metrics.Catalog
}

// Build returns the report of run over the merged metrics of the
// aggregator, which may be nil.
//
// The counters of run.Totals take precedence over the aggregator, because
// the orchestrator counts the open sessions of a failed agent as failed and
// sees the final snapshot of every agent. Gauges and histogram statistics
// come from the aggregator, which averages ratio gauges and merges
// histograms, and every derived gauge is computed again from the result.
func (e *Evaluator) Build(run runs.Run, merged map[string]float64) Report {
	totals := maps.Clone(merged)
	if totals == nil {
		totals = map[string]float64{}
	}
	for name, v := range run.Totals {
		_, inAgg := totals[name]
		m := e.cat.Lookup(name)
		if !inAgg || (m != nil && m.Kind == metrics.Counter) {
			totals[name] = v
		}
	}
	totals = e.Derive(totals)
	rep := Report{Run: run, Totals: totals, cat: e.cat}
	rep.Run.Totals = totals
	rep.Run.Verdict = nil
	if run.Verdict != nil {
		criteria := make([]compiler.CriterionConfig, len(run.Verdict.Criteria))
		for i, c := range run.Verdict.Criteria {
			criteria[i] = c.CriterionConfig
		}
		ev := e.Evaluate(criteria, totals)
		alive := len(run.Agents) == 0 || len(run.FailedAgents) < len(run.Agents)
		v := ToVerdict(ev, alive)
		rep.Evaluation, rep.Passed, rep.Run.Verdict = &ev, v.Passed, &v
	}
	return rep
}

// API returns the RunReport schema of the REST contract. The criteria of the
// verdict carry observed and passed only when they compared a value, so a
// missing metric shows neither and an invalid criterion shows passed false.
func (r Report) API() api.RunReport {
	out := api.RunReport{Run: runs.ToAPI(r.Run)}
	out.Run.Summary = summary(r.Totals)
	if r.Evaluation != nil && out.Run.Verdict != nil && out.Run.Verdict.Criteria != nil {
		cs := *out.Run.Verdict.Criteria
		for i, res := range r.Evaluation.Results {
			if i >= len(cs) {
				break
			}
			switch res.Status {
			case Pass, Fail:
			case Invalid:
				cs[i].Observed, cs[i].Passed = nil, ptr(false)
			case Missing:
				cs[i].Observed, cs[i].Passed = nil, nil
			}
		}
	}
	out.Realism = make([]struct {
		Observed float32 `json:"observed"`
		Passed   bool    `json:"passed"`
		Property string  `json:"property"`
		Target   string  `json:"target"`
	}, 0)
	return out
}

// JSON renders the report as indented JSON of the RunReport schema with a
// final newline. Operators such as < stay unescaped, because the report is
// served as application/json and never embedded in HTML.
func (r Report) JSON() ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(r.API()); err != nil {
		return nil, fmt.Errorf("encode report of run %s: %w", r.Run.ID, err)
	}
	return buf.Bytes(), nil
}

// CSVHeader is the header row of the CSV report.
var CSVHeader = []string{"kind", "name", "op", "threshold", "value", "result", "detail"}

// CSV renders the report as one table with the columns of CSVHeader. Rows
// of kind run describe the run in name and value, rows of kind criterion hold
// one criterion with its observed value, result, and reason, and rows of
// kind metric hold one run total in catalog order.
func (r Report) CSV() ([]byte, error) {
	var buf bytes.Buffer
	w := csv.NewWriter(&buf)
	rows := [][]string{CSVHeader}
	field := func(name, value string) { rows = append(rows, []string{"run", name, "", "", value, "", ""}) }
	text := func(name, value string) { field(name, cellText(value)) }
	run := r.Run
	text("id", run.ID)
	text("config_id", run.ConfigID)
	field("state", string(run.State))
	field("degraded", strconv.FormatBool(run.Degraded))
	text("reason", run.Reason)
	field("created_at", formatTime(run.CreatedAt))
	field("started_at", formatTime(run.StartedAt))
	field("ended_at", formatTime(run.EndedAt))
	field("seed", strconv.FormatUint(run.Seed, 10))
	text("engine_version", run.EngineVersion)
	for _, name := range slices.Sorted(maps.Keys(run.ModuleVersions)) {
		text("module."+name, run.ModuleVersions[name])
	}
	text("agents", strings.Join(run.Agents, " "))
	text("failed_agents", strings.Join(run.FailedAgents, " "))
	verdict := "not evaluated"
	if r.Evaluation != nil {
		verdict = "fail"
		if r.Passed {
			verdict = "pass"
		}
	}
	field("verdict", verdict)
	if r.Evaluation != nil {
		for _, res := range r.Evaluation.Results {
			value := ""
			if res.Evaluated() {
				value = formatNumber(res.Observed)
			}
			rows = append(rows, []string{"criterion", res.Metric, res.Op, formatNumber(res.Value), value, string(res.Status), cellText(res.Reason)})
		}
	}
	for _, name := range r.metricOrder() {
		rows = append(rows, []string{"metric", name, "", "", formatNumber(r.Totals[name]), "", ""})
	}
	if err := w.WriteAll(rows); err != nil {
		return nil, fmt.Errorf("encode csv report of run %s: %w", run.ID, err)
	}
	return buf.Bytes(), nil
}

// metricOrder returns the keys of the totals in catalog order, histogram
// statistics in the order of metrics.Stats, and other names sorted last.
func (r Report) metricOrder() []string {
	seen := map[string]bool{}
	var out []string
	add := func(name string) {
		if _, ok := r.Totals[name]; ok && !seen[name] {
			seen[name] = true
			out = append(out, name)
		}
	}
	if r.cat != nil {
		for _, m := range r.cat.Metrics() {
			if m.Kind != metrics.Histogram {
				add(m.Name)
				continue
			}
			for _, s := range metrics.Stats {
				add(m.Name + "." + s.Name)
			}
		}
	}
	for _, name := range slices.Sorted(maps.Keys(r.Totals)) {
		add(name)
	}
	return out
}

// summary fills the RunSummary schema from the totals, or returns nil when
// the totals hold none of its metrics.
func summary(t map[string]float64) *api.RunSummary {
	s := &api.RunSummary{}
	found := false
	count := func(metric string) *int {
		if v, ok := t[metric]; ok && v >= 0 && v <= math.MaxInt64 {
			found = true
			return ptr(int(v))
		}
		return nil
	}
	gauge := func(metric string) *float32 {
		if v, ok := t[metric]; ok && !math.IsNaN(v) && !math.IsInf(v, 0) {
			found = true
			return ptr(float32(v))
		}
		return nil
	}
	s.SessionsAttempted = count(runs.MetricSessionsAttempted)
	s.SessionsSucceeded = count(runs.MetricSessionsSucceeded)
	s.SessionsFailed = count(runs.MetricSessionsFailed)
	s.BytesTx = count(runs.MetricBytesTx)
	s.BytesRx = count(runs.MetricBytesRx)
	s.ConcurrentSessions = count("sessions.concurrent")
	s.ThroughputBps = gauge("throughput.bps")
	s.SessionsPerSecond = gauge("sessions.rate")
	if !found {
		return nil
	}
	return s
}

// cellText guards a text cell against formula injection in spreadsheets. A
// value that starts with =, +, -, @, a tab, or a carriage return gets a
// leading apostrophe, which spreadsheets show as text. Numbers stay as they
// are, because they come from the report and not from a client.
func cellText(s string) string {
	if s != "" && strings.ContainsRune("=+-@\t\r", rune(s[0])) {
		return "'" + s
	}
	return s
}

func formatTime(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return t.UTC().Format(time.RFC3339Nano)
}

func ptr[T any](v T) *T { return &v }
