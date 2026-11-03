// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"errors"
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

// API serves GET /v1/runs/{id}/report and answers every other operation
// with the 501 problem of api.Unimplemented. The binary embeds API next to
// the other operation sets.
type API struct {
	api.Unimplemented
	eval   *Evaluator
	runs   RunSource
	totals TotalsSource
}

var _ api.ServerInterface = (*API)(nil)

// NewAPI returns the report operation over the runs of rs and the metrics of
// ts. A nil ts builds reports from the run totals alone.
func NewAPI(e *Evaluator, rs RunSource, ts TotalsSource) *API {
	return &API{eval: e, runs: rs, totals: ts}
}

// safeName matches run ids that may appear in a file name unquoted.
var safeName = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,128}$`)

// GetRunReport answers GET /v1/runs/{id}/report in JSON, the default, or in
// CSV. The HTML format of the contract answers 501 until a package adds it.
func (a *API) GetRunReport(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunReportParams) {
	format := api.GetRunReportParamsFormatJson
	if params.Format != nil {
		format = *params.Format
	}
	if !format.Valid() {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "unknown report format %q", format))
		return
	}
	if format == api.GetRunReportParamsFormatHtml {
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
	rep := a.eval.Build(run, merged)
	var body []byte
	contentType := JSONContentType
	if format == api.GetRunReportParamsFormatCsv {
		contentType = CSVContentType
		body, err = rep.CSV()
	} else {
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
	if format == api.GetRunReportParamsFormatCsv && safeName.MatchString(run.ID) {
		h.Set("Content-Disposition", `attachment; filename="report-`+run.ID+`.csv"`)
	}
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(body)
}
