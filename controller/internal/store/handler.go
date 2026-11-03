// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// MaxBodyBytes bounds the size of a configuration body.
const MaxBodyBytes = 1 << 20

// Handler serves createConfig, getConfig, and validateConfig, and answers
// every other operation through the embedded api.Unimplemented.
type Handler struct {
	api.Unimplemented

	repo      Repository
	schema    *Schema
	validator *Validator
	logger    *slog.Logger
	now       func() time.Time
	newID     func() (string, error)
}

var _ api.ServerInterface = (*Handler)(nil)

// Options holds the optional dependencies of a Handler.
type Options struct {
	// Modules lists installed scenarios. Nil skips the module reference check.
	Modules ModuleCatalog
	// Agents lists available agents. Nil skips the capacity check.
	Agents AgentInventory
	// Logger receives server errors. Nil discards them.
	Logger *slog.Logger
	// Now returns the current time. Nil means time.Now.
	Now func() time.Time
	// NewID returns a fresh configuration id that matches
	// ^[A-Za-z0-9_-]{1,64}$. Nil means RandomID.
	NewID func() (string, error)
}

// NewHandler returns a handler that stores configurations in repo.
func NewHandler(repo Repository, opts Options) (*Handler, error) {
	schema, err := NewSchema()
	if err != nil {
		return nil, err
	}
	h := &Handler{
		repo:      repo,
		schema:    schema,
		validator: NewValidator(schema, opts.Modules, opts.Agents),
		logger:    opts.Logger,
		now:       opts.Now,
		newID:     opts.NewID,
	}
	if h.logger == nil {
		h.logger = slog.New(slog.DiscardHandler)
	}
	if h.now == nil {
		h.now = time.Now
	}
	if h.newID == nil {
		h.newID = RandomID
	}
	return h, nil
}

// RandomID returns cfg_ and 24 random hex digits.
func RandomID() (string, error) {
	var b [12]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", fmt.Errorf("new config id: %w", err)
	}
	return "cfg_" + hex.EncodeToString(b[:]), nil
}

// CreateConfig stores a schema valid configuration and answers 201 with its
// ConfigRef. A body that breaks the schema gets a 422 problem whose errors
// name every failing field.
func (h *Handler) CreateConfig(w http.ResponseWriter, r *http.Request) {
	f, err := FormatOf(r.Header.Get("Content-Type"))
	if err != nil {
		api.WriteProblem(w, r, api.NewProblem(http.StatusUnsupportedMediaType,
			"send the configuration as application/json or application/yaml"))
		return
	}
	data, err := io.ReadAll(http.MaxBytesReader(w, r.Body, MaxBodyBytes))
	if err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			api.WriteProblem(w, r, api.Problemf(http.StatusRequestEntityTooLarge, "the body exceeds %d bytes", MaxBodyBytes))
			return
		}
		api.WriteProblem(w, r, api.NewProblem(http.StatusBadRequest, "the body could not be read"))
		return
	}
	doc, err := Canonical(data, f)
	if err != nil {
		api.WriteProblem(w, r, api.NewProblem(http.StatusBadRequest, err.Error()))
		return
	}
	findings, err := h.schema.Check(doc)
	if err != nil {
		h.serverError(w, r, err)
		return
	}
	if len(findings) > 0 {
		p := api.Problemf(http.StatusUnprocessableEntity, "the configuration breaks the schema at %s", findings[0].Path)
		p.Errors = &findings
		api.WriteProblem(w, r, p)
		return
	}
	var head struct {
		Metadata struct {
			Name string `json:"name"`
		} `json:"metadata"`
	}
	if err := json.Unmarshal(doc, &head); err != nil {
		h.serverError(w, r, fmt.Errorf("read metadata.name: %w", err))
		return
	}
	id, err := h.newID()
	if err != nil {
		h.serverError(w, r, err)
		return
	}
	rec := Record{ID: id, Name: head.Metadata.Name, Body: doc, CreatedAt: h.now().UTC()}
	if err := h.repo.Create(r.Context(), rec); err != nil {
		h.serverError(w, r, err)
		return
	}
	h.logger.Info("config stored", "config_id", rec.ID, "name", rec.Name)
	created := rec.CreatedAt
	w.Header().Set("Location", api.BasePath+"/configs/"+rec.ID)
	writeJSON(w, http.StatusCreated, api.ConfigRef{Id: rec.ID, Name: rec.Name, CreatedAt: &created})
}

// GetConfig answers 200 with the stored configuration as JSON.
func (h *Handler) GetConfig(w http.ResponseWriter, r *http.Request, id api.Id) {
	rec, ok := h.get(w, r, id)
	if !ok {
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(append(rec.Body, '\n'))
}

// ValidateConfig answers 200 with the validation report of the stored
// configuration.
func (h *Handler) ValidateConfig(w http.ResponseWriter, r *http.Request, id api.Id) {
	rec, ok := h.get(w, r, id)
	if !ok {
		return
	}
	rep, err := h.validator.Validate(r.Context(), rec.Body)
	if err != nil {
		h.serverError(w, r, fmt.Errorf("validate config %s: %w", id, err))
		return
	}
	writeJSON(w, http.StatusOK, rep)
}

// get loads the configuration with id or writes the problem and returns false.
func (h *Handler) get(w http.ResponseWriter, r *http.Request, id string) (Record, bool) {
	rec, err := h.repo.Get(r.Context(), id)
	if errors.Is(err, ErrConfigNotFound) {
		api.WriteProblem(w, r, api.Problemf(http.StatusNotFound, "no config has id %s", id))
		return Record{}, false
	}
	if err != nil {
		h.serverError(w, r, err)
		return Record{}, false
	}
	return rec, true
}

// serverError logs err and answers 500 without exposing its text.
func (h *Handler) serverError(w http.ResponseWriter, r *http.Request, err error) {
	h.logger.Error("config operation failed", "method", r.Method, "path", r.URL.Path, "error", err)
	api.WriteProblem(w, r, api.NewProblem(http.StatusInternalServerError, "the controller could not complete the request"))
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	body, err := json.Marshal(v)
	if err != nil {
		status = http.StatusInternalServerError
		body = []byte(`{"type":"about:blank","status":500}`)
		w.Header().Set("Content-Type", api.ProblemContentType)
	} else {
		w.Header().Set("Content-Type", "application/json")
	}
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(status)
	_, _ = w.Write(append(body, '\n'))
}
