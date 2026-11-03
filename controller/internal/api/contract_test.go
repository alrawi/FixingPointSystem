// SPDX-License-Identifier: GPL-3.0-only

package api_test

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"maps"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"

	"github.com/getkin/kin-openapi/openapi3"
	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
)

// contractPath is the contract, relative to this package directory.
const contractPath = "../../../contracts/openapi/fps-api-v1.yaml"

// Tokens of the three scopes that the tests use.
var testTokens = map[api.Scope]string{
	api.ScopeRead:  "token-read",
	api.ScopeRun:   "token-run",
	api.ScopeAdmin: "token-admin",
}

func loadContract(t *testing.T) *openapi3.T {
	t.Helper()
	doc, err := contract.Load(contractPath)
	if err != nil {
		t.Fatal(err)
	}
	if err := contract.Validate(context.Background(), doc); err != nil {
		t.Fatal(err)
	}
	return doc
}

func newHandler(t *testing.T) http.Handler {
	t.Helper()
	tokens := make(map[string]api.Scope, len(testTokens))
	for s, tok := range testTokens {
		tokens[tok] = s
	}
	auth, err := api.NewStaticTokens(tokens)
	if err != nil {
		t.Fatal(err)
	}
	return api.New(api.Unimplemented{}, auth, slog.New(slog.DiscardHandler)).Handler()
}

// opCall is one request to one operation of the contract.
type opCall struct {
	route      *routers.Route
	pathParams map[string]string
	query      string
	body       []byte
	bodyType   string
}

// newOpCall builds a request that satisfies the contract for the operation:
// sample values for path parameters and required query parameters, and a
// sample body when the operation takes one.
func newOpCall(t *testing.T, doc *openapi3.T, path, method string, item *openapi3.PathItem, op *openapi3.Operation) opCall {
	t.Helper()
	c := opCall{
		route:      &routers.Route{Spec: doc, Path: path, PathItem: item, Method: method, Operation: op},
		pathParams: map[string]string{},
	}
	params := append(slices.Clone(item.Parameters), op.Parameters...)
	q := make([]string, 0, len(params))
	for _, pr := range params {
		p := pr.Value
		switch {
		case p.In == openapi3.ParameterInPath:
			c.pathParams[p.Name] = fmt.Sprint(sample(t, p.Schema.Value))
		case p.In == openapi3.ParameterInQuery && p.Required:
			q = append(q, p.Name+"="+fmt.Sprint(sample(t, p.Schema.Value)))
		}
	}
	c.query = strings.Join(q, "&")
	if op.RequestBody != nil {
		content := op.RequestBody.Value.Content
		types := slices.Sorted(maps.Keys(content))
		c.bodyType = types[0]
		if slices.Contains(types, "application/json") {
			c.bodyType = "application/json"
		}
		if c.bodyType == "application/json" {
			b, err := json.Marshal(sample(t, content[c.bodyType].Schema.Value))
			if err != nil {
				t.Fatal(err)
			}
			c.body = b
		} else {
			c.body = []byte("sample")
		}
	}
	return c
}

// url returns the request path under the API base path.
func (c opCall) url() string {
	p := c.route.Path
	for name, v := range c.pathParams {
		p = strings.ReplaceAll(p, "{"+name+"}", v)
	}
	if c.query != "" {
		p += "?" + c.query
	}
	return api.BasePath + p
}

func (c opCall) request(t *testing.T, token string) *http.Request {
	t.Helper()
	req := httptest.NewRequest(c.route.Method, c.url(), bytes.NewReader(c.body))
	if c.bodyType != "" {
		req.Header.Set("Content-Type", c.bodyType)
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	return req
}

// sample returns a value that s accepts. It covers the schema shapes that the
// contract uses for parameters and request bodies.
func sample(t *testing.T, s *openapi3.Schema) any {
	t.Helper()
	switch {
	case len(s.Enum) > 0:
		return s.Enum[0]
	case s.Default != nil:
		return s.Default
	case s.Type.Is(openapi3.TypeString):
		if s.Format == "date-time" {
			return "2026-11-03T00:00:00Z"
		}
		return "a1"
	case s.Type.Is(openapi3.TypeInteger), s.Type.Is(openapi3.TypeNumber):
		if s.Min != nil {
			return *s.Min
		}
		return 1
	case s.Type.Is(openapi3.TypeBoolean):
		return false
	case s.Type.Is(openapi3.TypeArray):
		return []any{}
	case s.Type.Is(openapi3.TypeObject), s.Type == nil:
		obj := map[string]any{}
		for _, name := range s.Required {
			obj[name] = sample(t, s.Properties[name].Value)
		}
		return obj
	}
	t.Fatalf("no sample for schema type %v", s.Type)
	return nil
}

// validate checks the request and the recorded response against the contract.
func (c opCall) validate(t *testing.T, rec *httptest.ResponseRecorder) {
	t.Helper()
	ctx := context.Background()
	in := &openapi3filter.RequestValidationInput{
		Request:    c.request(t, ""),
		PathParams: c.pathParams,
		Route:      c.route,
		Options:    &openapi3filter.Options{AuthenticationFunc: openapi3filter.NoopAuthenticationFunc},
	}
	if err := openapi3filter.ValidateRequest(ctx, in); err != nil {
		t.Fatalf("test request breaks the contract: %v", err)
	}
	out := &openapi3filter.ResponseValidationInput{
		RequestValidationInput: in,
		Status:                 rec.Code,
		Header:                 rec.Header(),
		Body:                   io.NopCloser(bytes.NewReader(rec.Body.Bytes())),
		Options:                &openapi3filter.Options{IncludeResponseStatus: true},
	}
	if err := openapi3filter.ValidateResponse(ctx, out); err != nil {
		t.Fatalf("response breaks the contract: %v\nbody: %s", err, rec.Body)
	}
}

// problemOf decodes a problem response and checks the fields that every
// problem of this package carries.
func problemOf(t *testing.T, rec *httptest.ResponseRecorder) api.Problem {
	t.Helper()
	if ct := rec.Header().Get("Content-Type"); ct != api.ProblemContentType {
		t.Fatalf("Content-Type = %q, want %q", ct, api.ProblemContentType)
	}
	var p api.Problem
	if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil {
		t.Fatalf("decode problem: %v", err)
	}
	if p.Status == nil || *p.Status != rec.Code {
		t.Fatalf("problem status %v, HTTP status %d", p.Status, rec.Code)
	}
	if p.Type == nil || *p.Type != api.ProblemTypeBlank || p.Title == nil || *p.Title != http.StatusText(rec.Code) {
		t.Fatalf("problem type %v title %v, want about:blank and %q", p.Type, p.Title, http.StatusText(rec.Code))
	}
	if p.Detail == nil || *p.Detail == "" {
		t.Fatal("problem has no detail")
	}
	return p
}

// TestContractEveryOperation calls every operation of the contract with each
// kind of token and validates each response against the contract.
func TestContractEveryOperation(t *testing.T) {
	doc := loadContract(t)
	h := newHandler(t)
	calls := 0
	for _, path := range slices.Sorted(maps.Keys(doc.Paths.Map())) {
		item := doc.Paths.Value(path)
		for _, method := range slices.Sorted(maps.Keys(item.Operations())) {
			op := item.GetOperation(method)
			name, _ := op.Extensions["x-required-scope"].(string)
			required, err := api.ParseScope(name)
			if err != nil {
				t.Fatalf("%s %s: %v", method, path, err)
			}
			c := newOpCall(t, doc, path, method, item, op)
			cases := []struct {
				name   string
				token  string
				status int
				auth   string
			}{
				{"required scope", testTokens[required], http.StatusNotImplemented, ""},
				{"admin scope", testTokens[api.ScopeAdmin], http.StatusNotImplemented, ""},
				{"no token", "", http.StatusUnauthorized, `Bearer realm="fps"`},
				{"unknown token", "not-a-token", http.StatusUnauthorized, `Bearer realm="fps", error="invalid_token"`},
			}
			if required > api.ScopeRead {
				cases = append(cases, struct {
					name   string
					token  string
					status int
					auth   string
				}{"lower scope", testTokens[required-1], http.StatusForbidden,
					fmt.Sprintf(`Bearer realm="fps", error="insufficient_scope", scope=%q`, required)})
			}
			for _, tc := range cases {
				t.Run(op.OperationID+"/"+tc.name, func(t *testing.T) {
					rec := httptest.NewRecorder()
					h.ServeHTTP(rec, c.request(t, tc.token))
					calls++
					if rec.Code != tc.status {
						t.Fatalf("%s %s: status %d, want %d\nbody: %s", method, c.url(), rec.Code, tc.status, rec.Body)
					}
					c.validate(t, rec)
					p := problemOf(t, rec)
					if got := rec.Header().Get("WWW-Authenticate"); got != tc.auth {
						t.Errorf("WWW-Authenticate = %q, want %q", got, tc.auth)
					}
					if tc.status == http.StatusNotImplemented && !strings.Contains(*p.Detail, op.OperationID+" ") {
						t.Errorf("detail %q does not name %s, so the request reached another operation", *p.Detail, op.OperationID)
					}
				})
			}
		}
	}
	if calls == 0 {
		t.Fatal("the contract has no operations")
	}
}

// TestContractParameterErrors checks that a request whose parameters the
// generated wrapper rejects gets a 400 problem that the contract allows.
func TestContractParameterErrors(t *testing.T) {
	doc := loadContract(t)
	h := newHandler(t)
	tests := []struct {
		path, method, query, want string
	}{
		{"/runs/{id}/metrics", http.MethodGet, "", "view"},
		{"/runs/{id}/metrics", http.MethodGet, "view=port&step_s=often", "step_s"},
		{"/modules", http.MethodGet, "page_size=many", "page_size"},
	}
	for _, tc := range tests {
		t.Run(tc.path+"?"+tc.query, func(t *testing.T) {
			item := doc.Paths.Value(tc.path)
			c := newOpCall(t, doc, tc.path, tc.method, item, item.GetOperation(tc.method))
			c.query = tc.query
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, c.request(t, testTokens[api.ScopeRead]))
			if rec.Code != http.StatusBadRequest {
				t.Fatalf("status %d, want 400\nbody: %s", rec.Code, rec.Body)
			}
			out := &openapi3filter.ResponseValidationInput{
				RequestValidationInput: &openapi3filter.RequestValidationInput{
					Request: c.request(t, ""), PathParams: c.pathParams, Route: c.route,
				},
				Status: rec.Code, Header: rec.Header(),
				Body:    io.NopCloser(bytes.NewReader(rec.Body.Bytes())),
				Options: &openapi3filter.Options{IncludeResponseStatus: true},
			}
			if err := openapi3filter.ValidateResponse(context.Background(), out); err != nil {
				t.Fatalf("response breaks the contract: %v", err)
			}
			if p := problemOf(t, rec); !strings.Contains(*p.Detail, tc.want) {
				t.Errorf("detail %q does not name %s", *p.Detail, tc.want)
			}
		})
	}
}

// TestUnroutedRequests checks the 404 and 405 problems for requests that match
// no operation, against the Problem schema of the contract.
func TestUnroutedRequests(t *testing.T) {
	doc := loadContract(t)
	problemSchema := doc.Components.Schemas["Problem"].Value
	h := newHandler(t)
	tests := []struct {
		method, path string
		status       int
		allow        string
	}{
		{http.MethodGet, "/v1/nothing", http.StatusNotFound, ""},
		{http.MethodGet, "/v1/runs/a1/extra/deep", http.StatusNotFound, ""},
		{http.MethodPost, "/v1/agents/a1:explode", http.StatusNotFound, ""},
		{http.MethodPost, "/v1/agents/:reserve", http.StatusNotFound, ""},
		{http.MethodPut, "/v1/runs", http.StatusMethodNotAllowed, "GET, POST"},
		{http.MethodDelete, "/v1/configs/a1:validate", http.StatusMethodNotAllowed, "POST"},
		{http.MethodGet, "/v1/configs/a1:validate", http.StatusMethodNotAllowed, "POST"},
		{http.MethodGet, "/v1/runs/a:b/report", http.StatusNotFound, ""},
		{http.MethodPatch, "/v1/modules/http/1.0.0", http.StatusMethodNotAllowed, "DELETE, GET"},
	}
	for _, tc := range tests {
		t.Run(tc.method+" "+tc.path, func(t *testing.T) {
			rec := httptest.NewRecorder()
			req := httptest.NewRequest(tc.method, tc.path, nil)
			req.Header.Set("Authorization", "Bearer "+testTokens[api.ScopeAdmin])
			h.ServeHTTP(rec, req)
			if rec.Code != tc.status {
				t.Fatalf("status %d, want %d\nbody: %s", rec.Code, tc.status, rec.Body)
			}
			if got := rec.Header().Get("Allow"); got != tc.allow {
				t.Errorf("Allow = %q, want %q", got, tc.allow)
			}
			problemOf(t, rec)
			var body any
			if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
				t.Fatal(err)
			}
			if err := problemSchema.VisitJSON(body); err != nil {
				t.Fatalf("body breaks the Problem schema: %v", err)
			}
		})
	}
}
