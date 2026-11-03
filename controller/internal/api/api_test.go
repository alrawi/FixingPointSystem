// SPDX-License-Identifier: GPL-3.0-only

package api

import (
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/app"
)

func TestScopeAllows(t *testing.T) {
	tests := []struct {
		have, need Scope
		want       bool
	}{
		{ScopeRead, ScopeRead, true},
		{ScopeRead, ScopeRun, false},
		{ScopeRead, ScopeAdmin, false},
		{ScopeRun, ScopeRead, true},
		{ScopeRun, ScopeRun, true},
		{ScopeRun, ScopeAdmin, false},
		{ScopeAdmin, ScopeRead, true},
		{ScopeAdmin, ScopeRun, true},
		{ScopeAdmin, ScopeAdmin, true},
		{0, ScopeRead, false},
		{ScopeAdmin, 0, false},
		{ScopeAdmin + 1, ScopeRead, false},
	}
	for _, tc := range tests {
		if got := tc.have.Allows(tc.need); got != tc.want {
			t.Errorf("%v.Allows(%v) = %v, want %v", tc.have, tc.need, got, tc.want)
		}
	}
}

func TestParseScope(t *testing.T) {
	for _, s := range []Scope{ScopeRead, ScopeRun, ScopeAdmin} {
		got, err := ParseScope(s.String())
		if err != nil || got != s {
			t.Errorf("ParseScope(%q) = %v, %v", s.String(), got, err)
		}
	}
	for _, name := range []string{"", "Admin", "write"} {
		if _, err := ParseScope(name); err == nil {
			t.Errorf("ParseScope(%q) succeeded", name)
		}
	}
	if got := Scope(9).String(); got != "Scope(9)" {
		t.Errorf("Scope(9).String() = %q", got)
	}
}

func TestStaticTokens(t *testing.T) {
	st, err := NewStaticTokens(map[string]Scope{"r": ScopeRead, "a": ScopeAdmin})
	if err != nil {
		t.Fatal(err)
	}
	if s, ok := st.Authenticate("a"); !ok || s != ScopeAdmin {
		t.Errorf("Authenticate(a) = %v, %v", s, ok)
	}
	for _, tok := range []string{"", "x", "R", "a "} {
		if _, ok := st.Authenticate(tok); ok {
			t.Errorf("Authenticate(%q) succeeded", tok)
		}
	}
	var none *StaticTokens
	if _, ok := none.Authenticate("a"); ok {
		t.Error("nil StaticTokens authenticated a token")
	}
	if _, err := NewStaticTokens(map[string]Scope{"": ScopeRead}); err == nil {
		t.Error("NewStaticTokens accepted an empty token")
	}
	if _, err := NewStaticTokens(map[string]Scope{"t": 0}); err == nil {
		t.Error("NewStaticTokens accepted scope 0")
	}
}

func TestBearerToken(t *testing.T) {
	tests := []struct {
		header, token string
		ok            bool
	}{
		{"Bearer abc", "abc", true},
		{"bearer abc", "abc", true},
		{"BEARER  abc ", "abc", true},
		{"Basic abc", "", false},
		{"Bearer", "", false},
		{"Bearer   ", "", false},
		{"", "", false},
	}
	for _, tc := range tests {
		r := httptest.NewRequest(http.MethodGet, "/", nil)
		r.Header.Set("Authorization", tc.header)
		tok, ok := bearerToken(r)
		if tok != tc.token || ok != tc.ok {
			t.Errorf("bearerToken(%q) = %q, %v, want %q, %v", tc.header, tok, ok, tc.token, tc.ok)
		}
	}
}

func TestWriteProblemDefaults(t *testing.T) {
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/v1/runs/a%2Fb", nil)
	WriteProblem(rec, req, Problem{Detail: ptr("boom")})
	if rec.Code != http.StatusInternalServerError {
		t.Fatalf("status %d, want 500", rec.Code)
	}
	var p Problem
	if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil {
		t.Fatal(err)
	}
	if p.Status == nil || *p.Status != 500 || p.Instance == nil || *p.Instance != "/v1/runs/a%2Fb" {
		t.Errorf("problem = %+v", p)
	}
	for k, want := range map[string]string{"Content-Type": ProblemContentType, "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff"} {
		if got := rec.Header().Get(k); got != want {
			t.Errorf("%s = %q, want %q", k, got, want)
		}
	}
}

func TestMatchTemplate(t *testing.T) {
	tests := []struct {
		tmpl, path string
		want       bool
	}{
		{"/runs", "/runs", true},
		{"/runs/{id}", "/runs/a1", true},
		{"/runs/{id}", "/runs/", false},
		{"/runs/{id}", "/runs/a:stop", false},
		{"/runs/{id}:stop", "/runs/a:stop", true},
		{"/runs/{id}:stop", "/runs/:stop", false},
		{"/runs/{id}:stop", "/runs/a:b:stop", false},
		{"/runs/{id}:stop", "/runs/a:start", false},
		{"/runs/{id}/metrics:stream", "/runs/a/metrics:stream", true},
		{"/runs/{id}/metrics:stream", "/runs/a/metrics", false},
		{"/runs/{id}/report", "/runs/a/report/x", false},
	}
	for _, tc := range tests {
		if got := matchTemplate(tc.tmpl, tc.path); got != tc.want {
			t.Errorf("matchTemplate(%q, %q) = %v, want %v", tc.tmpl, tc.path, got, tc.want)
		}
	}
}

// capabilities overrides one operation of Unimplemented and records the scope
// that reached it.
type capabilities struct {
	Unimplemented
	scope Scope
}

func (c *capabilities) GetCapabilities(w http.ResponseWriter, r *http.Request) {
	c.scope, _ = ScopeFromContext(r.Context())
	w.Header().Set("Content-Type", "application/json")
	_, _ = io.WriteString(w, `{"engine_api":"0.1.0","backends":[],"agents":[],"modules":[]}`)
}

// TestMountOnApp mounts the API on the controller app, the way the binary will,
// and checks an implemented operation, a stub, and /healthz side by side.
func TestMountOnApp(t *testing.T) {
	logger := slog.New(slog.DiscardHandler)
	a := app.New(app.DefaultConfig(), logger)
	auth, err := NewStaticTokens(map[string]Scope{"t": ScopeRun})
	if err != nil {
		t.Fatal(err)
	}
	impl := &capabilities{}
	New(impl, auth, logger).Mount(a)

	do := func(method, path string) *httptest.ResponseRecorder {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(method, path, nil)
		req.Header.Set("Authorization", "Bearer t")
		a.Handler().ServeHTTP(rec, req)
		return rec
	}
	if rec := do(http.MethodGet, "/v1/capabilities"); rec.Code != http.StatusOK {
		t.Errorf("GET /v1/capabilities: status %d", rec.Code)
	}
	if impl.scope != ScopeRun {
		t.Errorf("handler saw scope %v, want run", impl.scope)
	}
	if rec := do(http.MethodPost, "/v1/runs/a1:stop"); rec.Code != http.StatusNotImplemented {
		t.Errorf("POST /v1/runs/a1:stop: status %d", rec.Code)
	}
	if rec := do(http.MethodGet, "/healthz"); rec.Code != http.StatusOK {
		t.Errorf("GET /healthz: status %d", rec.Code)
	}
}
