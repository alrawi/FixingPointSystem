// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry/signing"
)

// Request headers that carry the detached signature of an installModule body.
// The signature is not part of the module tree, so it cannot travel inside the
// archive. The value of SignatureHeader is the standard base64 encoding of the
// 64 byte Ed25519 signature over the module tree digest, and the value of
// SignatureKeyHeader is the key id. The contract does not name the headers yet.
const (
	SignatureHeader    = "X-Module-Signature"
	SignatureKeyHeader = "X-Module-Signature-Key-Id"
)

// defaultPageSize and maxPageSize follow the PageSize parameter of the contract.
const (
	defaultPageSize = 50
	maxPageSize     = 500
)

// API serves the /v1/modules operations from a Registry. It embeds
// api.Unimplemented, so it satisfies api.ServerInterface on its own, and the
// controller composes it with the types of other packages.
type API struct {
	api.Unimplemented

	reg    *Registry
	logger *slog.Logger
}

var _ api.ServerInterface = (*API)(nil)

// NewAPI returns the module operations over reg. A nil logger discards logs.
func NewAPI(reg *Registry, logger *slog.Logger) *API {
	if logger == nil {
		logger = slog.New(slog.DiscardHandler)
	}
	return &API{reg: reg, logger: logger}
}

// ListModules serves listModules. The page token is the opaque key of the last
// module of the previous page, so a page stays stable when modules are
// installed or deleted between requests.
func (a *API) ListModules(w http.ResponseWriter, r *http.Request, params api.ListModulesParams) {
	typ := ""
	if params.Type != nil {
		typ = string(*params.Type)
	}
	size := defaultPageSize
	if params.PageSize != nil {
		size = *params.PageSize
	}
	if size < 1 || size > maxPageSize {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "page_size must be between 1 and %d", maxPageSize))
		return
	}
	entries := a.reg.List(r.Context(), typ)
	start := 0
	if params.PageToken != nil && *params.PageToken != "" {
		name, version, ok := decodePageToken(*params.PageToken)
		if !ok {
			api.WriteProblem(w, r, api.NewProblem(http.StatusBadRequest, "page_token is not a token that listModules returned"))
			return
		}
		start = len(entries)
		for i, e := range entries {
			if e.Name > name || (e.Name == name && compareVersions(e.Version, version) > 0) {
				start = i
				break
			}
		}
	}
	end := min(start+size, len(entries))
	type page struct {
		Items         []api.ModuleSummary `json:"items"`
		NextPageToken *string             `json:"next_page_token,omitempty"`
	}
	out := page{Items: make([]api.ModuleSummary, 0, end-start)}
	for _, e := range entries[start:end] {
		out.Items = append(out.Items, summary(e))
	}
	if end < len(entries) {
		tok := encodePageToken(entries[end-1])
		out.NextPageToken = &tok
	}
	writeJSON(w, http.StatusOK, out)
}

// InstallModule serves installModule. The body is a module archive, a tar
// file that may be gzip compressed, with manifest.yaml at its root. The
// signature headers carry the detached signature, which the registry verifies
// when it has a trust store. A request with only one of the two headers, or a
// signature that is not base64, is a 400.
func (a *API) InstallModule(w http.ResponseWriter, r *http.Request) {
	var opts []InstallOption
	keyID, sigText := r.Header.Get(SignatureKeyHeader), r.Header.Get(SignatureHeader)
	if keyID != "" || sigText != "" {
		sig, err := base64.StdEncoding.DecodeString(sigText)
		if keyID == "" || sigText == "" || err != nil {
			api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "%s and %s must both be set, and %s must be standard base64", SignatureKeyHeader, SignatureHeader, SignatureHeader))
			return
		}
		opts = append(opts, WithSignature(signing.Signature{KeyID: keyID, Bytes: sig}))
	}
	body := http.MaxBytesReader(w, r.Body, a.reg.limits.MaxBytes)
	e, err := a.reg.Install(r.Context(), body, opts...)
	if err != nil {
		var mbe *http.MaxBytesError
		if errors.As(err, &mbe) {
			err = errors.Join(ErrArchiveTooLarge, err)
		}
		a.writeError(w, r, err)
		return
	}
	a.logger.Info("module installed", "module", e.Name, "version", e.Version, "digest", e.Digest)
	w.Header().Set("Location", api.BasePath+"/modules/"+e.Name+"/"+e.Version)
	writeJSON(w, http.StatusCreated, summary(e))
}

// GetModule serves getModule with the manifest as JSON.
func (a *API) GetModule(w http.ResponseWriter, r *http.Request, name, version string) {
	m, err := a.reg.Manifest(r.Context(), name, version)
	if err != nil {
		a.writeError(w, r, err)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(append(m.Raw, '\n'))
}

// DeleteModule serves deleteModule.
func (a *API) DeleteModule(w http.ResponseWriter, r *http.Request, name, version string) {
	if err := a.reg.Delete(r.Context(), name, version); err != nil {
		a.writeError(w, r, err)
		return
	}
	a.logger.Info("module deleted", "module", name, "version", version)
	w.WriteHeader(http.StatusNoContent)
}

// writeError translates registry errors into problems in one place.
func (a *API) writeError(w http.ResponseWriter, r *http.Request, err error) {
	var verr *ValidationError
	switch {
	case errors.As(err, &verr):
		p := api.NewProblem(http.StatusUnprocessableEntity, verr.Error())
		findings := make([]api.Finding, len(verr.Findings))
		for i, f := range verr.Findings {
			findings[i] = api.Finding{Severity: api.FindingSeverityError, Path: f.Path, Message: f.Message}
		}
		p.Errors = &findings
		api.WriteProblem(w, r, p)
	case errors.Is(err, ErrUntrusted):
		api.WriteProblem(w, r, api.NewProblem(http.StatusForbidden, err.Error()))
	case errors.Is(err, ErrModuleNotFound):
		api.WriteProblem(w, r, api.NewProblem(http.StatusNotFound, err.Error()))
	case errors.Is(err, ErrModuleExists):
		api.WriteProblem(w, r, api.NewProblem(http.StatusConflict, err.Error()))
	case errors.Is(err, ErrArchiveTooLarge):
		api.WriteProblem(w, r, api.NewProblem(http.StatusRequestEntityTooLarge, err.Error()))
	case errors.Is(err, ErrArchive), errors.Is(err, ErrSymlink):
		api.WriteProblem(w, r, api.NewProblem(http.StatusBadRequest, err.Error()))
	default:
		a.logger.Error("module operation failed", "path", r.URL.Path, "error", err.Error())
		api.WriteProblem(w, r, api.NewProblem(http.StatusInternalServerError, "the module registry failed"))
	}
}

func summary(e Entry) api.ModuleSummary {
	s := api.ModuleSummary{Name: e.Name, Version: e.Version, Type: api.ModuleType(e.Type), License: e.License}
	if e.Summary != "" {
		s.Summary = &e.Summary
	}
	if len(e.Scenarios) > 0 {
		sc := append([]string(nil), e.Scenarios...)
		s.Scenarios = &sc
	}
	return s
}

func encodePageToken(e Entry) string {
	return base64.RawURLEncoding.EncodeToString([]byte(e.Name + "/" + e.Version))
}

func decodePageToken(tok string) (name, version string, ok bool) {
	b, err := base64.RawURLEncoding.DecodeString(tok)
	if err != nil {
		return "", "", false
	}
	name, version, ok = strings.Cut(string(b), "/")
	return name, version, ok && name != "" && version != ""
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
	w.WriteHeader(status)
	_, _ = w.Write(append(body, '\n'))
}
