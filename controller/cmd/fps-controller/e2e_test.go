// SPDX-License-Identifier: GPL-3.0-only

package main

import (
	"archive/tar"
	"bufio"
	"bytes"
	"compress/gzip"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"syscall"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

// examplesDir holds the contract examples, relative to this package directory.
const examplesDir = "../../../contracts/examples"

// e2eTokens holds the API tokens of the end to end test by scope. The run
// token reaches the controller through a token file.
var e2eTokens = map[api.Scope]string{
	api.ScopeAdmin: "e2e-admin-0123456789",
	api.ScopeRun:   "e2e-run-0123456789",
	api.ScopeRead:  "e2e-read-0123456789",
}

// The tokens of the three scopes.
var (
	e2eAdminToken = e2eTokens[api.ScopeAdmin]
	e2eRunToken   = e2eTokens[api.ScopeRun]
	e2eReadToken  = e2eTokens[api.ScopeRead]
)

// controllerProcess is one started fps-controller binary.
type controllerProcess struct {
	cmd      *exec.Cmd
	logs     *lockedBuffer
	restAddr string
	grpcAddr string
	waitErr  chan error
}

func startController(t *testing.T, bin, cfgPath string) *controllerProcess {
	t.Helper()
	p := &controllerProcess{logs: &lockedBuffer{}, waitErr: make(chan error, 1)}
	p.cmd = exec.Command(bin, "-config", cfgPath)
	p.cmd.Stderr = p.logs
	if err := p.cmd.Start(); err != nil {
		t.Fatal(err)
	}
	go func() { p.waitErr <- p.cmd.Wait() }()
	t.Cleanup(func() { _ = p.cmd.Process.Kill() })
	p.restAddr = waitForRecord(t, p.logs, "controller listening")
	p.grpcAddr = waitForRecord(t, p.logs, "agent service listening")
	return p
}

// stop sends SIGTERM and expects exit code 0.
func (p *controllerProcess) stop(t *testing.T) {
	t.Helper()
	if err := p.cmd.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-p.waitErr:
		if err != nil {
			t.Fatalf("process exit = %v, want 0; logs:\n%s", err, p.logs.String())
		}
	case <-time.After(20 * time.Second):
		t.Fatalf("process did not exit within 20s of SIGTERM; logs:\n%s", p.logs.String())
	}
}

// call sends one REST request and decodes a JSON response into out when out
// is not nil. It fails the test when the status differs from want.
func (p *controllerProcess) call(t *testing.T, method, path, token, contentType string, body io.Reader, want int, out any) http.Header {
	t.Helper()
	req, err := http.NewRequest(method, "http://"+p.restAddr+path, body)
	if err != nil {
		t.Fatal(err)
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("%s %s: %v", method, path, err)
	}
	defer func() { _ = resp.Body.Close() }()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != want {
		t.Fatalf("%s %s: status %d, want %d\nbody: %s\nlogs:\n%s", method, path, resp.StatusCode, want, data, p.logs.String())
	}
	if out != nil {
		if err := json.Unmarshal(data, out); err != nil {
			t.Fatalf("%s %s: decode %s: %v", method, path, data, err)
		}
	}
	return resp.Header
}

func (p *controllerProcess) getJSON(t *testing.T, path, token string, out any) {
	t.Helper()
	p.call(t, http.MethodGet, path, token, "", nil, http.StatusOK, out)
}

func (p *controllerProcess) postJSON(t *testing.T, path, token, body string, want int, out any) http.Header {
	t.Helper()
	return p.call(t, http.MethodPost, path, token, "application/json", strings.NewReader(body), want, out)
}

// TestEndToEnd builds the binary, starts it with a temporary configuration,
// registers a fake agent over gRPC, installs and lists the example modules,
// stores and validates a configuration, starts a run over REST, reads a
// snapshot of the fake agent from the run metrics and from GET /metrics, stops
// the run, and checks after a restart that the configuration and the modules
// persist.
func TestEndToEnd(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGTERM is not deliverable on Windows")
	}
	if testing.Short() {
		t.Skip("builds the binary")
	}
	bin := filepath.Join(t.TempDir(), "fps-controller")
	if out, err := exec.Command("go", "build", "-o", bin, ".").CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}

	dir := t.TempDir()
	runTokenFile := filepath.Join(dir, "run.token")
	if err := os.WriteFile(runTokenFile, []byte(e2eRunToken+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	cfgPath := writeConfig(t, fmt.Sprintf(`listen: 127.0.0.1:0
grpc_listen: 127.0.0.1:0
log_level: debug
shutdown_timeout: 10s
data_dir: %s
tokens:
  - { name: admin, scope: admin, token: %s }
  - { name: ci, scope: run, token_file: %s }
  - { name: viewer, scope: read, token: %s }
`, filepath.Join(dir, "data"), e2eAdminToken, runTokenFile, e2eReadToken))
	p := startController(t, bin, cfgPath)

	// The API checks tokens and scopes.
	p.call(t, http.MethodGet, "/v1/agents", "", "", nil, http.StatusUnauthorized, nil)
	p.postJSON(t, "/v1/agents", e2eRunToken, `{"id":"local"}`, http.StatusForbidden, nil)

	// Enroll the agent and let the fake agent register over gRPC.
	p.postJSON(t, "/v1/agents", e2eAdminToken, `{"id":"local","labels":{"site":"e2e"}}`, http.StatusCreated, nil)
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	caps := fakeagent.DefaultCapabilities()
	caps.EngineApi = api.EngineAPIVersion
	caps.Ports = []*agentpb.PortInfo{{Name: "lo", Driver: "loopback", SpeedMbps: 10000, Mtu: 65536}}
	ag, err := fakeagent.Start(ctx, fakeagent.Options{
		ID:            "local",
		Capabilities:  caps,
		AutoHeartbeat: true,
		Dial: func(context.Context) (*grpc.ClientConn, error) {
			return grpc.NewClient(p.grpcAddr, grpc.WithTransportCredentials(insecure.NewCredentials()))
		},
	})
	if err != nil {
		t.Fatalf("fake agent registration: %v\nlogs:\n%s", err, p.logs.String())
	}
	defer ag.Close()
	var agentList []api.Agent
	p.getJSON(t, "/v1/agents", e2eReadToken, &agentList)
	if len(agentList) != 1 || agentList[0].Id != "local" || agentList[0].State != api.AgentStateOnline {
		t.Fatalf("GET /v1/agents = %+v, want agent local online", agentList)
	}

	// Install the example modules as gzip compressed tar archives and list them.
	for _, name := range []string{"http1", "dns"} {
		var sum api.ModuleSummary
		p.call(t, http.MethodPost, "/v1/modules", e2eAdminToken, "application/gzip",
			bytes.NewReader(moduleArchive(t, filepath.Join(examplesDir, "modules", name))), http.StatusCreated, &sum)
		if sum.Name != name {
			t.Errorf("installed module %q, want %q", sum.Name, name)
		}
	}
	wantModules := []string{"dns@0.1.0", "http1@0.1.0"}
	if got := listModules(t, p); !slices.Equal(got, wantModules) {
		t.Fatalf("GET /v1/modules = %v, want %v", got, wantModules)
	}
	var capsResp api.Capabilities
	p.getJSON(t, "/v1/capabilities", e2eReadToken, &capsResp)
	if len(capsResp.Agents) != 1 || len(capsResp.Modules) != 2 || !slices.Equal(capsResp.Backends, []api.CapabilitiesBackends{api.CapabilitiesBackendsKernel}) {
		t.Errorf("GET /v1/capabilities = %+v", capsResp)
	}

	// Store the example test configuration and validate it against the
	// registry and the agent inventory.
	smoke, err := os.ReadFile(filepath.Join(examplesDir, "tests", "http_smoke.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	var ref struct {
		ID string `json:"id"`
	}
	p.call(t, http.MethodPost, "/v1/configs", e2eRunToken, "application/yaml", bytes.NewReader(smoke), http.StatusCreated, &ref)
	if ref.ID == "" {
		t.Fatal("POST /v1/configs returned no id")
	}
	var report api.ValidationReport
	p.postJSON(t, "/v1/configs/"+ref.ID+":validate", e2eRunToken, "", http.StatusOK, &report)
	if !report.Valid || len(report.Findings) != 0 {
		t.Fatalf("validation report = %+v, want valid without findings", report)
	}

	// Start a run on the agent, wait until it runs, and stop it.
	var run api.Run
	hdr := p.postJSON(t, "/v1/runs", e2eRunToken, fmt.Sprintf(`{"config_id":%q,"agents":["local"]}`, ref.ID), http.StatusAccepted, &run)
	if run.Id == "" || hdr.Get("Location") != "/v1/runs/"+run.Id {
		t.Fatalf("POST /v1/runs = %+v, Location %q", run, hdr.Get("Location"))
	}
	waitRunState(t, p, run.Id, api.RunStateRunning)
	if s, _ := ag.State(); s != agentpb.AgentState_AGENT_STATE_RUNNING {
		t.Errorf("fake agent state = %v, want RUNNING", s)
	}
	checkRunMetrics(t, p, ag, run.Id)
	p.postJSON(t, "/v1/runs/"+run.Id+":stop", e2eRunToken, "{}", http.StatusAccepted, nil)
	final := waitRunState(t, p, run.Id, api.RunStateCompleted)
	if final.Verdict == nil || final.Verdict.Passed == nil || !*final.Verdict.Passed {
		t.Errorf("run verdict = %+v, want passed", final.Verdict)
	}
	var kinds []string
	for _, c := range ag.Commands() {
		switch {
		case c.GetPrepare() != nil:
			kinds = append(kinds, "prepare")
		case c.GetStart() != nil:
			kinds = append(kinds, "start")
		case c.GetStop() != nil:
			kinds = append(kinds, "stop")
		}
	}
	if want := []string{"prepare", "start", "stop"}; !slices.Equal(kinds, want) {
		t.Errorf("fake agent commands = %v, want %v", kinds, want)
	}

	p.stop(t)
	msgs := logMessages(t, p.logs.String())
	for _, want := range []string{"controller stopped", "agent service stopped"} {
		if !msgs[want] {
			t.Errorf("logs lack %q", want)
		}
	}

	// A restart on the same data directory keeps configurations and modules.
	p2 := startController(t, bin, cfgPath)
	p2.getJSON(t, "/v1/configs/"+ref.ID, e2eReadToken, nil)
	if got := listModules(t, p2); !slices.Equal(got, wantModules) {
		t.Errorf("modules after restart = %v, want %v", got, wantModules)
	}
	p2.stop(t)
}

// checkRunMetrics sends one counter snapshot from the fake agent and waits
// until the run metrics and the Prometheus exposition both carry its count.
func checkRunMetrics(t *testing.T, p *controllerProcess, ag *fakeagent.Agent, runID string) {
	t.Helper()
	const attempted = 7
	if _, err := ag.Snapshot(&agentpb.CounterValue{
		Metric: "sessions.attempted",
		Labels: map[string]string{"application": "http1", "scenario": "web_get"},
		Value:  &agentpb.CounterValue_Count{Count: attempted},
	}); err != nil {
		t.Fatalf("fake agent snapshot: %v", err)
	}
	wantLine := fmt.Sprintf(`fps_sessions_attempted_total{run_id=%q,application="http1",scenario="web_get"} %d`, runID, attempted)
	var restOK, promOK bool
	var set struct {
		View    string `json:"view"`
		Samples []struct {
			Key    string             `json:"key"`
			Values map[string]float64 `json:"values"`
		} `json:"samples"`
	}
	var exposition string
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) && (!restOK || !promOK) {
		p.getJSON(t, "/v1/runs/"+runID+"/metrics?view=application", e2eReadToken, &set)
		for _, s := range set.Samples {
			if s.Key == "http1" && s.Values["sessions.attempted"] == attempted {
				restOK = true
			}
		}
		exposition = p.getText(t, "/metrics")
		promOK = slices.Contains(strings.Split(exposition, "\n"), wantLine)
		time.Sleep(50 * time.Millisecond)
	}
	if !restOK {
		t.Errorf("GET /v1/runs/%s/metrics?view=application = %+v, want key http1 with sessions.attempted %d", runID, set, attempted)
	}
	if !promOK {
		t.Errorf("GET /metrics lacks %q:\n%s", wantLine, exposition)
	}
}

// getText sends GET path without a token and returns the body of a 200
// response.
func (p *controllerProcess) getText(t *testing.T, path string) string {
	t.Helper()
	resp, err := http.Get("http://" + p.restAddr + path)
	if err != nil {
		t.Fatalf("GET %s: %v", path, err)
	}
	defer func() { _ = resp.Body.Close() }()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("GET %s: status %d, want 200\nbody: %s", path, resp.StatusCode, data)
	}
	return string(data)
}

// listModules returns name@version of every installed module.
func listModules(t *testing.T, p *controllerProcess) []string {
	t.Helper()
	var page struct {
		Items []api.ModuleSummary `json:"items"`
	}
	p.getJSON(t, "/v1/modules", e2eReadToken, &page)
	var out []string
	for _, m := range page.Items {
		out = append(out, m.Name+"@"+m.Version)
	}
	return out
}

// waitRunState polls GET /v1/runs/{id} until the run reaches want, and fails
// when it reaches another final state first.
func waitRunState(t *testing.T, p *controllerProcess, id string, want api.RunState) api.Run {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	var run api.Run
	for time.Now().Before(deadline) {
		p.getJSON(t, "/v1/runs/"+id, e2eReadToken, &run)
		if run.State == want {
			return run
		}
		switch run.State {
		case api.RunStateCompleted, api.RunStateFailed, api.RunStateAborted:
			t.Fatalf("run %s ended %s, want %s; logs:\n%s", id, run.State, want, p.logs.String())
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("run %s is %s after 20s, want %s; logs:\n%s", id, run.State, want, p.logs.String())
	return run
}

// moduleArchive packs the module tree in dir as a gzip compressed tar file
// with manifest.yaml at its root.
func moduleArchive(t *testing.T, dir string) []byte {
	t.Helper()
	var buf bytes.Buffer
	zw := gzip.NewWriter(&buf)
	tw := tar.NewWriter(zw)
	if err := tw.AddFS(os.DirFS(dir)); err != nil {
		t.Fatalf("pack %s: %v", dir, err)
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

// waitForRecord reads JSON log lines until a record with msg appears and
// returns its addr attribute.
func waitForRecord(t *testing.T, logs *lockedBuffer, msg string) string {
	t.Helper()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		sc := bufio.NewScanner(strings.NewReader(logs.String()))
		for sc.Scan() {
			var rec struct {
				Msg  string `json:"msg"`
				Addr string `json:"addr"`
			}
			if json.Unmarshal(sc.Bytes(), &rec) == nil && rec.Msg == msg {
				return rec.Addr
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("no %q record within 15s; logs:\n%s", msg, logs.String())
	return ""
}
