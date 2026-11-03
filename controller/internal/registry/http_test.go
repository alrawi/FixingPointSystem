// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/getkin/kin-openapi/openapi3"
	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"
	"github.com/santhosh-tekuri/jsonschema/v6"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
)

const contractPath = contractsDir + "/openapi/fps-api-v1.yaml"

const (
	readToken  = "token-read"
	adminToken = "token-admin"
)

// apiHarness serves API through the REST layer and checks every response
// against the OpenAPI contract.
type apiHarness struct {
	t   *testing.T
	doc *openapi3.T
	h   http.Handler
}

func newAPIHarness(t *testing.T, reg *Registry) *apiHarness {
	t.Helper()
	doc, err := contract.Load(contractPath)
	if err != nil {
		t.Fatal(err)
	}
	auth, err := api.NewStaticTokens(map[string]api.Scope{readToken: api.ScopeRead, adminToken: api.ScopeAdmin})
	if err != nil {
		t.Fatal(err)
	}
	logger := slog.New(slog.DiscardHandler)
	return &apiHarness{t: t, doc: doc, h: api.New(NewAPI(reg, logger), auth, logger).Handler()}
}

// do sends one request for the operation at the contract path template tmpl
// and validates the response.
func (a *apiHarness) do(method, tmpl, path, token string, body []byte) *httptest.ResponseRecorder {
	a.t.Helper()
	req := httptest.NewRequest(method, api.BasePath+path, bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer "+token)
	if body != nil {
		req.Header.Set("Content-Type", "application/octet-stream")
	}
	rec := httptest.NewRecorder()
	a.h.ServeHTTP(rec, req)

	item := a.doc.Paths.Find(tmpl)
	if item == nil {
		a.t.Fatalf("contract has no path %s", tmpl)
	}
	params := map[string]string{}
	ts, ps := strings.Split(tmpl, "/"), strings.Split(strings.SplitN(path, "?", 2)[0], "/")
	for i := range ts {
		if strings.HasPrefix(ts[i], "{") && i < len(ps) {
			params[strings.Trim(ts[i], "{}")] = ps[i]
		}
	}
	vreq := httptest.NewRequest(method, path, nil)
	in := &openapi3filter.RequestValidationInput{
		Request:    vreq,
		PathParams: params,
		Route:      &routers.Route{Spec: a.doc, Path: tmpl, PathItem: item, Method: method, Operation: item.GetOperation(method)},
	}
	out := &openapi3filter.ResponseValidationInput{
		RequestValidationInput: in,
		Status:                 rec.Code,
		Header:                 rec.Header(),
		Body:                   io.NopCloser(bytes.NewReader(rec.Body.Bytes())),
		Options:                &openapi3filter.Options{IncludeResponseStatus: true},
	}
	if err := openapi3filter.ValidateResponse(context.Background(), out); err != nil {
		a.t.Fatalf("%s %s: response breaks the contract: %v\nbody: %s", method, path, err, rec.Body)
	}
	return rec
}

func wantStatus(t *testing.T, rec *httptest.ResponseRecorder, status int) {
	t.Helper()
	if rec.Code != status {
		t.Fatalf("status %d, want %d, body %s", rec.Code, status, rec.Body)
	}
}

func TestAPIInstallExamplesAndManage(t *testing.T) {
	reg := openRegistry(t)
	a := newAPIHarness(t, reg)

	for i, name := range []string{"http1", "dns"} {
		rec := a.do(http.MethodPost, "/modules", "/modules", adminToken, tarFiles(t, exampleTree(t, name, nil), i == 0))
		wantStatus(t, rec, http.StatusCreated)
		var s api.ModuleSummary
		if err := json.Unmarshal(rec.Body.Bytes(), &s); err != nil {
			t.Fatal(err)
		}
		if s.Name != name || s.Version != "0.1.0" || s.Type != api.ModuleTypeApp || s.License != "GPL-3.0-only" {
			t.Errorf("summary %+v", s)
		}
		if loc := rec.Header().Get("Location"); loc != "/v1/modules/"+name+"/0.1.0" {
			t.Errorf("Location %q", loc)
		}
	}
	want := planDigests(t)
	for _, name := range []string{"http1", "dns"} {
		e, err := reg.Entry(context.Background(), name, "0.1.0")
		if err != nil || e.Digest != want[name] {
			t.Errorf("%s digest %s, want %s (%v)", name, e.Digest, want[name], err)
		}
	}

	// A read token cannot install.
	wantStatus(t, a.do(http.MethodPost, "/modules", "/modules", readToken, tarFiles(t, exampleTree(t, "dns", nil), false)), http.StatusForbidden)
	// The same version twice conflicts.
	wantStatus(t, a.do(http.MethodPost, "/modules", "/modules", adminToken, tarFiles(t, exampleTree(t, "dns", nil), false)), http.StatusConflict)

	rec := a.do(http.MethodGet, "/modules", "/modules", readToken, nil)
	wantStatus(t, rec, http.StatusOK)
	var page struct {
		Items []api.ModuleSummary `json:"items"`
		Next  *string             `json:"next_page_token"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 2 || page.Items[0].Name != "dns" || page.Next != nil {
		t.Fatalf("list %s", rec.Body)
	}
	if page.Items[0].Scenarios == nil || strings.Join(*page.Items[0].Scenarios, ",") != "lookup,resolve" {
		t.Errorf("dns scenarios %v", page.Items[0].Scenarios)
	}

	// Paging walks the list one module at a time.
	rec = a.do(http.MethodGet, "/modules", "/modules?page_size=1", readToken, nil)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].Name != "dns" || page.Next == nil {
		t.Fatalf("first page %s", rec.Body)
	}
	rec = a.do(http.MethodGet, "/modules", "/modules?page_size=1&page_token="+*page.Next, readToken, nil)
	page.Next = nil
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].Name != "http1" || page.Next != nil {
		t.Fatalf("second page %s", rec.Body)
	}
	wantStatus(t, a.do(http.MethodGet, "/modules", "/modules?page_token=%21%21", readToken, nil), http.StatusBadRequest)
	rec = a.do(http.MethodGet, "/modules", "/modules?type=transport", readToken, nil)
	if !strings.Contains(rec.Body.String(), `"items":[]`) {
		t.Errorf("type filter %s", rec.Body)
	}

	// getModule returns the manifest, which validates against its schema.
	rec = a.do(http.MethodGet, "/modules/{name}/{version}", "/modules/http1/0.1.0", readToken, nil)
	wantStatus(t, rec, http.StatusOK)
	inst, err := jsonschema.UnmarshalJSON(bytes.NewReader(rec.Body.Bytes()))
	if err != nil {
		t.Fatal(err)
	}
	if err := manifestSchema.Validate(inst); err != nil {
		t.Errorf("getModule body breaks the manifest schema: %v", err)
	}
	if !strings.Contains(rec.Body.String(), `"name":"request"`) {
		t.Errorf("manifest lacks the function catalog: %s", rec.Body)
	}
	wantStatus(t, a.do(http.MethodGet, "/modules/{name}/{version}", "/modules/http1/9.9.9", readToken, nil), http.StatusNotFound)

	wantStatus(t, a.do(http.MethodDelete, "/modules/{name}/{version}", "/modules/http1/0.1.0", readToken, nil), http.StatusForbidden)
	wantStatus(t, a.do(http.MethodDelete, "/modules/{name}/{version}", "/modules/http1/0.1.0", adminToken, nil), http.StatusNoContent)
	wantStatus(t, a.do(http.MethodDelete, "/modules/{name}/{version}", "/modules/http1/0.1.0", adminToken, nil), http.StatusNotFound)
	wantStatus(t, a.do(http.MethodGet, "/modules/{name}/{version}", "/modules/http1/0.1.0", readToken, nil), http.StatusNotFound)
}

func TestAPIInstallRejectsLicenses(t *testing.T) {
	for _, id := range []string{"LicenseRef-acme", "Proprietary", "GPL-2.0-only", "MIT AND SSPL-1.0"} {
		t.Run(id, func(t *testing.T) {
			reg := openRegistry(t)
			a := newAPIHarness(t, reg)
			files := exampleTree(t, "http1", func(s string) string {
				return strings.Replace(s, "license: GPL-3.0-only", "license: "+id, 1)
			})
			rec := a.do(http.MethodPost, "/modules", "/modules", adminToken, tarFiles(t, files, true))
			wantStatus(t, rec, http.StatusUnprocessableEntity)
			var p api.Problem
			if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil {
				t.Fatal(err)
			}
			if p.Errors == nil || len(*p.Errors) != 1 || (*p.Errors)[0].Path != "/license" {
				t.Fatalf("problem %s", rec.Body)
			}
			if got := reg.List(context.Background(), ""); len(got) != 0 {
				t.Errorf("registry holds %+v", got)
			}
		})
	}
}

func TestAPIInstallRejectsArchives(t *testing.T) {
	small, err := Open(filepath.Join(t.TempDir(), "small"), Limits{MaxBytes: 512, MaxFiles: 100})
	if err != nil {
		t.Fatal(err)
	}
	a := newAPIHarness(t, small)
	wantStatus(t, a.do(http.MethodPost, "/modules", "/modules", adminToken, []byte("not an archive")), http.StatusBadRequest)
	wantStatus(t, a.do(http.MethodPost, "/modules", "/modules", adminToken, tarFiles(t, exampleTree(t, "http1", nil), false)), http.StatusRequestEntityTooLarge)
}
