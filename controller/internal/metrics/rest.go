// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// RunSource tells which runs exist and whether they ended. The orchestrator
// of controller/internal/runs satisfies it.
type RunSource interface {
	Get(id string) (runs.Run, error)
}

// API serves GET /v1/runs/{id}/metrics and GET /v1/runs/{id}/metrics:stream
// from an Aggregator and answers every other operation with the 501 problem
// of api.Unimplemented. The binary embeds API next to the other operation
// sets.
type API struct {
	api.Unimplemented
	agg  *Aggregator
	runs RunSource
}

var _ api.ServerInterface = (*API)(nil)

// NewAPI returns the metrics operations of agg. With a nil rs, a run exists
// once the aggregator holds a snapshot of it, and a stream ends only when
// the client leaves.
func NewAPI(agg *Aggregator, rs RunSource) *API { return &API{agg: agg, runs: rs} }

// lookup reports whether the run with id exists, and otherwise writes a
// 404 problem. A run exists when the RunSource knows it or the aggregator
// holds its metrics.
func (a *API) lookup(w http.ResponseWriter, r *http.Request, id string) bool {
	if a.runs != nil {
		_, err := a.runs.Get(id)
		switch {
		case err == nil:
			return true
		case !errors.Is(err, runs.ErrRunNotFound):
			api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "look up run %s: %v", id, err))
			return false
		}
	}
	if a.agg.Has(id) {
		return true
	}
	api.WriteProblem(w, r, api.Problemf(http.StatusNotFound, "run %s not found", id))
	return false
}

// GetRunMetrics answers GET /v1/runs/{id}/metrics with the samples of one
// view over a time window. Without from and to, the window covers the whole
// run, and step_s keeps the newest sample of every step. The side parameter
// selects the client side, which is the default, or the server side (CON-12).
func (a *API) GetRunMetrics(w http.ResponseWriter, r *http.Request, id api.Id, params api.GetRunMetricsParams) {
	if !ValidView(params.View) {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "view %q is not one of %v", params.View, Views))
		return
	}
	side := ""
	if params.Side != nil {
		side = string(*params.Side)
	}
	if !ValidSide(side) {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "side %q is not client or server", side))
		return
	}
	win := Window{Step: time.Second, Side: side}
	if params.StepS != nil {
		if *params.StepS < 1 {
			api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "step_s %d is below 1", *params.StepS))
			return
		}
		win.Step = time.Duration(*params.StepS) * time.Second
	}
	if params.From != nil {
		win.From = *params.From
	}
	if params.To != nil {
		win.To = *params.To
	}
	if !win.From.IsZero() && !win.To.IsZero() && win.To.Before(win.From) {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "to %s is before from %s", win.To.Format(time.RFC3339), win.From.Format(time.RFC3339)))
		return
	}
	if !a.lookup(w, r, id) {
		return
	}
	samples, err := a.agg.Samples(id, params.View, win)
	if err != nil && !errors.Is(err, ErrRunUnknown) {
		api.WriteProblem(w, r, api.Problemf(http.StatusInternalServerError, "%v", err))
		return
	}
	if samples == nil {
		samples = []Sample{}
	}
	writeJSON(w, http.StatusOK, SeriesSet{View: params.View, Samples: samples})
}

// StreamRunMetrics answers GET /v1/runs/{id}/metrics:stream with Server Sent
// Events. Once per stream interval it sends one event per sample of every
// requested view, all views when the request names none. Each event carries
// a MetricSample as JSON in its data field and the tick number as its id.
// The side parameter selects the client side, which is the default, or the
// server side for every view (CON-12). When the run ends, the stream sends the final samples and an event named
// end whose data holds the run state, and closes.
func (a *API) StreamRunMetrics(w http.ResponseWriter, r *http.Request, id api.Id, params api.StreamRunMetricsParams) {
	views := Views
	if params.View != nil && len(*params.View) > 0 {
		views = *params.View
		for _, v := range views {
			if !ValidView(v) {
				api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "view %q is not one of %v", v, Views))
				return
			}
		}
	}
	side := ""
	if params.Side != nil {
		side = string(*params.Side)
	}
	if !ValidSide(side) {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "side %q is not client or server", side))
		return
	}
	if !a.lookup(w, r, id) {
		return
	}
	rc := http.NewResponseController(w)
	_ = rc.SetWriteDeadline(time.Time{}) // a stream outlives the server write timeout
	h := w.Header()
	h.Set("Content-Type", "text/event-stream")
	h.Set("Cache-Control", "no-store")
	h.Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)
	if _, err := fmt.Fprintf(w, "retry: %d\n\n", a.agg.cfg.StreamInterval.Milliseconds()); err != nil {
		return
	}
	if rc.Flush() != nil {
		return
	}
	ticker := time.NewTicker(a.agg.cfg.StreamInterval)
	defer ticker.Stop()
	var tick uint64
	for {
		tick++
		final, state := a.runState(id)
		if err := a.emit(w, id, views, side, tick); err != nil {
			return
		}
		if final {
			_, _ = fmt.Fprintf(w, "event: end\ndata: {\"state\":%q}\n\n", state)
			_ = rc.Flush()
			return
		}
		if rc.Flush() != nil {
			return
		}
		select {
		case <-r.Context().Done():
			return
		case <-ticker.C:
		}
	}
}

// runState reports whether the run ended and its state.
func (a *API) runState(id string) (bool, string) {
	if a.runs == nil {
		return false, ""
	}
	run, err := a.runs.Get(id)
	if err != nil {
		return errors.Is(err, runs.ErrRunNotFound), "unknown"
	}
	return runs.Final(run.State), string(run.State)
}

// emit writes one event per current sample of every view.
func (a *API) emit(w http.ResponseWriter, id string, views []api.MetricView, side string, tick uint64) error {
	for _, v := range views {
		samples, err := a.agg.LatestSide(id, v, side)
		if err != nil && !errors.Is(err, ErrRunUnknown) {
			return err
		}
		for _, s := range samples {
			body, err := json.Marshal(s)
			if err != nil {
				return fmt.Errorf("encode sample of run %s: %w", id, err)
			}
			if _, err := fmt.Fprintf(w, "id: %s\ndata: %s\n\n", strconv.FormatUint(tick, 10), body); err != nil {
				return fmt.Errorf("write sample of run %s: %w", id, err)
			}
		}
	}
	return nil
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
