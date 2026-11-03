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

// Handlers holds the handler of every package for NewServer. Metrics is
// optional.
type Handlers struct {
	// Agents serves /v1/agents.
	Agents *agents.API
	// Capabilities serves /v1/capabilities.
	Capabilities *api.CapabilitiesAPI
	// Configs serves /v1/configs.
	Configs *store.Handler
	// Modules serves /v1/modules.
	Modules *registry.API
	// Runs serves POST /v1/runs, GET /v1/runs/{id}, and POST /v1/runs/{id}:stop.
	Runs *runs.API
	// Metrics serves the two metrics operations of a run. Nil answers them
	// with 501 until the metrics package of CTL-08 is wired.
	Metrics MetricsHandler
}

// NewServer returns the combined server over h.
func NewServer(h Handlers) *Server {
	return &Server{
		agentOps:      h.Agents,
		capabilityOps: h.Capabilities,
		configOps:     h.Configs,
		moduleOps:     h.Modules,
		runOps:        h.Runs,
		pending:       pending{metrics: h.Metrics},
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

// pending answers the operations that no package implements yet. It forwards
// the metrics operations to a MetricsHandler when one is set.
type pending struct {
	metrics MetricsHandler
}

// ListRuns answers listRuns with a 501 problem.
func (pending) ListRuns(w http.ResponseWriter, r *http.Request, params api.ListRunsParams) {
	api.Unimplemented{}.ListRuns(w, r, params)
}

// ListCaptures answers listCaptures with a 501 problem.
func (pending) ListCaptures(w http.ResponseWriter, r *http.Request, id api.Id) {
	api.Unimplemented{}.ListCaptures(w, r, id)
}

// StartCapture answers startCapture with a 501 problem.
func (pending) StartCapture(w http.ResponseWriter, r *http.Request, id api.Id) {
	api.Unimplemented{}.StartCapture(w, r, id)
}

// DownloadCapture answers downloadCapture with a 501 problem.
func (pending) DownloadCapture(w http.ResponseWriter, r *http.Request, id api.Id, captureID string) {
	api.Unimplemented{}.DownloadCapture(w, r, id, captureID)
}

// GetRunReport answers getRunReport with a 501 problem.
func (pending) GetRunReport(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunReportParams) {
	api.Unimplemented{}.GetRunReport(w, r, id, params)
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
