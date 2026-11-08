// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

const (
	runToken  = "run-token"
	readToken = "read-token"
)

var fixedNow = time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)

// testServer serves h through the REST layer with a run and a read token.
func testServer(t *testing.T, h *Handler) *httptest.Server {
	t.Helper()
	auth, err := api.NewStaticTokens(map[string]api.Scope{runToken: api.ScopeRun, readToken: api.ScopeRead})
	if err != nil {
		t.Fatal(err)
	}
	srv := httptest.NewServer(api.New(h, auth, nil).Handler())
	t.Cleanup(srv.Close)
	return srv
}

// sequentialIDs returns an id generator that yields cfg_1, cfg_2, and so on.
func sequentialIDs() func() (string, error) {
	n := 0
	return func() (string, error) {
		n++
		return fmt.Sprintf("cfg_%d", n), nil
	}
}

func newTestHandler(t *testing.T, repo Repository, opts Options) *Handler {
	t.Helper()
	if opts.Now == nil {
		opts.Now = func() time.Time { return fixedNow }
	}
	if opts.NewID == nil {
		opts.NewID = sequentialIDs()
	}
	h, err := NewHandler(repo, opts)
	if err != nil {
		t.Fatal(err)
	}
	return h
}

func do(t *testing.T, srv *httptest.Server, method, path, token, ct string, body []byte) (*http.Response, []byte) {
	t.Helper()
	req, err := http.NewRequestWithContext(context.Background(), method, srv.URL+path, bytes.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+token)
	if ct != "" {
		req.Header.Set("Content-Type", ct)
	}
	resp, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	return resp, data
}

func decodeProblem(t *testing.T, resp *http.Response, data []byte, status int) api.Problem {
	t.Helper()
	if resp.StatusCode != status {
		t.Fatalf("status %d, want %d, body %s", resp.StatusCode, status, data)
	}
	if ct := resp.Header.Get("Content-Type"); ct != api.ProblemContentType {
		t.Fatalf("Content-Type %q, want %q", ct, api.ProblemContentType)
	}
	var p api.Problem
	if err := json.Unmarshal(data, &p); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestEveryExampleStoresAndValidates(t *testing.T) {
	h := newTestHandler(t, openMemory(t), Options{Modules: exampleCatalog(t), Agents: exampleInventory()})
	srv := testServer(t, h)
	for _, ex := range examples(t) {
		canon := mustCanonical(t, ex.data)
		for _, body := range []struct {
			ct   string
			data []byte
		}{{"application/yaml", ex.data}, {"application/json", canon}} {
			resp, data := do(t, srv, http.MethodPost, "/v1/configs", runToken, body.ct, body.data)
			if resp.StatusCode != http.StatusCreated {
				t.Fatalf("%s as %s: status %d, body %s", ex.name, body.ct, resp.StatusCode, data)
			}
			var ref api.ConfigRef
			if err := json.Unmarshal(data, &ref); err != nil {
				t.Fatal(err)
			}
			if ref.Name != ex.name || ref.CreatedAt == nil || !ref.CreatedAt.Equal(fixedNow) {
				t.Errorf("%s: ref %+v", ex.name, ref)
			}
			if loc := resp.Header.Get("Location"); loc != "/v1/configs/"+ref.Id {
				t.Errorf("%s: Location %q", ex.name, loc)
			}

			resp, data = do(t, srv, http.MethodGet, "/v1/configs/"+ref.Id, readToken, "", nil)
			if resp.StatusCode != http.StatusOK || resp.Header.Get("Content-Type") != "application/json" {
				t.Fatalf("%s: GET status %d, type %q", ex.name, resp.StatusCode, resp.Header.Get("Content-Type"))
			}
			if !bytes.Equal(bytes.TrimSpace(data), canon) {
				t.Errorf("%s: GET body %s, want %s", ex.name, data, canon)
			}

			resp, data = do(t, srv, http.MethodPost, "/v1/configs/"+ref.Id+":validate", runToken, "", nil)
			if resp.StatusCode != http.StatusOK {
				t.Fatalf("%s: validate status %d, body %s", ex.name, resp.StatusCode, data)
			}
			var rep api.ValidationReport
			if err := json.Unmarshal(data, &rep); err != nil {
				t.Fatal(err)
			}
			if !rep.Valid || len(rep.Findings) != 0 {
				t.Errorf("%s: report %s, want valid", ex.name, data)
			}
		}
	}
}

func TestInvalidFixturesReturnAProblemNamingTheField(t *testing.T) {
	repo := openMemory(t)
	srv := testServer(t, newTestHandler(t, repo, Options{}))
	for _, fx := range readFixtures(t, filepath.Join("testdata", "invalid")) {
		resp, data := do(t, srv, http.MethodPost, "/v1/configs", runToken, "application/yaml", fx.data)
		p := decodeProblem(t, resp, data, http.StatusUnprocessableEntity)
		if p.Errors == nil || !hasFinding(*p.Errors, api.FindingSeverityError, fx.want) {
			t.Errorf("%s: problem %s names no error at %s", fx.name, data, fx.want)
		}
		if p.Detail == nil || !strings.Contains(*p.Detail, "/") {
			t.Errorf("%s: detail %v names no field", fx.name, p.Detail)
		}
	}
	if _, err := repo.Get(context.Background(), "cfg_1"); !errors.Is(err, ErrConfigNotFound) {
		t.Errorf("an invalid config was stored: %v", err)
	}
}

func TestUnresolvedFixturesStoreAndFailValidation(t *testing.T) {
	srv := testServer(t, newTestHandler(t, openMemory(t), Options{Modules: exampleCatalog(t), Agents: exampleInventory()}))
	for _, fx := range readFixtures(t, filepath.Join("testdata", "unresolved")) {
		resp, data := do(t, srv, http.MethodPost, "/v1/configs", runToken, "application/yaml", fx.data)
		if resp.StatusCode != http.StatusCreated {
			t.Fatalf("%s: status %d, body %s", fx.name, resp.StatusCode, data)
		}
		var ref api.ConfigRef
		if err := json.Unmarshal(data, &ref); err != nil {
			t.Fatal(err)
		}
		_, data = do(t, srv, http.MethodPost, "/v1/configs/"+ref.Id+":validate", runToken, "", nil)
		var rep api.ValidationReport
		if err := json.Unmarshal(data, &rep); err != nil {
			t.Fatal(err)
		}
		if rep.Valid || !hasFinding(rep.Findings, api.FindingSeverityError, fx.want) {
			t.Errorf("%s: report %s has no error at %s", fx.name, data, fx.want)
		}
	}
}

func TestRequestProblems(t *testing.T) {
	srv := testServer(t, newTestHandler(t, openMemory(t), Options{}))
	tests := []struct {
		name, method, path, token, ct string
		body                          []byte
		status                        int
	}{
		{"unknown config", http.MethodGet, "/v1/configs/nope", readToken, "", nil, http.StatusNotFound},
		{"validate unknown config", http.MethodPost, "/v1/configs/nope:validate", runToken, "", nil, http.StatusNotFound},
		{"text body", http.MethodPost, "/v1/configs", runToken, "text/plain", []byte("x"), http.StatusUnsupportedMediaType},
		{"no content type", http.MethodPost, "/v1/configs", runToken, "", []byte("{}"), http.StatusUnsupportedMediaType},
		{"malformed json", http.MethodPost, "/v1/configs", runToken, "application/json", []byte("{"), http.StatusBadRequest},
		{"malformed yaml", http.MethodPost, "/v1/configs", runToken, "application/yaml", []byte("a: ["), http.StatusBadRequest},
		{"too large", http.MethodPost, "/v1/configs", runToken, "application/json", bytes.Repeat([]byte(" "), MaxBodyBytes+1), http.StatusRequestEntityTooLarge},
		{"read scope cannot create", http.MethodPost, "/v1/configs", readToken, "application/json", []byte("{}"), http.StatusForbidden},
	}
	for _, tc := range tests {
		resp, data := do(t, srv, tc.method, tc.path, tc.token, tc.ct, tc.body)
		decodeProblem(t, resp, data, tc.status)
	}
}

// failingRepo fails every call with err.
type failingRepo struct{ err error }

func (f failingRepo) Create(context.Context, Record) error        { return f.err }
func (f failingRepo) Get(context.Context, string) (Record, error) { return Record{}, f.err }

// fixedRepo serves one record for every id.
type fixedRepo struct{ rec Record }

func (f fixedRepo) Create(context.Context, Record) error        { return nil }
func (f fixedRepo) Get(context.Context, string) (Record, error) { return f.rec, nil }

func TestServerErrorsHideTheirCause(t *testing.T) {
	smoke := exampleNamed(t, "enterprise_mix_10g")
	cause := errors.New("disk on fire")
	for _, tc := range []struct {
		name   string
		h      *Handler
		method string
		path   string
	}{
		{"create", newTestHandler(t, failingRepo{cause}, Options{}), http.MethodPost, "/v1/configs"},
		{"get", newTestHandler(t, failingRepo{cause}, Options{}), http.MethodGet, "/v1/configs/x"},
		{"id", newTestHandler(t, openMemory(t), Options{NewID: func() (string, error) { return "", cause }}), http.MethodPost, "/v1/configs"},
		{"catalog", newTestHandler(t, fixedRepo{Record{ID: "x", Body: mustCanonical(t, smoke.data)}}, Options{Modules: fakeCatalog{err: cause}}), http.MethodPost, "/v1/configs/x:validate"},
		{"stored body", newTestHandler(t, fixedRepo{Record{ID: "x", Body: []byte("{")}}, Options{}), http.MethodPost, "/v1/configs/x:validate"},
	} {
		srv := testServer(t, tc.h)
		resp, data := do(t, srv, tc.method, tc.path, runToken, "application/yaml", smoke.data)
		decodeProblem(t, resp, data, http.StatusInternalServerError)
		if bytes.Contains(data, []byte(cause.Error())) {
			t.Errorf("%s: problem %s exposes the cause", tc.name, data)
		}
	}
}

func TestDefaultsAndRandomID(t *testing.T) {
	h, err := NewHandler(openMemory(t), Options{})
	if err != nil {
		t.Fatal(err)
	}
	if h.now == nil || h.newID == nil || h.logger == nil {
		t.Fatal("NewHandler left a default unset")
	}
	a, err := RandomID()
	if err != nil {
		t.Fatal(err)
	}
	b, _ := RandomID()
	if a == b || len(a) != 28 || !strings.HasPrefix(a, "cfg_") {
		t.Errorf("RandomID = %q, %q", a, b)
	}
	srv := testServer(t, h)
	resp, data := do(t, srv, http.MethodPost, "/v1/configs", runToken, "application/yaml", exampleNamed(t, "enterprise_mix_10g").data)
	if resp.StatusCode != http.StatusCreated {
		t.Errorf("status %d, body %s", resp.StatusCode, data)
	}
}

func TestOtherOperationsStayUnimplemented(t *testing.T) {
	srv := testServer(t, newTestHandler(t, openMemory(t), Options{}))
	resp, data := do(t, srv, http.MethodGet, "/v1/agents", readToken, "", nil)
	decodeProblem(t, resp, data, http.StatusNotImplemented)
}
