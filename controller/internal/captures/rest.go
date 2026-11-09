// SPDX-License-Identifier: GPL-3.0-only

package captures

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// maxBody bounds the request body of the start operation.
const maxBody = 1 << 20

// API serves the three capture operations of the REST contract from a
// Service and answers every other operation with the 501 problem of
// api.Unimplemented.
type API struct {
	api.Unimplemented
	svc *Service
}

var _ api.ServerInterface = (*API)(nil)

// NewAPI returns the REST operations of s.
func NewAPI(s *Service) *API { return &API{svc: s} }

// ListCaptures answers GET /v1/runs/{id}/captures.
func (a *API) ListCaptures(w http.ResponseWriter, r *http.Request, id api.Id) {
	cs, err := a.svc.List(r.Context(), id)
	if err != nil {
		writeError(w, r, err)
		return
	}
	out := make([]api.Capture, len(cs))
	for i, c := range cs {
		out[i] = ToAPI(c)
	}
	writeJSON(w, http.StatusOK, out)
}

// StartCapture answers POST /v1/runs/{id}/captures with 202 and the capture.
func (a *API) StartCapture(w http.ResponseWriter, r *http.Request, id api.Id) {
	var body api.StartCaptureJSONRequestBody
	if err := decodeBody(r, &body); err != nil {
		writeError(w, r, err)
		return
	}
	req := StartRequest{Port: body.Port, BPF: deref(body.Bpf)}
	if body.MaxBytes != nil {
		if *body.MaxBytes < 1 {
			writeError(w, r, fmt.Errorf("%w: max_bytes must be at least 1", ErrInvalidRequest))
			return
		}
		req.MaxBytes = uint64(*body.MaxBytes)
	}
	req.ExportTLSKeys = body.ExportTlsKeys != nil && *body.ExportTlsKeys
	c, err := a.svc.Start(r.Context(), id, req)
	if err != nil {
		writeError(w, r, err)
		return
	}
	w.Header().Set("Location", api.BasePath+"/runs/"+id+"/captures/"+c.ID)
	writeJSON(w, http.StatusAccepted, ToAPI(c))
}

// DownloadCapture answers GET /v1/runs/{id}/captures/{capture_id} with the
// pcapng file of the capture. A capture that still runs ends first. The
// headers X-FPS-Capture-Agents and X-FPS-Capture-Missing name the agents whose
// files the body holds and the agents whose files are missing.
func (a *API) DownloadCapture(w http.ResponseWriter, r *http.Request, id api.Id, captureID string) {
	d, err := a.svc.Prepare(r.Context(), id, captureID)
	if err != nil {
		writeError(w, r, err)
		return
	}
	defer d.Close()
	h := w.Header()
	h.Set("Content-Type", ContentType)
	h.Set("Content-Length", strconv.FormatInt(d.Size, 10))
	h.Set("Content-Disposition", `attachment; filename="capture-`+id+`-`+captureID+`.pcapng"`)
	h.Set("Cache-Control", "no-store")
	h.Set("X-Content-Type-Options", "nosniff")
	h.Set("X-FPS-Capture-Agents", strings.Join(d.Agents, ","))
	if len(d.Missing) > 0 {
		h.Set("X-FPS-Capture-Missing", strings.Join(d.Missing, ","))
	}
	w.WriteHeader(http.StatusOK)
	// The status is sent, so a failure now can only cut the body short, and
	// the Content-Length tells the client.
	_, _ = d.WriteTo(w)
}

// ToAPI returns the Capture schema of the REST contract for c. The bytes
// appear once the capture completed.
func ToAPI(c Capture) api.Capture {
	out := api.Capture{Id: c.ID, Port: c.Port, State: c.State}
	if c.State == api.CaptureStateCompleted {
		n := int(c.Bytes) //nolint:gosec // G115: bounded by MaxFileBytes per agent
		out.Bytes = &n
	}
	return out
}

func deref(s *string) string {
	if s == nil {
		return ""
	}
	return *s
}

// errBadBody reports a request body that is not the JSON of the operation.
var errBadBody = errors.New("bad request body")

func decodeBody(r *http.Request, v any) error {
	data, err := io.ReadAll(io.LimitReader(r.Body, maxBody+1))
	if err != nil {
		return fmt.Errorf("%w: %w", errBadBody, err)
	}
	if len(data) > maxBody {
		return fmt.Errorf("%w: larger than %d bytes", errBadBody, maxBody)
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

// writeError maps the errors of the service to problems.
func writeError(w http.ResponseWriter, r *http.Request, err error) {
	status := http.StatusInternalServerError
	switch {
	case errors.Is(err, errBadBody):
		status = http.StatusBadRequest
	case errors.Is(err, runs.ErrRunNotFound), errors.Is(err, ErrCaptureNotFound):
		status = http.StatusNotFound
	case errors.Is(err, ErrInvalidRequest):
		status = http.StatusUnprocessableEntity
	case errors.Is(err, ErrRunNotRunning), errors.Is(err, ErrStartFailed),
		errors.Is(err, agents.ErrAgentOffline), errors.Is(err, agents.ErrSessionLost):
		status = http.StatusConflict
	case errors.Is(err, agents.ErrCaptureUnsupported):
		status = http.StatusNotImplemented
	case errors.Is(err, ErrNoFile), errors.Is(err, ErrDigestMismatch):
		status = http.StatusBadGateway
	}
	api.WriteProblem(w, r, api.NewProblem(status, problemDetail(err)))
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
