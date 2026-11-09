// SPDX-License-Identifier: GPL-3.0-only

package api

import "net/http"

// Unimplemented answers every operation with a 501 problem. A package that
// implements operations embeds it and overrides those methods, so the
// controller serves the whole contract at every stage of the plan.
type Unimplemented struct{}

var _ ServerInterface = Unimplemented{}

// notImplemented writes the 501 problem for the operation with id operationID.
func notImplemented(w http.ResponseWriter, r *http.Request, operationID string) {
	WriteProblem(w, r, Problemf(http.StatusNotImplemented, "operation %s is not implemented yet", operationID))
}

// ListAgents answers listAgents with a 501 problem.
func (Unimplemented) ListAgents(w http.ResponseWriter, r *http.Request) {
	notImplemented(w, r, "listAgents")
}

// RegisterAgent answers registerAgent with a 501 problem.
func (Unimplemented) RegisterAgent(w http.ResponseWriter, r *http.Request) {
	notImplemented(w, r, "registerAgent")
}

// ReleaseAgent answers releaseAgent with a 501 problem.
func (Unimplemented) ReleaseAgent(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "releaseAgent")
}

// ReserveAgent answers reserveAgent with a 501 problem.
func (Unimplemented) ReserveAgent(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "reserveAgent")
}

// GetCapabilities answers getCapabilities with a 501 problem.
func (Unimplemented) GetCapabilities(w http.ResponseWriter, r *http.Request) {
	notImplemented(w, r, "getCapabilities")
}

// CreateConfig answers createConfig with a 501 problem.
func (Unimplemented) CreateConfig(w http.ResponseWriter, r *http.Request) {
	notImplemented(w, r, "createConfig")
}

// GetConfig answers getConfig with a 501 problem.
func (Unimplemented) GetConfig(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "getConfig")
}

// ValidateConfig answers validateConfig with a 501 problem.
func (Unimplemented) ValidateConfig(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "validateConfig")
}

// ListModules answers listModules with a 501 problem.
func (Unimplemented) ListModules(w http.ResponseWriter, r *http.Request, _ ListModulesParams) {
	notImplemented(w, r, "listModules")
}

// InstallModule answers installModule with a 501 problem.
func (Unimplemented) InstallModule(w http.ResponseWriter, r *http.Request, _ InstallModuleParams) {
	notImplemented(w, r, "installModule")
}

// DeleteModule answers deleteModule with a 501 problem.
func (Unimplemented) DeleteModule(w http.ResponseWriter, r *http.Request, _ string, _ string) {
	notImplemented(w, r, "deleteModule")
}

// GetModule answers getModule with a 501 problem.
func (Unimplemented) GetModule(w http.ResponseWriter, r *http.Request, _ string, _ string) {
	notImplemented(w, r, "getModule")
}

// ListRuns answers listRuns with a 501 problem.
func (Unimplemented) ListRuns(w http.ResponseWriter, r *http.Request, _ ListRunsParams) {
	notImplemented(w, r, "listRuns")
}

// StartRun answers startRun with a 501 problem.
func (Unimplemented) StartRun(w http.ResponseWriter, r *http.Request) {
	notImplemented(w, r, "startRun")
}

// GetRun answers getRun with a 501 problem.
func (Unimplemented) GetRun(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "getRun")
}

// ListCaptures answers listCaptures with a 501 problem.
func (Unimplemented) ListCaptures(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "listCaptures")
}

// StartCapture answers startCapture with a 501 problem.
func (Unimplemented) StartCapture(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "startCapture")
}

// DownloadCapture answers downloadCapture with a 501 problem.
func (Unimplemented) DownloadCapture(w http.ResponseWriter, r *http.Request, _ Id, _ string) {
	notImplemented(w, r, "downloadCapture")
}

// GetRunMetrics answers getRunMetrics with a 501 problem.
func (Unimplemented) GetRunMetrics(w http.ResponseWriter, r *http.Request, _ Id, _ GetRunMetricsParams) {
	notImplemented(w, r, "getRunMetrics")
}

// StreamRunMetrics answers streamRunMetrics with a 501 problem.
func (Unimplemented) StreamRunMetrics(w http.ResponseWriter, r *http.Request, _ Id, _ StreamRunMetricsParams) {
	notImplemented(w, r, "streamRunMetrics")
}

// GetRunReport answers getRunReport with a 501 problem.
func (Unimplemented) GetRunReport(w http.ResponseWriter, r *http.Request, _ Id, _ GetRunReportParams) {
	notImplemented(w, r, "getRunReport")
}

// StopRun answers stopRun with a 501 problem.
func (Unimplemented) StopRun(w http.ResponseWriter, r *http.Request, _ Id) {
	notImplemented(w, r, "stopRun")
}
