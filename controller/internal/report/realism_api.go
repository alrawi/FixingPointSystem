// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"net/http"
	"slices"
	"sync"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// RealismStore keeps the realism report that a run holds. A run holds at most
// one. Implementations are safe for concurrent use.
type RealismStore interface {
	// Replace makes doc, the report as the client sent it, and the results
	// that the mapping of CON-22 made from it the report of the run. It
	// reports whether the run held a report before.
	Replace(runID string, doc []byte, results []api.RealismResult) (replaced bool)
	// Results returns the results of the report of the run, or nil when the
	// run holds none. The caller may change the slice.
	Results(runID string) []api.RealismResult
}

// MemoryRealism is the RealismStore in memory. The orchestrator keeps its runs
// in memory too, so a report lives as long as its run.
type MemoryRealism struct {
	mu      sync.Mutex
	reports map[string]realismRecord
}

type realismRecord struct {
	doc     []byte
	results []api.RealismResult
}

var _ RealismStore = (*MemoryRealism)(nil)

// NewMemoryRealism returns an empty MemoryRealism.
func NewMemoryRealism() *MemoryRealism {
	return &MemoryRealism{reports: map[string]realismRecord{}}
}

// Replace implements RealismStore.
func (m *MemoryRealism) Replace(runID string, doc []byte, results []api.RealismResult) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	_, replaced := m.reports[runID]
	m.reports[runID] = realismRecord{doc: slices.Clone(doc), results: slices.Clone(results)}
	return replaced
}

// Results implements RealismStore.
func (m *MemoryRealism) Results(runID string) []api.RealismResult {
	m.mu.Lock()
	defer m.mu.Unlock()
	rec, ok := m.reports[runID]
	if !ok {
		return nil
	}
	return slices.Clone(rec.results)
}

// Document returns the report of the run as the client sent it, for a later
// operation that serves it, and whether the run holds one.
func (m *MemoryRealism) Document(runID string) ([]byte, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	rec, ok := m.reports[runID]
	return slices.Clone(rec.doc), ok
}

// UseRealism makes s the place where attachRealismReport keeps the reports and
// where the report of a run reads its realism results. NewAPI sets a
// MemoryRealism.
func (a *API) UseRealism(s RealismStore) {
	a.realism = s
}

// AttachRealismReport answers PUT /v1/runs/{id}/realism. It stores the realism
// report of a run that has ended, as a whole in place of an earlier one, and
// answers 200 with the results that the run now holds.
//
// The checks run in this order, and the first that fails answers: the run
// exists (404), the run has ended as completed, failed, or aborted (409), the
// content type is application/json (415), the body is at most MaxRealismBytes
// (413), the body is one JSON document (400), and the document conforms to
// the report schema and maps to results with distinct metrics and labels
// (422). A refused report leaves the one that the run held.
func (a *API) AttachRealismReport(w http.ResponseWriter, r *http.Request, id api.Id) {
	run, err := a.runs.Get(id)
	if err != nil {
		status := http.StatusInternalServerError
		if errors.Is(err, runs.ErrRunNotFound) {
			status = http.StatusNotFound
		}
		api.WriteProblem(w, r, api.NewProblem(status, err.Error()))
		return
	}
	if !runs.Final(run.State) {
		api.WriteProblem(w, r, api.Problemf(http.StatusConflict,
			"run %s is %s, and a realism report attaches only to a run that has completed, failed, or aborted", id, run.State))
		return
	}
	if mt, _, err := mime.ParseMediaType(r.Header.Get("Content-Type")); err != nil || mt != JSONContentType {
		api.WriteProblem(w, r, api.NewProblem(http.StatusUnsupportedMediaType, "send the realism report as application/json"))
		return
	}
	doc, err := readBody(w, r, MaxRealismBytes)
	switch {
	case errors.Is(err, errBodyTooLarge):
		api.WriteProblem(w, r, api.Problemf(http.StatusRequestEntityTooLarge, "the body exceeds %d bytes", MaxRealismBytes))
		return
	case err != nil:
		api.WriteProblem(w, r, api.NewProblem(http.StatusBadRequest, "the body could not be read"))
		return
	}
	if !json.Valid(doc) {
		api.WriteProblem(w, r, api.NewProblem(http.StatusBadRequest, "the body is not one JSON document"))
		return
	}
	findings, err := a.realismCheck.check(doc)
	if err != nil {
		api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "realism report of run %s: %v", id, err))
		return
	}
	var results []api.RealismResult
	if len(findings) == 0 {
		if results, findings, err = MapRealism(doc); err != nil {
			api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "realism report of run %s: %v", id, err))
			return
		}
	}
	if len(findings) > 0 {
		p := api.Problemf(http.StatusUnprocessableEntity, "the realism report is not valid at %s", pathOrRoot(findings[0].Path))
		p.Errors = &findings
		api.WriteProblem(w, r, p)
		return
	}
	replaced := a.realism.Replace(id, doc, results)
	body, err := json.Marshal(api.RealismAttachment{Realism: results, Replaced: replaced})
	if err != nil {
		api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "realism report of run %s: %v", id, err))
		return
	}
	h := w.Header()
	h.Set("Content-Type", JSONContentType)
	h.Set("Cache-Control", "no-store")
	h.Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(append(body, '\n'))
}

// errBodyTooLarge reports a body above the limit that readBody was given.
var errBodyTooLarge = errors.New("body too large")

// readBody reads the body of r up to limit bytes. It fails with
// errBodyTooLarge when the body is longer.
func readBody(w http.ResponseWriter, r *http.Request, limit int64) ([]byte, error) {
	data, err := io.ReadAll(http.MaxBytesReader(w, r.Body, limit))
	if err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			return nil, fmt.Errorf("read body: %w", errBodyTooLarge)
		}
		return nil, fmt.Errorf("read body: %w", err)
	}
	return data, nil
}

// pathOrRoot names the root of the document for the empty JSON Pointer.
func pathOrRoot(p string) string {
	if p == "" {
		return "the root"
	}
	return p
}
