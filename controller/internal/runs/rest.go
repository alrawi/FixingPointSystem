// SPDX-License-Identifier: GPL-3.0-only

package runs

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// maxBody bounds the request bodies of the run operations.
const maxBody = 1 << 20

// API serves POST /v1/runs, GET /v1/runs/{id}, and POST /v1/runs/{id}:stop
// from an Orchestrator and answers every other operation with the 501
// problem of api.Unimplemented.
type API struct {
	api.Unimplemented
	o *Orchestrator
}

var _ api.ServerInterface = (*API)(nil)

// NewAPI returns the REST operations of o.
func NewAPI(o *Orchestrator) *API { return &API{o: o} }

// StartRun answers POST /v1/runs. It returns 202 with the run in pending and
// a Location header, and the run proceeds in the background.
func (a *API) StartRun(w http.ResponseWriter, r *http.Request) {
	var body api.StartRunJSONRequestBody
	if err := decodeBody(r, &body, true); err != nil {
		writeError(w, r, err)
		return
	}
	req := StartRequest{ConfigID: body.ConfigId}
	if body.Agents != nil {
		req.Agents = *body.Agents
	}
	if body.Labels != nil {
		req.Labels = *body.Labels
	}
	run, err := a.o.Create(r.Context(), req)
	if err != nil {
		writeError(w, r, err)
		return
	}
	w.Header().Set("Location", api.BasePath+"/runs/"+run.ID)
	writeJSON(w, http.StatusAccepted, ToAPI(run))
}

// GetRun answers GET /v1/runs/{id}.
func (a *API) GetRun(w http.ResponseWriter, r *http.Request, id api.Id) {
	run, err := a.o.Get(id)
	if err != nil {
		writeError(w, r, err)
		return
	}
	writeJSON(w, http.StatusOK, ToAPI(run))
}

// StopRun answers POST /v1/runs/{id}:stop. Without force the run ramps down,
// and with force it aborts. It returns 202 with the run as it stands.
func (a *API) StopRun(w http.ResponseWriter, r *http.Request, id api.Id) {
	var body api.StopRunJSONRequestBody
	if err := decodeBody(r, &body, false); err != nil {
		writeError(w, r, err)
		return
	}
	run, err := a.o.Stop(id, body.Force != nil && *body.Force)
	if err != nil {
		writeError(w, r, err)
		return
	}
	writeJSON(w, http.StatusAccepted, ToAPI(run))
}

// errBadBody reports a request body that is not the JSON of the operation.
var errBadBody = errors.New("bad request body")

// decodeBody decodes the JSON body of r into v and rejects unknown fields.
// An empty body is an error only when required holds.
func decodeBody(r *http.Request, v any, required bool) error {
	data, err := io.ReadAll(io.LimitReader(r.Body, maxBody+1))
	if err != nil {
		return fmt.Errorf("%w: %w", errBadBody, err)
	}
	if len(data) > maxBody {
		return fmt.Errorf("%w: larger than %d bytes", errBadBody, maxBody)
	}
	if len(bytes.TrimSpace(data)) == 0 {
		if required {
			return fmt.Errorf("%w: empty", errBadBody)
		}
		return nil
	}
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		return fmt.Errorf("%w: %w", errBadBody, err)
	}
	if dec.More() {
		return fmt.Errorf("%w: trailing data", errBadBody)
	}
	return nil
}

// writeError maps the errors of the orchestrator to problems.
func writeError(w http.ResponseWriter, r *http.Request, err error) {
	status := http.StatusInternalServerError
	switch {
	case errors.Is(err, errBadBody):
		status = http.StatusBadRequest
	case errors.Is(err, ErrRunNotFound):
		status = http.StatusNotFound
	case errors.Is(err, store.ErrConfigNotFound), errors.Is(err, ErrInvalidRequest):
		status = http.StatusUnprocessableEntity
	case errors.Is(err, ErrRunFinished), errors.Is(err, ErrNoAgents),
		errors.Is(err, agents.ErrAgentOffline), errors.Is(err, agents.ErrAgentReserved), errors.Is(err, agents.ErrAgentBusy):
		status = http.StatusConflict
	case errors.Is(err, ErrClosed):
		status = http.StatusServiceUnavailable
	}
	api.WriteProblem(w, r, api.NewProblem(status, err.Error()))
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	body, err := json.Marshal(v)
	if err != nil {
		api.WriteProblem(w, nil, api.Problemf(http.StatusInternalServerError, "encode response: %v", err))
		return
	}
	h := w.Header()
	h.Set("Content-Type", "application/json")
	h.Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_, _ = w.Write(append(body, '\n'))
}

// ToAPI returns the Run schema of the REST contract for run.
func ToAPI(run Run) api.Run {
	out := api.Run{Id: run.ID, ConfigId: run.ConfigID, State: run.State}
	if run.Degraded {
		out.Degraded = ptr(true)
	}
	if run.Phase != "" {
		out.Phase = ptr(run.Phase)
	}
	if !run.CreatedAt.IsZero() {
		out.CreatedAt = ptr(run.CreatedAt)
	}
	if !run.StartedAt.IsZero() {
		out.StartedAt = ptr(run.StartedAt)
	}
	if !run.EndedAt.IsZero() {
		out.EndedAt = ptr(run.EndedAt)
	}
	if run.Seed <= math.MaxInt64 {
		out.Seed = ptr(int(run.Seed)) //nolint:gosec // G115: bounded by the check above
	}
	if run.EngineVersion != "" {
		out.EngineVersion = ptr(run.EngineVersion)
	}
	if len(run.ModuleVersions) > 0 {
		out.ModuleVersions = ptr(run.ModuleVersions)
	}
	if run.Reason != "" && (run.State == Failed || run.State == Aborted) {
		out.Reason = ptr(run.Reason)
	}
	if run.Totals != nil {
		out.Summary = summary(run.Totals)
	}
	if v := run.Verdict; v != nil {
		out.Verdict = &api.RunVerdict{Passed: ptr(v.Passed)}
		cs := make([]api.CriterionResult, len(v.Criteria))
		for i, c := range v.Criteria {
			cs[i].Metric, cs[i].Op, cs[i].Value = ptr(c.Metric), ptr(c.Op), ptr(float32(c.Value))
			switch {
			case c.Evaluated:
				cs[i].Observed, cs[i].Passed = ptr(float32(c.Observed)), ptr(c.Passed)
			case c.Invalid:
				cs[i].Passed = ptr(false)
			}
			if c.Reason != "" {
				cs[i].Reason = ptr(c.Reason)
			}
		}
		out.Verdict.Criteria = &cs
	}
	return out
}

// summary fills the RunSummary schema from the run totals.
func summary(t map[string]float64) *api.RunSummary {
	s := &api.RunSummary{}
	count := func(metric string) *int {
		if v, ok := t[metric]; ok && v >= 0 && v <= math.MaxInt64 {
			return ptr(int(v))
		}
		return nil
	}
	s.SessionsAttempted = count(MetricSessionsAttempted)
	s.SessionsSucceeded = count(MetricSessionsSucceeded)
	s.SessionsFailed = count(MetricSessionsFailed)
	s.BytesTx = count(MetricBytesTx)
	s.BytesRx = count(MetricBytesRx)
	return s
}

func ptr[T any](v T) *T { return &v }
