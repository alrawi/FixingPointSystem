// SPDX-License-Identifier: GPL-3.0-only

package html

import (
	"fmt"
	"maps"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/report"
)

// Options adds to a report what the report model does not hold.
type Options struct {
	// Title heads the page. Empty means "Run report <id>".
	Title string
	// Objective describes the load objective of the run, such as
	// "sessions per second 1000". The run record does not hold it, so the
	// caller reads it from the test configuration, for example with
	// FormatObjective.
	Objective string
	// Samples are the per second samples of a run, from
	// metrics.Aggregator.Samples. Only the samples of metrics.RunKey, which
	// stand for the whole run, are drawn.
	Samples []metrics.Sample
	// Realism holds the realism results. The report of CTL-09 has none yet.
	Realism []Realism
}

// FormatObjective formats the objective of a test configuration, whose type
// is one of concurrent_sessions, sessions_per_second, throughput_bps, and
// simulated_users.
func FormatObjective(typ string, value float64) string {
	return strings.ReplaceAll(typ, "_", " ") + " " + strconv.FormatFloat(value, 'f', -1, 64)
}

// chartSpec names a per second series that the page draws when the samples
// hold it.
type chartSpec struct{ title, metric, unit string }

var chartSpecs = []chartSpec{
	{"Concurrent sessions", "sessions.concurrent", "sessions"},
	{"Session rate", "sessions.rate", "sessions per second"},
	{"Throughput", "throughput.bps", "bits per second"},
	{"Failed session ratio", "sessions.failed_ratio", "ratio"},
	{"Response time p99", "sessions.response_time.p99", "microseconds"},
}

// summarySpecs names the totals of the summary, in order.
var summarySpecs = []struct{ label, metric, unit string }{
	{"Sessions attempted", "sessions.attempted", ""},
	{"Sessions succeeded", "sessions.succeeded", ""},
	{"Sessions failed", "sessions.failed", ""},
	{"Failed ratio", "sessions.failed_ratio", ""},
	{"Concurrent sessions", "sessions.concurrent", ""},
	{"Sessions per second", "sessions.rate", ""},
	{"Throughput", "throughput.bps", "bit/s"},
	{"Bytes sent", "bytes.tx", "bytes"},
	{"Bytes received", "bytes.rx", "bytes"},
	{"Response time p99", "sessions.response_time.p99", "us"},
}

// FromReport builds the page of r. A report without evaluation has the
// verdict "not evaluated" and no criteria.
func FromReport(r report.Report, o Options) Page {
	run := r.Run
	p := Page{
		Title:        o.Title,
		RunID:        run.ID,
		ConfigID:     run.ConfigID,
		State:        string(run.State),
		Phase:        run.Phase,
		Degraded:     run.Degraded,
		Reason:       run.Reason,
		Created:      formatTime(run.CreatedAt),
		Started:      formatTime(run.StartedAt),
		Ended:        formatTime(run.EndedAt),
		Duration:     duration(run.StartedAt, run.EndedAt),
		Seed:         run.Seed,
		Engine:       run.EngineVersion,
		Objective:    o.Objective,
		Agents:       slices.Clone(run.Agents),
		FailedAgents: slices.Clone(run.FailedAgents),
		Verdict:      VerdictNotEvaluated,
		Realism:      slices.Clone(o.Realism),
	}
	for _, name := range slices.Sorted(maps.Keys(run.ModuleVersions)) {
		p.Modules = append(p.Modules, KV{name, run.ModuleVersions[name]})
	}
	for _, s := range summarySpecs {
		v, ok := r.Totals[s.metric]
		if !ok {
			continue
		}
		val := plain(v)
		if s.unit != "" {
			val = Compact(v) + " " + s.unit
		}
		p.Summary = append(p.Summary, KV{s.label, val})
	}
	if ev := r.Evaluation; ev != nil {
		p.Verdict = VerdictFail
		if r.Passed {
			p.Verdict = VerdictPass
		}
		for _, res := range ev.Results {
			c := Criterion{
				Metric:    res.Metric,
				Where:     whereText(res.Where),
				Op:        res.Op,
				Threshold: plain(res.Value),
				Status:    string(res.Status),
				Reason:    res.Reason,
			}
			if res.Evaluated() {
				c.Observed = plain(res.Observed)
			}
			p.Criteria = append(p.Criteria, c)
		}
	}
	p.Charts = charts(o.Samples)
	return p
}

// charts builds one chart per spec from the samples of the whole run. A
// metric that no sample holds gets no chart.
func charts(samples []metrics.Sample) []Chart {
	var first time.Time
	for _, s := range samples {
		if s.Key == metrics.RunKey && (first.IsZero() || s.Ts.Before(first)) {
			first = s.Ts
		}
	}
	var out []Chart
	for _, spec := range chartSpecs {
		var pts []Point
		for _, s := range samples {
			if s.Key != metrics.RunKey {
				continue
			}
			if v, ok := s.Values[spec.metric]; ok {
				pts = append(pts, Point{T: s.Ts.Sub(first).Seconds(), V: v})
			}
		}
		if len(pts) == 0 {
			continue
		}
		slices.SortStableFunc(pts, func(a, b Point) int {
			switch {
			case a.T < b.T:
				return -1
			case a.T > b.T:
				return 1
			}
			return 0
		})
		out = append(out, Chart{Title: spec.title, Unit: spec.unit, Series: []Line{{Name: spec.metric, Points: pts}}})
	}
	return out
}

func formatTime(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return t.UTC().Format(time.RFC3339)
}

// duration returns the time from start to end, or "" when either is unknown
// or the run ended before it started.
func duration(start, end time.Time) string {
	if start.IsZero() || end.IsZero() || end.Before(start) {
		return ""
	}
	d := end.Sub(start)
	if d >= time.Second {
		d = d.Round(time.Second)
	}
	return d.String()
}

// whereText formats a label filter as name=value pairs sorted by name.
func whereText(where map[string]string) string {
	parts := make([]string, 0, len(where))
	for _, k := range slices.Sorted(maps.Keys(where)) {
		parts = append(parts, fmt.Sprintf("%s=%s", k, where[k]))
	}
	return strings.Join(parts, ",")
}
