// SPDX-License-Identifier: GPL-3.0-only

package api_test

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// contractsReadme is the contracts README, relative to this package directory.
const contractsReadme = "../../../contracts/README.md"

// fakeModules is a module lister with a fixed answer.
type fakeModules struct {
	list []api.ModuleSummary
	err  error
}

func (f fakeModules) ModuleSummaries(context.Context) ([]api.ModuleSummary, error) {
	return f.list, f.err
}

// fakeAgents is an agent inventory with a fixed answer.
type fakeAgents struct {
	list []api.AgentCapability
	err  error
}

func (f fakeAgents) AgentCapabilities(context.Context) ([]api.AgentCapability, error) {
	return f.list, f.err
}

func ptrTo[T any](v T) *T { return &v }

// twoAgents returns two fake agents. The first supports kernel and dpdk, and
// the second, which is offline and never registered, supports none.
func twoAgents() []api.AgentCapability {
	backend := api.AgentBackendKernel
	ports := []struct {
		Driver    *string `json:"driver,omitempty"`
		Name      string  `json:"name"`
		SpeedMbps *int    `json:"speed_mbps,omitempty"`
	}{{Name: "eth1", Driver: ptrTo("ixgbe"), SpeedMbps: ptrTo(10000)}}
	return []api.AgentCapability{
		{
			Agent: api.Agent{
				Id: "agent-a", State: "online", Address: ptrTo("192.0.2.10:40000"),
				Version: ptrTo("0.1.0"), Cores: ptrTo(8), Backend: &backend, Ports: &ports,
				Labels: &map[string]string{"rack": "r1"},
			},
			Backends: []api.CapabilitiesBackends{api.CapabilitiesBackendsDpdk, api.CapabilitiesBackendsKernel},
		},
		{Agent: api.Agent{Id: "agent-b", State: "offline"}},
	}
}

// twoModules returns two installed modules.
func twoModules() []api.ModuleSummary {
	return []api.ModuleSummary{
		{Name: "dns", Version: "0.1.0", Type: api.ModuleTypeApp, License: "GPL-3.0-only", Scenarios: &[]string{"lookup"}},
		{Name: "http1", Version: "1.0.0", Type: api.ModuleTypeApp, License: "GPL-3.0-only", Summary: ptrTo("HTTP/1.1")},
	}
}

func capabilitiesHandler(t *testing.T, impl api.ServerInterface) http.Handler {
	t.Helper()
	tokens := make(map[string]api.Scope, len(testTokens))
	for s, tok := range testTokens {
		tokens[tok] = s
	}
	auth, err := api.NewStaticTokens(tokens)
	if err != nil {
		t.Fatal(err)
	}
	return api.New(impl, auth, slog.New(slog.DiscardHandler)).Handler()
}

// getCapabilities calls GET /v1/capabilities with a read token and validates
// the response against the contract.
func getCapabilities(t *testing.T, impl api.ServerInterface) *httptest.ResponseRecorder {
	t.Helper()
	doc := loadContract(t)
	item := doc.Paths.Value("/capabilities")
	c := newOpCall(t, doc, "/capabilities", http.MethodGet, item, item.Get)
	rec := httptest.NewRecorder()
	capabilitiesHandler(t, impl).ServeHTTP(rec, c.request(t, testTokens[api.ScopeRead]))
	c.validate(t, rec)
	return rec
}

// TestContractCapabilities is the acceptance test of CTL-10. It serves two
// fake agents and two modules and checks the response against the contract.
func TestContractCapabilities(t *testing.T) {
	agents, modules := twoAgents(), twoModules()
	rec := getCapabilities(t, api.NewCapabilitiesAPI(fakeModules{list: modules}, fakeAgents{list: agents}))
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d, want 200\nbody: %s", rec.Code, rec.Body)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "application/json" {
		t.Errorf("Content-Type = %q", ct)
	}
	var got api.Capabilities
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.EngineApi != api.EngineAPIVersion || got.ApiVersion == nil || *got.ApiVersion != api.APIVersion {
		t.Errorf("engine_api %q api_version %v, want %q and %q", got.EngineApi, got.ApiVersion, api.EngineAPIVersion, api.APIVersion)
	}
	wantBackends := []api.CapabilitiesBackends{api.CapabilitiesBackendsKernel, api.CapabilitiesBackendsDpdk}
	if !reflect.DeepEqual(got.Backends, wantBackends) {
		t.Errorf("backends = %v, want %v", got.Backends, wantBackends)
	}
	wantAgents := []api.Agent{agents[0].Agent, agents[1].Agent}
	if !reflect.DeepEqual(got.Agents, wantAgents) {
		t.Errorf("agents = %+v, want %+v", got.Agents, wantAgents)
	}
	if !reflect.DeepEqual(got.Modules, modules) {
		t.Errorf("modules = %+v, want %+v", got.Modules, modules)
	}
}

// TestCapabilitiesControllerVersion checks that the option sets
// controller_version, and that a build without a version leaves the field out.
func TestCapabilitiesControllerVersion(t *testing.T) {
	rec := getCapabilities(t, api.NewCapabilitiesAPI(nil, nil, api.WithControllerVersion("1.4.0-rc1")))
	var got api.Capabilities
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.ControllerVersion == nil || *got.ControllerVersion != "1.4.0-rc1" {
		t.Errorf("controller_version = %v, want 1.4.0-rc1", got.ControllerVersion)
	}
	if got.ApiVersion == nil || *got.ApiVersion != api.APIVersion {
		t.Errorf("api_version = %v, want %q: the controller version must not replace it", got.ApiVersion, api.APIVersion)
	}
	for name, impl := range map[string]*api.CapabilitiesAPI{
		"no option":     api.NewCapabilitiesAPI(nil, nil),
		"empty version": api.NewCapabilitiesAPI(nil, nil, api.WithControllerVersion("")),
	} {
		t.Run(name, func(t *testing.T) {
			if body := getCapabilities(t, impl).Body.String(); strings.Contains(body, "controller_version") {
				t.Errorf("body %s carries controller_version", body)
			}
		})
	}
}

// TestCapabilitiesEmpty checks that no agents and no modules, and nil
// sources, give empty arrays, which the contract requires, not null.
func TestCapabilitiesEmpty(t *testing.T) {
	for name, impl := range map[string]*api.CapabilitiesAPI{
		"empty sources": api.NewCapabilitiesAPI(fakeModules{}, fakeAgents{}),
		"nil sources":   api.NewCapabilitiesAPI(nil, nil),
	} {
		t.Run(name, func(t *testing.T) {
			rec := getCapabilities(t, impl)
			if rec.Code != http.StatusOK {
				t.Fatalf("status %d, want 200\nbody: %s", rec.Code, rec.Body)
			}
			body := rec.Body.String()
			for _, field := range []string{`"backends":[]`, `"agents":[]`, `"modules":[]`} {
				if !strings.Contains(body, field) {
					t.Errorf("body %s lacks %s", body, field)
				}
			}
		})
	}
}

// TestCapabilitiesSourceErrors checks that a failing source gives a 500
// problem that the contract allows and that names the source.
func TestCapabilitiesSourceErrors(t *testing.T) {
	boom := errors.New("boom")
	tests := []struct {
		name string
		impl *api.CapabilitiesAPI
		want string
	}{
		{"agents", api.NewCapabilitiesAPI(fakeModules{list: twoModules()}, fakeAgents{err: boom}), "list agents: boom"},
		{"modules", api.NewCapabilitiesAPI(fakeModules{err: boom}, fakeAgents{list: twoAgents()}), "list modules: boom"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			rec := getCapabilities(t, tc.impl)
			if rec.Code != http.StatusInternalServerError {
				t.Fatalf("status %d, want 500\nbody: %s", rec.Code, rec.Body)
			}
			if p := problemOf(t, rec); !strings.Contains(*p.Detail, tc.want) {
				t.Errorf("detail %q does not contain %q", *p.Detail, tc.want)
			}
			if _, err := tc.impl.Capabilities(context.Background()); !errors.Is(err, boom) {
				t.Errorf("Capabilities error %v does not wrap the cause", err)
			}
		})
	}
}

// TestCapabilitiesOtherOperations checks that the type leaves every other
// operation to Unimplemented.
func TestCapabilitiesOtherOperations(t *testing.T) {
	h := capabilitiesHandler(t, api.NewCapabilitiesAPI(nil, nil))
	req := httptest.NewRequest(http.MethodGet, api.BasePath+"/agents", nil)
	req.Header.Set("Authorization", "Bearer "+testTokens[api.ScopeRead])
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusNotImplemented {
		t.Fatalf("GET /v1/agents status %d, want 501", rec.Code)
	}
}

// TestCapabilitiesVersions keeps the reported versions in step with the
// contract files.
func TestCapabilitiesVersions(t *testing.T) {
	doc := loadContract(t)
	if doc.Info.Version != api.APIVersion {
		t.Errorf("APIVersion = %q, contract info.version = %q", api.APIVersion, doc.Info.Version)
	}
	f, err := os.Open(contractsReadme)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = f.Close() }()
	sc := bufio.NewScanner(f)
	if !sc.Scan() {
		t.Fatalf("read %s: %v", contractsReadme, sc.Err())
	}
	minor, ok := strings.CutPrefix(sc.Text(), "# Contracts v")
	if !ok {
		t.Fatalf("first line of %s is %q, want a Contracts heading", contractsReadme, sc.Text())
	}
	if !strings.HasPrefix(api.EngineAPIVersion, minor+".") {
		t.Errorf("EngineAPIVersion = %q, contracts README names v%s", api.EngineAPIVersion, minor)
	}
}
