// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"bytes"
	"math"
	"net/http"
	"strconv"
	"strings"
)

// PrometheusContentType is the media type of the text exposition format
// 0.0.4.
const PrometheusContentType = "text/plain; version=0.0.4; charset=utf-8"

// RunLabel is the Prometheus label that names the run of a series.
const RunLabel = "run_id"

// Prometheus serves the merged metrics of every run that the aggregator
// holds in the Prometheus text exposition format, for GET /metrics.
//
// A catalog name maps to fps_ followed by the name with dots replaced by
// underscores, and counters add _total, as contracts/README.md describes.
// Each series carries the label run_id and its catalog labels. A histogram
// becomes a summary with the quantiles 0.5, 0.9, 0.99, and 0.999, a _count,
// and a _sum that equals the HDR mean times the count. We write the format
// with the standard library, because the exposition of precomputed values
// needs none of the registry of client_golang and the controller module
// stays free of its dependencies.
type Prometheus struct {
	agg *Aggregator
}

// NewPrometheus returns the /metrics handler of agg.
func NewPrometheus(agg *Aggregator) *Prometheus { return &Prometheus{agg: agg} }

// promSeries is one run's series in the exposition.
type promSeries struct {
	run string
	s   *Series
}

// ServeHTTP writes the exposition.
func (p *Prometheus) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet && r.Method != http.MethodHead {
		w.Header().Set("Allow", "GET, HEAD")
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	w.Header().Set("Content-Type", PrometheusContentType)
	w.Header().Set("Cache-Control", "no-store")
	if r.Method == http.MethodHead {
		return
	}
	var buf bytes.Buffer
	p.write(&buf)
	_, _ = w.Write(buf.Bytes())
}

// write renders every metric family in catalog order, each with the series
// of every run ordered by run id and labels.
func (p *Prometheus) write(w *bytes.Buffer) {
	byMetric := map[string][]promSeries{}
	for _, run := range p.agg.Runs() {
		ss, err := p.agg.Series(run)
		if err != nil {
			continue // evicted since Runs
		}
		for _, s := range ss {
			byMetric[s.Metric.Name] = append(byMetric[s.Metric.Name], promSeries{run: run, s: s})
		}
	}
	for _, m := range p.agg.cfg.Catalog.metrics {
		list := byMetric[m.Name]
		if len(list) == 0 {
			continue
		}
		name := PrometheusName(m)
		typ := string(m.Kind)
		help := m.Description + " Unit " + m.Unit + "."
		if m.Kind == Histogram {
			typ = "summary"
			help += " Quantiles of the HDR histogram."
		}
		w.WriteString("# HELP " + name + " " + escapeHelp(help) + "\n")
		w.WriteString("# TYPE " + name + " " + typ + "\n")
		for _, ps := range list {
			labels := labelText(ps.run, ps.s.Labels, "")
			if m.Kind != Histogram {
				writeSample(w, name, labels, ps.s.Value())
				continue
			}
			h := ps.s.Hist
			for _, st := range Stats[:4] {
				writeSample(w, name, labelText(ps.run, ps.s.Labels, strconv.FormatFloat(st.Quantile, 'g', -1, 64)), float64(h.Quantile(st.Quantile)))
			}
			writeSample(w, name+"_sum", labels, h.Mean()*float64(h.Count()))
			writeSample(w, name+"_count", labels, float64(h.Count()))
		}
	}
}

func writeSample(w *bytes.Buffer, name, labels string, v float64) {
	w.WriteString(name)
	w.WriteString(labels)
	w.WriteByte(' ')
	w.WriteString(formatFloat(v))
	w.WriteByte('\n')
}

// labelText renders {run_id="...",<labels>,quantile="..."}.
func labelText(run string, labels []Label, quantile string) string {
	var b strings.Builder
	b.WriteString("{" + RunLabel + "=\"" + escapeLabel(run) + "\"")
	for _, l := range labels {
		b.WriteString("," + l.Name + "=\"" + escapeLabel(l.Value) + "\"")
	}
	if quantile != "" {
		b.WriteString(",quantile=\"" + quantile + "\"")
	}
	b.WriteByte('}')
	return b.String()
}

func formatFloat(v float64) string {
	switch {
	case math.IsNaN(v):
		return "NaN"
	case math.IsInf(v, 1):
		return "+Inf"
	case math.IsInf(v, -1):
		return "-Inf"
	}
	if v == math.Trunc(v) && math.Abs(v) < 1e15 {
		return strconv.FormatFloat(v, 'f', -1, 64) // integers keep every digit
	}
	return strconv.FormatFloat(v, 'g', -1, 64)
}

var (
	labelEscaper = strings.NewReplacer(`\`, `\\`, `"`, `\"`, "\n", `\n`)
	helpEscaper  = strings.NewReplacer(`\`, `\\`, "\n", `\n`)
)

func escapeLabel(s string) string { return labelEscaper.Replace(s) }
func escapeHelp(s string) string  { return helpEscaper.Replace(s) }
