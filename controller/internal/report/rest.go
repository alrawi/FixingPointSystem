// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"regexp"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// Media types of the report formats.
const (
	JSONContentType = "application/json"
	CSVContentType  = "text/csv; charset=utf-8"
	// HTMLContentType equals html.ContentType, which this package cannot
	// import.
	HTMLContentType = "text/html; charset=utf-8"
)

// RunSource returns runs by id. *runs.Orchestrator satisfies it.
type RunSource interface {
	Get(id string) (runs.Run, error)
}

// TotalsSource returns the merged metrics of a run. *metrics.Aggregator
// satisfies it, and an error that matches metrics.ErrRunUnknown means that
// it holds no metrics of the run.
type TotalsSource interface {
	Totals(runID string) (map[string]float64, error)
}

var (
	_ RunSource    = (*runs.Orchestrator)(nil)
	_ TotalsSource = (*metrics.Aggregator)(nil)
)

// API serves GET /v1/runs/{id}/report and PUT /v1/runs/{id}/realism and
// answers every other operation with the 501 problem of api.Unimplemented.
// The binary embeds API next to the other operation sets.
type API struct {
	api.Unimplemented
	eval   *Evaluator
	runs   RunSource
	totals TotalsSource

	// html, samples, and objectives serve format=html. See serve.go.
	html         HTMLRenderer
	samples      SampleSource
	objectives   ObjectiveSource
	declarations DeclarationSource

	// realism keeps the realism reports that attachRealismReport receives,
	// and realismCheck validates them. See realism_api.go.
	realism      RealismStore
	realismCheck *realismValidator
}

var _ api.ServerInterface = (*API)(nil)

// NewAPI returns the report operation over the runs of rs and the metrics of
// ts. A nil ts builds reports from the run totals alone. When ts is also a
// SeriesSource, as *metrics.Aggregator is, reports select labeled series, and
// e reads the same source when a run stops, through UseSeries. When ts is a
// SampleSource, as *metrics.Aggregator is, the HTML report draws its samples.
func NewAPI(e *Evaluator, rs RunSource, ts TotalsSource) *API {
	if ss, ok := ts.(SeriesSource); ok {
		e.UseSeries(ss)
	}
	a := &API{eval: e, runs: rs, totals: ts, realism: NewMemoryRealism(), realismCheck: &realismValidator{}}
	if ss, ok := ts.(SampleSource); ok {
		a.samples = ss
	}
	return a
}

// safeName matches run ids that may appear in a file name unquoted.
var safeName = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,128}$`)

// GetRunReport answers GET /v1/runs/{id}/report in JSON, the default, in CSV,
// or in HTML. The HTML format answers 501 until UseHTML sets a renderer.
func (a *API) GetRunReport(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunReportParams) {
	format := api.GetRunReportParamsFormatJson
	if params.Format != nil {
		format = *params.Format
	}
	if !format.Valid() {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "unknown report format %q", format))
		return
	}
	if format == api.GetRunReportParamsFormatHtml && a.html == nil {
		api.WriteProblem(w, r, api.Problemf(http.StatusNotImplemented, "the html report format is not implemented"))
		return
	}
	run, err := a.runs.Get(id)
	if err != nil {
		status := http.StatusInternalServerError
		if errors.Is(err, runs.ErrRunNotFound) {
			status = http.StatusNotFound
		}
		api.WriteProblem(w, r, api.NewProblem(status, err.Error()))
		return
	}
	var merged map[string]float64
	if a.totals != nil {
		merged, err = a.totals.Totals(id)
		if err != nil && !errors.Is(err, metrics.ErrRunUnknown) {
			api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "report of run %s: %v", id, err))
			return
		}
	}
	var series []*metrics.Series
	labeled := len(run.Series) > 0
	if ss, ok := a.totals.(SeriesSource); ok {
		series, err = ss.Series(id)
		switch {
		case err == nil:
			labeled = true
		case !errors.Is(err, metrics.ErrRunUnknown):
			api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "report of run %s: %v", id, err))
			return
		}
	}
	var rep Report
	if labeled {
		rep = a.eval.BuildSeries(run, series)
	} else {
		rep = a.eval.Build(run, merged)
	}
	if a.declarations != nil {
		// A configuration that is gone or unreadable leaves the report
		// without declarations, as the HTML report does for the objective.
		if d, derr := a.declarations.Declarations(r.Context(), run.ConfigID); derr == nil {
			rep.Declared = d
		}
	}
	rep.Realism = a.realism.Results(id)
	var body []byte
	contentType := JSONContentType
	switch format {
	case api.GetRunReportParamsFormatHtml:
		contentType = HTMLContentType
		var in HTMLInput
		if in, err = a.htmlInput(r.Context(), run); err == nil {
			body, err = a.html(rep, in)
		}
	case api.GetRunReportParamsFormatCsv:
		contentType = CSVContentType
		body, err = rep.CSV()
	default:
		body, err = rep.JSON()
	}
	if err != nil {
		api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "%v", err))
		return
	}
	h := w.Header()
	h.Set("Content-Type", contentType)
	h.Set("Cache-Control", "no-store")
	h.Set("X-Content-Type-Options", "nosniff")
	if format == api.GetRunReportParamsFormatHtml {
		h.Set("Content-Security-Policy", HTMLContentSecurityPolicy)
	}
	if format == api.GetRunReportParamsFormatCsv && safeName.MatchString(run.ID) {
		h.Set("Content-Disposition", `attachment; filename="report-`+run.ID+`.csv"`)
	}
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(body)
}

// htmlInput gathers what the HTML report holds beyond the report: the
// per second samples of the run and the objective of its configuration. A run
// without samples gets a page without charts, and a run whose configuration
// is gone or unreadable gets a page without objective.
func (a *API) htmlInput(ctx context.Context, run runs.Run) (HTMLInput, error) {
	var in HTMLInput
	if a.samples != nil {
		s, err := a.samples.Samples(run.ID, api.MetricViewApplication, metrics.Window{})
		if err != nil && !errors.Is(err, metrics.ErrRunUnknown) {
			return in, fmt.Errorf("report of run %s: %w", run.ID, err)
		}
		in.Samples = s
	}
	if a.objectives != nil {
		if o, err := a.objectives.Objective(ctx, run.ConfigID); err == nil {
			in.Objective = o
		}
	}
	return in, nil
}
