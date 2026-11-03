// SPDX-License-Identifier: GPL-3.0-only

package agents_test

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/getkin/kin-openapi/openapi3"
	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/peer"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

const contractPath = "../../../contracts/openapi/fps-api-v1.yaml"

// restHarness serves the API of a harness manager behind the real router and
// token check, and validates every exchange against the contract.
type restHarness struct {
	doc     *openapi3.T
	handler http.Handler
}

func newRESTHarness(t *testing.T, m *agents.Manager) *restHarness {
	t.Helper()
	doc, err := contract.Load(contractPath)
	if err != nil {
		t.Fatal(err)
	}
	auth, err := api.NewStaticTokens(map[string]api.Scope{"token-admin": api.ScopeAdmin})
	if err != nil {
		t.Fatal(err)
	}
	return &restHarness{doc: doc, handler: api.New(agents.NewAPI(m), auth, slog.New(slog.DiscardHandler)).Handler()}
}

// call sends one request to the operation at path and method and validates
// the request and the response against the contract.
func (h *restHarness) call(t *testing.T, method, path string, params map[string]string, body string) *httptest.ResponseRecorder {
	t.Helper()
	item := h.doc.Paths.Value(path)
	if item == nil || item.GetOperation(method) == nil {
		t.Fatalf("the contract has no %s %s", method, path)
	}
	url := path
	for k, v := range params {
		url = strings.ReplaceAll(url, "{"+k+"}", v)
	}
	newReq := func() *http.Request {
		r := httptest.NewRequest(method, api.BasePath+url, strings.NewReader(body))
		if body != "" {
			r.Header.Set("Content-Type", "application/json")
		}
		r.Header.Set("Authorization", "Bearer token-admin")
		return r
	}
	rec := httptest.NewRecorder()
	h.handler.ServeHTTP(rec, newReq())
	in := &openapi3filter.RequestValidationInput{
		Request:    newReq(),
		PathParams: params,
		Route:      &routers.Route{Spec: h.doc, Path: path, PathItem: item, Method: method, Operation: item.GetOperation(method)},
		Options:    &openapi3filter.Options{AuthenticationFunc: openapi3filter.NoopAuthenticationFunc},
	}
	out := &openapi3filter.ResponseValidationInput{
		RequestValidationInput: in,
		Status:                 rec.Code,
		Header:                 rec.Header(),
		Body:                   io.NopCloser(bytes.NewReader(rec.Body.Bytes())),
		Options:                &openapi3filter.Options{IncludeResponseStatus: true},
	}
	if err := openapi3filter.ValidateResponse(context.Background(), out); err != nil {
		t.Fatalf("%s %s: response breaks the contract: %v\nbody: %s", method, url, err, rec.Body)
	}
	return rec
}

func decode[T any](t *testing.T, rec *httptest.ResponseRecorder) T {
	t.Helper()
	var v T
	if err := json.Unmarshal(rec.Body.Bytes(), &v); err != nil {
		t.Fatalf("decode %s: %v", rec.Body, err)
	}
	return v
}

func TestRESTAgents(t *testing.T) {
	h := newHarness(t, nil)
	rh := newRESTHarness(t, h.mgr)

	rec := rh.call(t, http.MethodGet, "/agents", nil, "")
	if rec.Code != http.StatusOK || strings.TrimSpace(rec.Body.String()) != "[]" {
		t.Fatalf("empty list: %d %s", rec.Code, rec.Body)
	}

	rec = rh.call(t, http.MethodPost, "/agents", nil, `{"id":"agent-a","labels":{"rack":"r2"}}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("enroll: %d %s", rec.Code, rec.Body)
	}
	if ag := decode[api.Agent](t, rec); ag.Id != "agent-a" || ag.State != api.AgentStateOffline || (*ag.Labels)["rack"] != "r2" {
		t.Fatalf("enrolled agent %+v", ag)
	}

	id := map[string]string{"id": "agent-a"}
	if rec = rh.call(t, http.MethodPost, "/agents/{id}:reserve", id, ""); rec.Code != http.StatusConflict {
		t.Fatalf("reserve offline: %d %s", rec.Code, rec.Body)
	}

	caps := fakeagent.DefaultCapabilities()
	caps.Backends = []agentpb.Backend{agentpb.Backend_BACKEND_UNSPECIFIED, agentpb.Backend_BACKEND_AF_XDP}
	caps.Labels = map[string]string{"rack": "reported", "zone": "z1"}
	h.start(t, fakeagent.Options{ID: "agent-a", Capabilities: caps})

	rec = rh.call(t, http.MethodGet, "/agents", nil, "")
	list := decode[[]api.Agent](t, rec)
	if len(list) != 1 {
		t.Fatalf("list %s", rec.Body)
	}
	ag := list[0]
	if ag.State != api.AgentStateOnline || ag.Address == nil || *ag.Cores != 2 || *ag.Backend != api.AgentBackendAfXdp ||
		*ag.Version != "fake-0.0.0" || len(*ag.Ports) != 1 || (*ag.Ports)[0].Name != "eth1" || *(*ag.Ports)[0].SpeedMbps != 10000 {
		t.Fatalf("online agent %s", rec.Body)
	}
	if (*ag.Labels)["rack"] != "r2" || (*ag.Labels)["zone"] != "z1" {
		t.Fatalf("enrolled labels must override reported labels: %v", *ag.Labels)
	}

	rec = rh.call(t, http.MethodPost, "/agents/{id}:reserve", id, "")
	if rec.Code != http.StatusOK || decode[api.Agent](t, rec).State != api.AgentStateReserved {
		t.Fatalf("reserve: %d %s", rec.Code, rec.Body)
	}
	if _, err := h.mgr.Reserve("agent-a", "run-1"); !errors.Is(err, agents.ErrAgentReserved) {
		t.Fatalf("a run reserved an agent that the operator holds: %v", err)
	}
	rec = rh.call(t, http.MethodPost, "/agents/{id}:release", id, "")
	if rec.Code != http.StatusOK || decode[api.Agent](t, rec).State != api.AgentStateOnline {
		t.Fatalf("release: %d %s", rec.Code, rec.Body)
	}
	if _, err := h.mgr.Reserve("agent-a", "run-1"); err != nil {
		t.Fatal(err)
	}
	if rec = rh.call(t, http.MethodPost, "/agents/{id}:reserve", id, ""); rec.Code != http.StatusConflict {
		t.Fatalf("reserve of a run agent: %d %s", rec.Code, rec.Body)
	}
	if rec = rh.call(t, http.MethodPost, "/agents/{id}:release", id, ""); rec.Code != http.StatusOK {
		t.Fatalf("operator release of a run agent: %d %s", rec.Code, rec.Body)
	}

	missing := map[string]string{"id": "nobody"}
	for _, p := range []string{"/agents/{id}:reserve", "/agents/{id}:release"} {
		if rec = rh.call(t, http.MethodPost, p, missing, ""); rec.Code != http.StatusNotFound {
			t.Fatalf("%s of an unknown agent: %d %s", p, rec.Code, rec.Body)
		}
	}
}

func TestRESTEnrollRejects(t *testing.T) {
	m, err := agents.New(agents.Config{})
	if err != nil {
		t.Fatal(err)
	}
	rh := newRESTHarness(t, m)
	cases := []struct {
		body   string
		status int
	}{
		{`{"id":"a b"}`, http.StatusUnprocessableEntity},
		{`{"id":"a","extra":1}`, http.StatusBadRequest},
		{`{"id":`, http.StatusBadRequest},
		{`{"id":"a"} {"id":"b"}`, http.StatusBadRequest},
	}
	for _, tc := range cases {
		rec := rh.call(t, http.MethodPost, "/agents", nil, tc.body)
		if rec.Code != tc.status {
			t.Errorf("enroll %s: %d, want %d: %s", tc.body, rec.Code, tc.status, rec.Body)
		}
	}
	if n := len(m.List()); n != 0 {
		t.Fatalf("rejected enrollments entered the inventory: %v", m.List())
	}
}

func TestPeerCertIdentity(t *testing.T) {
	withCert := func(c *x509.Certificate) context.Context {
		state := tls.ConnectionState{VerifiedChains: [][]*x509.Certificate{{c}}}
		return peer.NewContext(context.Background(), &peer.Peer{AuthInfo: credentials.TLSInfo{State: state}})
	}
	cases := []struct {
		name string
		ctx  context.Context
		want string
	}{
		{"common name", withCert(&x509.Certificate{Subject: pkix.Name{CommonName: "agent-a"}, DNSNames: []string{"dns-a"}}), "agent-a"},
		{"dns name", withCert(&x509.Certificate{DNSNames: []string{"agent-b"}}), "agent-b"},
		{"no name", withCert(&x509.Certificate{}), ""},
		{"no peer", context.Background(), ""},
		{"no tls", peer.NewContext(context.Background(), &peer.Peer{}), ""},
		{"unverified", peer.NewContext(context.Background(), &peer.Peer{AuthInfo: credentials.TLSInfo{}}), ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, err := agents.PeerCertIdentity(tc.ctx, "claimed")
			if tc.want == "" {
				if !errors.Is(err, agents.ErrNoIdentity) {
					t.Fatalf("PeerCertIdentity = %q, %v, want ErrNoIdentity", got, err)
				}
				return
			}
			if err != nil || got != tc.want {
				t.Fatalf("PeerCertIdentity = %q, %v, want %q", got, err, tc.want)
			}
		})
	}
}
