// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"archive/tar"
	"bytes"
	"context"
	"encoding/json"
	"io/fs"
	"maps"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
	"testing/fstest"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
	"github.com/alrawi/FixingPointSystem/controller/internal/app"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// Paths relative to this package directory.
const (
	contractPath = "../../../contracts/openapi/fps-api-v1.yaml"
	examplesDir  = "../../../contracts/examples/modules"
)

// Tokens of the tests.
const (
	adminToken = "admin-token-0123456789"
	readToken  = "read-token-0123456789"
)

// pendingOps are the operations that no package implements yet.
var pendingOps = []string{
	"downloadCapture", "getRunMetrics", "listCaptures",
	"listRuns", "startCapture", "streamRunMetrics",
}

func testConfig(t *testing.T, dataDir string) app.Config {
	t.Helper()
	cfg := app.DefaultConfig()
	cfg.DataDir = dataDir
	cfg.GRPCListen = "127.0.0.1:0"
	cfg.ShutdownTimeout = app.Duration(5 * time.Second)
	cfg.Tokens = []app.Token{
		{Name: "admin", Scope: "admin", Token: adminToken},
		{Name: "read", Scope: "read", Token: readToken},
	}
	if err := cfg.Validate(); err != nil {
		t.Fatal(err)
	}
	return cfg
}

func newController(t *testing.T, cfg app.Config) *Controller {
	t.Helper()
	c, err := New(context.Background(), Options{Config: cfg})
	if err != nil {
		t.Fatalf("New error = %v", err)
	}
	t.Cleanup(func() {
		if err := c.Close(); err != nil {
			t.Errorf("Close error = %v", err)
		}
	})
	return c
}

func newHandler(t *testing.T, c *Controller) http.Handler {
	t.Helper()
	mux := http.NewServeMux()
	c.Mount(mux)
	return mux
}

func do(t *testing.T, h http.Handler, method, url, token, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, url, strings.NewReader(body))
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

var paramRE = regexp.MustCompile(`\{[a-z_]+\}`)

// TestServerRoutesEveryOperation calls every operation of the contract with
// an admin token and checks that exactly the pending operations answer 501,
// each with a detail that names it, so every other operation reached the
// package that implements it.
func TestServerRoutesEveryOperation(t *testing.T) {
	doc, err := contract.Load(contractPath)
	if err != nil {
		t.Fatal(err)
	}
	h := newHandler(t, newController(t, testConfig(t, "")))
	var unimplemented []string
	for _, path := range slices.Sorted(maps.Keys(doc.Paths.Map())) {
		item := doc.Paths.Value(path)
		for method, op := range item.Operations() {
			url := api.BasePath + paramRE.ReplaceAllString(path, "x1")
			if op.OperationID == "getRunMetrics" {
				url += "?view=port"
			}
			body := ""
			if method == http.MethodPost {
				body = "{}"
			}
			rec := do(t, h, method, url, adminToken, body)
			if rec.Code != http.StatusNotImplemented {
				continue
			}
			unimplemented = append(unimplemented, op.OperationID)
			if !strings.Contains(rec.Body.String(), op.OperationID+" ") {
				t.Errorf("%s %s: 501 body %s does not name %s", method, url, rec.Body, op.OperationID)
			}
		}
	}
	slices.Sort(unimplemented)
	if !slices.Equal(unimplemented, pendingOps) {
		t.Errorf("operations that answer 501 = %v, want %v", unimplemented, pendingOps)
	}
}

func TestMountChecksTokens(t *testing.T) {
	h := newHandler(t, newController(t, testConfig(t, "")))
	if rec := do(t, h, http.MethodGet, "/v1/agents", "", ""); rec.Code != http.StatusUnauthorized {
		t.Errorf("no token: status %d, want 401", rec.Code)
	}
	if rec := do(t, h, http.MethodGet, "/v1/agents", readToken, ""); rec.Code != http.StatusOK {
		t.Errorf("read token on listAgents: status %d, want 200; body %s", rec.Code, rec.Body)
	}
	if rec := do(t, h, http.MethodPost, "/v1/agents", readToken, `{"id":"a"}`); rec.Code != http.StatusForbidden {
		t.Errorf("read token on registerAgent: status %d, want 403", rec.Code)
	}
}

func TestNewRejectsBadTokenFile(t *testing.T) {
	cfg := testConfig(t, "")
	cfg.Tokens = []app.Token{{Name: "x", Scope: "read", TokenFile: filepath.Join(t.TempDir(), "absent")}}
	if _, err := New(context.Background(), Options{Config: cfg}); err == nil {
		t.Fatal("New error = nil, want an error for the missing token file")
	}
}

// fakeMetrics records the metrics operations that reach it.
type fakeMetrics struct{ calls []string }

func (f *fakeMetrics) GetRunMetrics(w http.ResponseWriter, _ *http.Request, id api.Id, p api.GetRunMetricsParams) {
	f.calls = append(f.calls, "get "+id+" "+string(p.View))
	w.WriteHeader(http.StatusTeapot)
}

func (f *fakeMetrics) StreamRunMetrics(w http.ResponseWriter, _ *http.Request, id api.Id, _ api.StreamRunMetricsParams) {
	f.calls = append(f.calls, "stream "+id)
	w.WriteHeader(http.StatusTeapot)
}

func TestSetMetricsRoutesMetricsOperations(t *testing.T) {
	c := newController(t, testConfig(t, ""))
	m := &fakeMetrics{}
	c.SetMetrics(m)
	h := newHandler(t, c)
	for _, url := range []string{"/v1/runs/r1/metrics?view=tcp", "/v1/runs/r1/metrics:stream"} {
		if rec := do(t, h, http.MethodGet, url, readToken, ""); rec.Code != http.StatusTeapot {
			t.Errorf("GET %s: status %d, want 418", url, rec.Code)
		}
	}
	if want := []string{"get r1 tcp", "stream r1"}; !slices.Equal(m.calls, want) {
		t.Errorf("metrics calls = %v, want %v", m.calls, want)
	}
}

// fakeReport records the report operations that reach it.
type fakeReport struct{ calls []string }

func (f *fakeReport) GetRunReport(w http.ResponseWriter, _ *http.Request, id api.Id, p api.GetRunReportParams) {
	format := "default"
	if p.Format != nil {
		format = string(*p.Format)
	}
	f.calls = append(f.calls, id+" "+format)
	w.WriteHeader(http.StatusTeapot)
}

func TestNewServesRunReports(t *testing.T) {
	c := newController(t, testConfig(t, ""))
	if c.Evaluator == nil {
		t.Fatal("Evaluator = nil, want the CTL-09 evaluator")
	}
	h := newHandler(t, c)
	rec := do(t, h, http.MethodGet, "/v1/runs/absent/report", readToken, "")
	if rec.Code != http.StatusNotFound || !strings.Contains(rec.Body.String(), "absent") {
		t.Errorf("GET /v1/runs/absent/report: status %d body %s, want a 404 problem that names the run", rec.Code, rec.Body)
	}
}

func TestSetReportRoutesReportOperation(t *testing.T) {
	c := newController(t, testConfig(t, ""))
	f := &fakeReport{}
	c.SetReport(f)
	h := newHandler(t, c)
	for _, url := range []string{"/v1/runs/r1/report", "/v1/runs/r1/report?format=csv"} {
		if rec := do(t, h, http.MethodGet, url, readToken, ""); rec.Code != http.StatusTeapot {
			t.Errorf("GET %s: status %d, want 418", url, rec.Code)
		}
	}
	if want := []string{"r1 default", "r1 csv"}; !slices.Equal(f.calls, want) {
		t.Errorf("report calls = %v, want %v", f.calls, want)
	}
}

func TestMemoryStorePathMatchesStore(t *testing.T) {
	if app.MemoryStorePath != store.MemoryPath {
		t.Fatalf("app.MemoryStorePath = %q, store.MemoryPath = %q", app.MemoryStorePath, store.MemoryPath)
	}
}

// installExamples installs every example module into reg.
func installExamples(t *testing.T, reg *registry.Registry) {
	t.Helper()
	for _, name := range []string{"dns", "http1"} {
		if _, err := reg.InstallDir(context.Background(), filepath.Join(examplesDir, name)); err != nil {
			t.Fatalf("InstallDir(%s) = %v", name, err)
		}
	}
}

func TestCatalogListsLatestScenarios(t *testing.T) {
	ctx := context.Background()
	reg, err := registry.Open(t.TempDir(), registry.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	installExamples(t, reg)

	// A second http1 version must replace the first in the catalog.
	if _, err := reg.Install(ctx, bytes.NewReader(bumpedArchive(t, filepath.Join(examplesDir, "http1"), "0.2.0"))); err != nil {
		t.Fatal(err)
	}

	got, err := Catalog{Registry: reg}.Scenarios(ctx)
	if err != nil {
		t.Fatalf("Scenarios error = %v", err)
	}
	var refs []string
	for _, s := range got {
		refs = append(refs, s.Module.Name+"@"+s.Module.Version+"/"+s.Name)
		if !strings.HasPrefix(s.Module.Digest, "sha256:") {
			t.Errorf("%s/%s: digest %q", s.Module.Name, s.Name, s.Module.Digest)
		}
		if !strings.Contains(s.Source, "scenario: "+s.Name) {
			t.Errorf("%s/%s: source does not hold its scenario name:\n%s", s.Module.Name, s.Name, s.Source)
		}
	}
	slices.Sort(refs)
	want := []string{"dns@0.1.0/lookup", "dns@0.1.0/resolve", "http1@0.2.0/web_browse", "http1@0.2.0/web_get", "http1@0.2.0/web_page"}
	if !slices.Equal(refs, want) {
		t.Errorf("scenarios = %v, want %v", refs, want)
	}

	sums, err := Catalog{Registry: reg}.ModuleSummaries(ctx)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, s := range sums {
		names = append(names, s.Name+"@"+s.Version)
	}
	if want := []string{"dns@0.1.0", "http1@0.1.0", "http1@0.2.0"}; !slices.Equal(names, want) {
		t.Errorf("module summaries = %v, want %v", names, want)
	}
}

// bumpedArchive packs the module tree in dir as a tar archive whose manifest
// carries version in place of 0.1.0.
func bumpedArchive(t *testing.T, dir, version string) []byte {
	t.Helper()
	src := os.DirFS(dir)
	tree := fstest.MapFS{}
	err := fs.WalkDir(src, ".", func(p string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		data, err := fs.ReadFile(src, p)
		if err != nil {
			return err
		}
		if p == "manifest.yaml" {
			data = bytes.Replace(data, []byte("version: 0.1.0"), []byte("version: "+version), 1)
		}
		tree[p] = &fstest.MapFile{Data: data, Mode: 0o644}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	tw := tar.NewWriter(&buf)
	if err := tw.AddFS(tree); err != nil {
		t.Fatal(err)
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

// fakeLister is a fixed agent list.
type fakeLister []agents.Agent

func (f fakeLister) List() []agents.Agent { return f }

func TestInventoryAdaptsAgents(t *testing.T) {
	caps := &agentpb.Capabilities{
		Cores:    4,
		Backends: []agentpb.Backend{agentpb.Backend_BACKEND_UNSPECIFIED, agentpb.Backend_BACKEND_AF_XDP, agentpb.Backend_BACKEND_KERNEL},
		Ports:    []*agentpb.PortInfo{{Name: "eth1"}, {Name: "eth2"}},
	}
	inv := Inventory{Agents: fakeLister{
		{ID: "a", State: api.AgentStateOnline, Capabilities: caps},
		{ID: "b", State: api.AgentStateReserved},
		{ID: "c", State: api.AgentStateBusy, Capabilities: caps},
		{ID: "d", State: api.AgentStateOffline, Capabilities: caps},
	}}
	got, err := inv.AvailableAgents(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].ID != "a" || got[1].ID != "b" {
		t.Fatalf("AvailableAgents = %+v, want agents a and b", got)
	}
	if a := got[0]; a.Cores != 4 || a.Backend != "af_xdp" || !slices.Equal(a.Interfaces, []string{"eth1", "eth2"}) {
		t.Errorf("agent a = %+v", a)
	}

	capsList, err := inv.AgentCapabilities(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(capsList) != 4 {
		t.Fatalf("AgentCapabilities returned %d agents, want 4", len(capsList))
	}
	want := []api.CapabilitiesBackends{api.CapabilitiesBackendsAfXdp, api.CapabilitiesBackendsKernel}
	if got := capsList[0].Backends; !slices.Equal(got, want) {
		t.Errorf("backends of a = %v, want %v", got, want)
	}
	if capsList[1].Backends != nil || capsList[1].Agent.Id != "b" {
		t.Errorf("agent b = %+v, want no backends", capsList[1])
	}
}

// TestRunServesAgentsAndStops runs the gRPC service on a loopback listener,
// registers a fake agent, checks the REST inventory, and stops.
func TestRunServesAgentsAndStops(t *testing.T) {
	cfg := testConfig(t, t.TempDir())
	cfg.AgentAutoEnroll = true
	c := newController(t, cfg)
	var events atomic.Int64
	c.Observe(func(agents.Event) { events.Add(1) })
	h := newHandler(t, c)

	ctx, cancel := context.WithCancel(context.Background())
	ln, err := c.ListenGRPC(ctx)
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { done <- c.Run(ctx, ln) }()

	addr := ln.Addr().String()
	ag, err := fakeagent.Start(ctx, fakeagent.Options{ID: "agent-a", Dial: func(context.Context) (*grpc.ClientConn, error) {
		return grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	}})
	if err != nil {
		t.Fatalf("fake agent start = %v", err)
	}
	defer ag.Close()

	rec := do(t, h, http.MethodGet, "/v1/agents", readToken, "")
	var list []api.Agent
	if err := json.Unmarshal(rec.Body.Bytes(), &list); err != nil {
		t.Fatalf("decode %s: %v", rec.Body, err)
	}
	if len(list) != 1 || list[0].Id != "agent-a" || list[0].State != api.AgentStateOnline {
		t.Fatalf("GET /v1/agents = %s", rec.Body)
	}
	installExamples(t, c.Registry)
	rec = do(t, h, http.MethodGet, "/v1/capabilities", readToken, "")
	var caps api.Capabilities
	if err := json.Unmarshal(rec.Body.Bytes(), &caps); err != nil {
		t.Fatal(err)
	}
	if len(caps.Agents) != 1 || len(caps.Modules) != 2 {
		t.Errorf("capabilities = %s, want one agent and two modules", rec.Body)
	}

	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("Run = %v, want nil", err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("Run did not return after cancel")
	}
	if events.Load() == 0 {
		t.Error("the observer saw no agent event")
	}
}
