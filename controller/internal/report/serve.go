// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"context"
	"fmt"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// HTMLContentSecurityPolicy is the Content-Security-Policy of the HTML
// report. The page is self contained and has an inline style sheet, so it
// needs nothing else.
const HTMLContentSecurityPolicy = "default-src 'none'; style-src 'unsafe-inline'"

// SampleSource returns the per second samples of a run. *metrics.Aggregator
// satisfies it, and an error that matches metrics.ErrRunUnknown means that it
// holds no samples of the run.
type SampleSource interface {
	Samples(runID string, v api.MetricView, w metrics.Window) ([]metrics.Sample, error)
}

var _ SampleSource = (*metrics.Aggregator)(nil)

// Objective is the load objective of a test configuration, with the type
// names of the configuration, for example sessions_per_second.
type Objective struct {
	Type  string
	Value float64
}

// ObjectiveSource returns the load objective of the test configuration with
// the given id.
type ObjectiveSource interface {
	Objective(ctx context.Context, configID string) (Objective, error)
}

// ConfigStore returns stored test configurations. *store.SQLite satisfies it.
type ConfigStore interface {
	Get(ctx context.Context, id string) (store.Record, error)
}

var _ ConfigStore = (*store.SQLite)(nil)

// ConfigObjectives is the ObjectiveSource that reads the objective from the
// stored test configuration.
type ConfigObjectives struct{ Store ConfigStore }

// Objective parses the configuration configID and returns its load
// objective.
func (c ConfigObjectives) Objective(ctx context.Context, configID string) (Objective, error) {
	rec, err := c.Store.Get(ctx, configID)
	if err != nil {
		return Objective{}, fmt.Errorf("objective of config %s: %w", configID, err)
	}
	cfg, err := compiler.ParseConfig(rec.Body)
	if err != nil {
		return Objective{}, fmt.Errorf("objective of config %s: %w", configID, err)
	}
	return Objective{Type: cfg.Load.Objective.Type, Value: cfg.Load.Objective.Value}, nil
}

// HTMLInput is what the HTML report holds beyond the Report.
type HTMLInput struct {
	// Objective is the load objective of the run, empty when unknown.
	Objective Objective
	// Samples are the per second samples of the run.
	Samples []metrics.Sample
}

// HTMLRenderer renders the report as an HTML page. The package report/html
// provides it as html.Renderer. It is a function type because report/html
// imports this package.
type HTMLRenderer func(rep Report, in HTMLInput) ([]byte, error)

// UseHTML sets the renderer of format=html. Without one the format answers
// 501. It returns a for chaining.
func (a *API) UseHTML(r HTMLRenderer) *API {
	a.html = r
	return a
}

// UseSamples sets the source of the per second samples of the HTML report.
// NewAPI sets it already when its totals source is a SampleSource.
func (a *API) UseSamples(s SampleSource) *API {
	a.samples = s
	return a
}

// UseObjectives sets the source of the objective of the HTML report. A run
// whose objective the source cannot return gets a page without objective.
// When o is also a DeclarationSource, as ConfigObjectives is, it becomes the
// source of the declarations too, so the binary needs no further call.
func (a *API) UseObjectives(o ObjectiveSource) *API {
	a.objectives = o
	if d, ok := o.(DeclarationSource); ok {
		a.declarations = d
	}
	return a
}
