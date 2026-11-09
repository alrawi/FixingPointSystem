// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"net/http"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// Server is the one api.ServerInterface of the controller. It embeds the
// handler of every package, and each package embeds api.Unimplemented, so Go
// promotes the method that a package implements over the 501 methods that the
// other packages inherit, because the implemented method sits one level
// higher. The operations that no package implements yet sit in pending at the
// same level as the implemented ones, so no selector is ambiguous.
//
// A package that implements a pending operation removes it from pending and
// embeds its handler here. The compiler reports an ambiguous selector when
// both remain, so an operation never reaches two handlers.
type Server struct {
	*agentOps
	*capabilityOps
	*configOps
	*moduleOps
	*runOps
	pending
}

// Aliases give each embedded handler its own field name, since three of the
// handler types share the name API.
type (
	agentOps      = agents.API
	capabilityOps = api.CapabilitiesAPI
	configOps     = store.Handler
	moduleOps     = registry.API
	runOps        = runs.API
)

var _ api.ServerInterface = (*Server)(nil)

// Handlers holds the handler of every package for NewServer. Metrics and
// Report are optional.
type Handlers struct {
	// Agents serves /v1/agents.
	Agents *agents.API
	// Capabilities serves /v1/capabilities.
	Capabilities *api.CapabilitiesAPI
	// Configs serves /v1/configs.
	Configs *store.Handler
	// Modules serves /v1/modules.
	Modules *registry.API
	// Runs serves GET and POST /v1/runs, GET /v1/runs/{id}, and POST /v1/runs/{id}:stop.
	Runs *runs.API
	// Metrics serves the two metrics operations of a run. Nil answers them
	// with 501 until the metrics package of CTL-08 is wired.
	Metrics MetricsHandler
	// Report serves GET /v1/runs/{id}/report. Nil answers it with 501.
	Report ReportHandler
	// Captures serves the four capture operations of a run. Nil answers them
	// with 501.
	Captures CaptureHandler
}

// NewServer returns the combined server over h.
func NewServer(h Handlers) *Server {
	return &Server{
		agentOps:      h.Agents,
		capabilityOps: h.Capabilities,
		configOps:     h.Configs,
		moduleOps:     h.Modules,
		runOps:        h.Runs,
		pending:       pending{metrics: h.Metrics, report: h.Report, captures: h.Captures},
	}
}

// MetricsHandler serves the metrics operations of a run, which CTL-08 adds
// in internal/metrics. It is the hook for that package: the binary passes
// its handler as Handlers.Metrics, or Controller.SetMetrics before Mount.
type MetricsHandler interface {
	// GetRunMetrics serves GET /v1/runs/{id}/metrics.
	GetRunMetrics(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunMetricsParams)
	// StreamRunMetrics serves GET /v1/runs/{id}/metrics:stream.
	StreamRunMetrics(w http.ResponseWriter, r *http.Request, id api.Id, params api.StreamRunMetricsParams)
}

// ReportHandler serves the report operation of a run, which CTL-09 adds in
// internal/report. Controller.SetReport replaces the handler that New sets.
type ReportHandler interface {
	// GetRunReport serves GET /v1/runs/{id}/report.
	GetRunReport(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunReportParams)
}

// CaptureHandler serves the capture operations of a run, which CTL-14 adds in
// internal/captures. *captures.API satisfies it.
type CaptureHandler interface {
	// ListCaptures serves GET /v1/runs/{id}/captures.
	ListCaptures(w http.ResponseWriter, r *http.Request, id api.Id)
	// StartCapture serves POST /v1/runs/{id}/captures.
	StartCapture(w http.ResponseWriter, r *http.Request, id api.Id)
	// DownloadCapture serves GET /v1/runs/{id}/captures/{capture_id}.
	DownloadCapture(w http.ResponseWriter, r *http.Request, id api.Id, captureID string)
	// StopCapture serves POST /v1/runs/{id}/captures/{capture_id}:stop.
	StopCapture(w http.ResponseWriter, r *http.Request, id api.Id, captureID string)
}

// pending answers the operations that no package implements yet. It forwards
// the metrics operations to a MetricsHandler, the report operation to a
// ReportHandler, and the capture operations to a CaptureHandler when one is
// set.
type pending struct {
	metrics  MetricsHandler
	report   ReportHandler
	captures CaptureHandler
}

// ListCaptures forwards listCaptures to the capture handler, or answers 501.
func (p pending) ListCaptures(w http.ResponseWriter, r *http.Request, id api.Id) {
	if p.captures == nil {
		api.Unimplemented{}.ListCaptures(w, r, id)
		return
	}
	p.captures.ListCaptures(w, r, id)
}

// StartCapture forwards startCapture to the capture handler, or answers 501.
func (p pending) StartCapture(w http.ResponseWriter, r *http.Request, id api.Id) {
	if p.captures == nil {
		api.Unimplemented{}.StartCapture(w, r, id)
		return
	}
	p.captures.StartCapture(w, r, id)
}

// DownloadCapture forwards downloadCapture to the capture handler, or answers 501.
func (p pending) DownloadCapture(w http.ResponseWriter, r *http.Request, id api.Id, captureID string) {
	if p.captures == nil {
		api.Unimplemented{}.DownloadCapture(w, r, id, captureID)
		return
	}
	p.captures.DownloadCapture(w, r, id, captureID)
}

// StopCapture forwards stopCapture to the capture handler, or answers 501. It
// also resolves the method that every embedded handler inherits from
// api.Unimplemented.
func (p pending) StopCapture(w http.ResponseWriter, r *http.Request, id api.Id, captureID string) {
	if p.captures == nil {
		api.Unimplemented{}.StopCapture(w, r, id, captureID)
		return
	}
	p.captures.StopCapture(w, r, id, captureID)
}

// GetRunReport forwards getRunReport to the report handler, or answers 501.
func (p pending) GetRunReport(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunReportParams) {
	if p.report == nil {
		api.Unimplemented{}.GetRunReport(w, r, id, params)
		return
	}
	p.report.GetRunReport(w, r, id, params)
}

// GetRunMetrics forwards getRunMetrics to the metrics handler, or answers 501.
func (p pending) GetRunMetrics(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunMetricsParams) {
	if p.metrics == nil {
		api.Unimplemented{}.GetRunMetrics(w, r, id, params)
		return
	}
	p.metrics.GetRunMetrics(w, r, id, params)
}

// StreamRunMetrics forwards streamRunMetrics to the metrics handler, or
// answers 501.
func (p pending) StreamRunMetrics(w http.ResponseWriter, r *http.Request, id api.Id, params api.StreamRunMetricsParams) {
	if p.metrics == nil {
		api.Unimplemented{}.StreamRunMetrics(w, r, id, params)
		return
	}
	p.metrics.StreamRunMetrics(w, r, id, params)
}
