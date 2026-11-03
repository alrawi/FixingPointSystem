// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	_ "embed" // catalog.yaml is a byte copy of contracts/metrics/catalog.yaml
	"fmt"
	"slices"
	"strings"

	"go.yaml.in/yaml/v3"
)

// catalogYAML is a copy of contracts/metrics/catalog.yaml. Go embeds only
// files inside the module, and TestCatalogMatchesContract keeps the copy equal
// to the contract.
//
//go:embed catalog.yaml
var catalogYAML []byte

// Kind is the type of a catalog metric.
type Kind string

// The three metric kinds of the catalog.
const (
	Counter   Kind = "counter"
	Gauge     Kind = "gauge"
	Histogram Kind = "histogram"
)

// Metric is one entry of the metric catalog.
type Metric struct {
	Name        string   `yaml:"name"`
	Kind        Kind     `yaml:"type"`
	Unit        string   `yaml:"unit"`
	Level       string   `yaml:"level"`
	Labels      []string `yaml:"labels"`
	Description string   `yaml:"description"`
	Formula     string   `yaml:"formula"`

	// formula is the parsed Formula of a derived gauge.
	formula *formula
	// index is the position in the catalog, which orders every output.
	index int
}

// HasLabel reports whether the catalog declares label for m.
func (m *Metric) HasLabel(label string) bool { return slices.Contains(m.Labels, label) }

// additive reports whether values of the gauge from several agents, workers,
// or label values add up to the value of their union. Counts and rates add,
// while ratios and percentages do not and combine by their mean.
func (m *Metric) additive() bool {
	switch m.Unit {
	case "ratio", "percent", "percentage_points":
		return false
	}
	return true
}

// formula is a binary expression over two catalog metrics, the only form
// that the catalog uses.
type formula struct {
	left, right string
	op          byte
}

// eval applies the formula. It reports false when the value is undefined,
// such as a ratio over zero attempts.
func (f *formula) eval(l, r float64) (float64, bool) {
	switch f.op {
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

// Catalog is the parsed metric catalog.
type Catalog struct {
	// Version is the catalog version.
	Version string
	metrics []*Metric
	byName  map[string]*Metric
}

// DefaultCatalog parses the embedded copy of contracts/metrics/catalog.yaml.
func DefaultCatalog() (*Catalog, error) { return ParseCatalog(catalogYAML) }

// ParseCatalog parses a metric catalog in the format of
// contracts/metrics/catalog.schema.json.
func ParseCatalog(data []byte) (*Catalog, error) {
	var doc struct {
		Version string    `yaml:"version"`
		Metrics []*Metric `yaml:"metrics"`
	}
	if err := yaml.Unmarshal(data, &doc); err != nil {
		return nil, fmt.Errorf("parse metric catalog: %w", err)
	}
	c := &Catalog{Version: doc.Version, metrics: doc.Metrics, byName: make(map[string]*Metric, len(doc.Metrics))}
	for i, m := range doc.Metrics {
		if m == nil || m.Name == "" {
			return nil, fmt.Errorf("parse metric catalog: entry %d has no name", i)
		}
		switch m.Kind {
		case Counter, Gauge, Histogram:
		default:
			return nil, fmt.Errorf("parse metric catalog: metric %s has unknown type %q", m.Name, m.Kind)
		}
		if _, dup := c.byName[m.Name]; dup {
			return nil, fmt.Errorf("parse metric catalog: metric %s appears twice", m.Name)
		}
		m.index = i
		c.byName[m.Name] = m
	}
	for _, m := range c.metrics {
		if m.Formula == "" {
			continue
		}
		f, err := parseFormula(m.Formula)
		if err != nil {
			return nil, fmt.Errorf("parse metric catalog: metric %s: %w", m.Name, err)
		}
		for _, ref := range []string{f.left, f.right} {
			if r := c.byName[ref]; r == nil || r.Kind == Histogram {
				return nil, fmt.Errorf("parse metric catalog: metric %s: formula refers to %q, which is not a catalog counter or gauge", m.Name, ref)
			}
		}
		m.formula = f
	}
	return c, nil
}

func parseFormula(s string) (*formula, error) {
	fields := strings.Fields(s)
	if len(fields) != 3 || len(fields[1]) != 1 || !strings.Contains("+-*/", fields[1]) {
		return nil, fmt.Errorf("formula %q is not of the form <metric> <op> <metric>", s)
	}
	return &formula{left: fields[0], op: fields[1][0], right: fields[2]}, nil
}

// Lookup returns the metric with name, or nil.
func (c *Catalog) Lookup(name string) *Metric { return c.byName[name] }

// Metrics returns every metric in catalog order.
func (c *Catalog) Metrics() []*Metric { return slices.Clone(c.metrics) }

// PrometheusName maps a catalog name to its Prometheus name as
// contracts/README.md describes. The name gets the prefix fps_, dots become
// underscores, and counters get the suffix _total.
func PrometheusName(m *Metric) string {
	name := "fps_" + strings.ReplaceAll(m.Name, ".", "_")
	if m.Kind == Counter {
		name += "_total"
	}
	return name
}
