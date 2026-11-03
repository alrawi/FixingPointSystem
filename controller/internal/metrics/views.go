// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"strings"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// RunKey is the sample key that every view uses for the whole run.
// Application, scenario, and port names never contain an asterisk.
const RunKey = "*"

// Views lists the metric views of the REST contract in its order.
var Views = []api.MetricView{"application", "scenario", "port", "tcp", "tls", "realism"}

// ValidView reports whether v is a view of the contract.
func ValidView(v api.MetricView) bool {
	for _, x := range Views {
		if x == v {
			return true
		}
	}
	return false
}

// member reports whether metric m belongs to view v.
//
//   - application holds the scenario and flow action metrics summed over every
//     label except application, and the mix gauges.
//   - scenario holds the same metrics keyed by application/scenario.
//   - port holds the port and host metrics keyed by port.
//   - tcp and tls hold the transport metrics of their protocol keyed by port.
//   - realism holds the realism gauges keyed by application.
func member(v api.MetricView, m *Metric) bool {
	switch v {
	case "application":
		return m.Level == "scenario" || m.Level == "flow_action" ||
			(m.Level == "application" && strings.HasPrefix(m.Name, "mix."))
	case "scenario":
		return m.Level == "scenario" || m.Level == "flow_action"
	case "port":
		return m.Level == "port_host"
	case "tcp":
		return m.Level == "transport" && strings.HasPrefix(m.Name, "tcp.")
	case "tls":
		return m.Level == "transport" && strings.HasPrefix(m.Name, "tls.")
	case "realism":
		return m.Level == "application" && strings.HasPrefix(m.Name, "realism.")
	}
	return false
}

// viewKey returns the key of series s in view v, and false when s lacks a
// key label and therefore counts only toward RunKey.
func viewKey(v api.MetricView, s *Series) (string, bool) {
	switch v {
	case "application", "realism":
		return s.Label("application")
	case "scenario":
		app, ok1 := s.Label("application")
		sc, ok2 := s.Label("scenario")
		return app + "/" + sc, ok1 && ok2
	default:
		return s.Label("port")
	}
}

// Sample is one MetricSample of the REST contract. The generated type holds
// float32 values, which represent integers exactly only up to 2^24, so a
// byte counter loses precision within seconds. Sample holds float64 values
// and encodes to the same JSON, which the contract test checks against the
// MetricSample schema.
type Sample struct {
	Ts     time.Time          `json:"ts"`
	View   api.MetricView     `json:"view"`
	Key    string             `json:"key"`
	Values map[string]float64 `json:"values"`
}

// SeriesSet is the MetricSeriesSet of the REST contract, with Sample values.
type SeriesSet struct {
	View    api.MetricView `json:"view"`
	Samples []Sample       `json:"samples"`
}

// computeView renders the merged series of a run as the samples of view v at
// time ts, with the RunKey sample first and the other keys in order of
// first appearance, which follows catalog order.
func computeView(c *Catalog, merged []*Series, v api.MetricView, ts time.Time) []Sample {
	g := newGrouping()
	g.order = append(g.order, RunKey)
	g.groups[RunKey] = &group{key: RunKey, series: map[string]*Series{}}
	for _, s := range merged {
		if !member(v, s.Metric) || s.Metric.formula != nil {
			continue
		}
		g.add(RunKey, s, nil)
		if k, ok := viewKey(v, s); ok {
			g.add(k, s, nil)
		}
	}
	isMember := func(m *Metric) bool { return member(v, m) }
	out := make([]Sample, 0, len(g.order))
	for _, k := range g.order {
		gr := g.groups[k]
		gr.derive(c, isMember, nil)
		if len(gr.series) == 0 {
			continue
		}
		out = append(out, Sample{Ts: ts, View: v, Key: k, Values: gr.values()})
	}
	return out
}

// totals aggregates the merged series of a run over every label.
func totals(c *Catalog, merged []*Series) map[string]float64 {
	gr := &group{key: RunKey, series: map[string]*Series{}}
	g := &grouping{order: []string{RunKey}, groups: map[string]*group{RunKey: gr}}
	for _, s := range merged {
		if s.Metric.formula == nil {
			g.add(RunKey, s, nil)
		}
	}
	gr.derive(c, func(*Metric) bool { return true }, nil)
	return gr.values()
}
