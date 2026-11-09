// SPDX-License-Identifier: GPL-3.0-only

// Package e2e drives the fps binary against a real fps-controller with fake
// agents. TestMain builds three binaries into a temporary directory: fps from
// this module, and fps-controller and fps-fakeagent from ../controller. The
// controller module needs its dependencies, so a machine that cannot build it
// skips the tests with a message, and CI, which downloads modules, runs them.
package e2e

import (
	"archive/tar"
	"bufio"
	"bytes"
	"compress/gzip"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"
)

// examplesDir holds the contract examples, relative to this package directory.
const examplesDir = "../../contracts/examples"

const (
	adminToken = "e2e-admin-0123456789"
	runToken   = "e2e-run-0123456789"
	readToken  = "e2e-read-0123456789"
)

// bins holds the paths of the built binaries.
var bins struct{ fps, controller, fakeagent string }

// skipReason is not empty when the end to end tests cannot run.
var skipReason string

func TestMain(m *testing.M) {
	os.Exit(testMain(m))
}

func testMain(m *testing.M) int {
	flag.Parse() // testing.Short needs the flags
	switch {
	case runtime.GOOS == "windows":
		skipReason = "the end to end tests send SIGTERM, which Windows cannot deliver"
	case testing.Short():
		skipReason = "the end to end tests build three binaries, so -short skips them"
	}
	if skipReason != "" {
		return m.Run()
	}
	dir, err := os.MkdirTemp("", "fps-cli-e2e-")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return 1
	}
	defer func() { _ = os.RemoveAll(dir) }()
	bins.fps = filepath.Join(dir, "fps")
	if out, err := build("..", bins.fps, "./cmd/fps"); err != nil {
		fmt.Fprintf(os.Stderr, "build fps: %v\n%s", err, out)
		return 1
	}
	bins.controller = filepath.Join(dir, "fps-controller")
	bins.fakeagent = filepath.Join(dir, "fps-fakeagent")
	if out, err := build("../../controller", bins.controller, "./cmd/fps-controller"); err != nil {
		skipReason = fmt.Sprintf("../controller cannot build, so there is no controller to test against: %v\n%s", err, out)
	} else if out, err := build("../../controller", bins.fakeagent, "./cmd/fps-fakeagent"); err != nil {
		skipReason = fmt.Sprintf("the fake agent helper of ../controller cannot build: %v\n%s", err, out)
	}
	if skipReason != "" {
		fmt.Fprintf(os.Stderr, "SKIP: %s\n", skipReason)
	}
	return m.Run()
}

func build(dir, out, pkg string) ([]byte, error) {
	cmd := exec.Command("go", "build", "-o", out, pkg)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "GOWORK=off")
	return cmd.CombinedOutput()
}

// lockedBuffer is a bytes.Buffer that a process and a test can share.
type lockedBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *lockedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *lockedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

// process is a started helper binary.
type process struct {
	cmd    *exec.Cmd
	stdout *lockedBuffer
	stderr *lockedBuffer
	done   chan error
}

func start(t *testing.T, name string, args ...string) *process {
	t.Helper()
	p := &process{stdout: &lockedBuffer{}, stderr: &lockedBuffer{}, done: make(chan error, 1)}
	p.cmd = exec.Command(name, args...)
	p.cmd.Stdout, p.cmd.Stderr = p.stdout, p.stderr
	if err := p.cmd.Start(); err != nil {
		t.Fatal(err)
	}
	go func() { p.done <- p.cmd.Wait() }()
	t.Cleanup(func() { _ = p.cmd.Process.Kill() })
	return p
}

// stop sends SIGTERM and expects exit code 0.
func (p *process) stop(t *testing.T) {
	t.Helper()
	if err := p.cmd.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-p.done:
		if err != nil {
			t.Errorf("%s exit = %v; stderr:\n%s", filepath.Base(p.cmd.Path), err, p.stderr.String())
		}
	case <-time.After(20 * time.Second):
		t.Errorf("%s did not exit within 20s of SIGTERM", filepath.Base(p.cmd.Path))
	}
}

// waitFor polls until f returns a non empty string or the time ends.
func waitFor(t *testing.T, what string, f func() string) string {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		if s := f(); s != "" {
			return s
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("no %s within 20s", what)
	return ""
}

// logAddr returns the addr of the first log record with msg.
func logAddr(logs *lockedBuffer, msg string) string {
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
	return ""
}

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

// fixture is a running controller with one fake agent.
type fixture struct {
	url       string
	grpcAddr  string
	dir       string
	tokenFile string // holds the run token
	readFile  string
	history   string
	ctl       *process
	agent     *process
}

func newFixture(t *testing.T) *fixture {
	t.Helper()
	if skipReason != "" {
		t.Skip(skipReason)
	}
	f := &fixture{dir: t.TempDir()}
	f.tokenFile = filepath.Join(f.dir, "run.token")
	f.readFile = filepath.Join(f.dir, "read.token")
	f.history = filepath.Join(f.dir, "history.json")
	for file, token := range map[string]string{f.tokenFile: runToken, f.readFile: readToken} {
		if err := os.WriteFile(file, []byte(token+"\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	cfg := filepath.Join(f.dir, "controller.yaml")
	conf := fmt.Sprintf(`listen: 127.0.0.1:0
grpc_listen: 127.0.0.1:0
insecure_plain_grpc: true
log_level: debug
shutdown_timeout: 10s
data_dir: %s
tokens:
  - { name: admin, scope: admin, token: %s }
  - { name: ci, scope: run, token_file: %s }
  - { name: viewer, scope: read, token: %s }
`, filepath.Join(f.dir, "data"), adminToken, f.tokenFile, readToken)
	if err := os.WriteFile(cfg, []byte(conf), 0o600); err != nil {
		t.Fatal(err)
	}
	f.ctl = start(t, bins.controller, "-config", cfg)
	f.url = "http://" + waitFor(t, "controller listening record", func() string { return logAddr(f.ctl.stderr, "controller listening") })
	f.grpcAddr = waitFor(t, "agent service listening record", func() string { return logAddr(f.ctl.stderr, "agent service listening") })
	t.Cleanup(func() {
		if t.Failed() {
			t.Logf("controller logs:\n%s", f.ctl.stderr.String())
			if f.agent != nil {
				t.Logf("fake agent stderr:\n%s", f.agent.stderr.String())
			}
		}
	})

	// Setup that the CLI does not offer goes straight to the REST API: enroll
	// the agent and install the module that the configuration names.
	f.rest(t, http.MethodPost, "/v1/agents", "application/json", []byte(`{"id":"local"}`), http.StatusCreated)
	f.rest(t, http.MethodPost, "/v1/modules", "application/gzip", moduleArchive(t, filepath.Join(examplesDir, "modules", "http1")), http.StatusCreated)

	f.agent = start(t, bins.fakeagent, "-controller", f.grpcAddr, "-agents", "local", "-ports", "lo", "-finish-after", "3s")
	waitFor(t, `"fake agents ready" line`, func() string {
		if strings.Contains(f.agent.stdout.String(), "fake agents ready") {
			return "ok"
		}
		select {
		case err := <-f.agent.done:
			t.Fatalf("fake agent exited early: %v\n%s", err, f.agent.stderr.String())
		default:
		}
		return ""
	})
	return f
}

func (f *fixture) rest(t *testing.T, method, path, ctype string, body []byte, want int) {
	t.Helper()
	req, err := http.NewRequestWithContext(context.Background(), method, f.url+path, bytes.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+adminToken)
	req.Header.Set("Content-Type", ctype)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	data, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != want {
		t.Fatalf("%s %s: status %d, want %d: %s", method, path, resp.StatusCode, want, data)
	}
}

// out is the result of one fps invocation.
type out struct {
	code           int
	stdout, stderr string
}

// fps runs the binary with the run token file and the history of the fixture.
func (f *fixture) fps(t *testing.T, stdin string, args ...string) out {
	t.Helper()
	return f.fpsEnv(t, stdin, map[string]string{"FPS_TOKEN_FILE": f.tokenFile}, args...)
}

func (f *fixture) fpsEnv(t *testing.T, stdin string, env map[string]string, args ...string) out {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, bins.fps, args...)
	cmd.Env = []string{"FPS_URL=" + f.url, "FPS_HISTORY_FILE=" + f.history, "PATH=" + os.Getenv("PATH")}
	for k, v := range env {
		cmd.Env = append(cmd.Env, k+"="+v)
	}
	cmd.Stdin = strings.NewReader(stdin)
	var so, se bytes.Buffer
	cmd.Stdout, cmd.Stderr = &so, &se
	err := cmd.Run()
	code := 0
	if err != nil {
		var ee *exec.ExitError
		if !errors.As(err, &ee) {
			t.Fatalf("fps %v: %v", args, err)
		}
		code = ee.ExitCode()
	}
	return out{code: code, stdout: so.String(), stderr: se.String()}
}

func (o out) expect(t *testing.T, what string, code int) out {
	t.Helper()
	if o.code != code {
		t.Fatalf("%s: exit %d, want %d\nstdout:\n%s\nstderr:\n%s", what, o.code, code, o.stdout, o.stderr)
	}
	return o
}

// smokeConfig is the example configuration without its criteria, so that the
// verdict depends only on what the test appends.
func smokeConfig(t *testing.T, criteria string) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(examplesDir, "tests", "http_smoke.yaml")) //nolint:gosec // G304: a fixed path of the repository
	if err != nil {
		t.Fatal(err)
	}
	doc := string(data)
	i := strings.Index(doc, "\ncriteria:")
	if i < 0 {
		t.Fatal("http_smoke.yaml has no criteria block to replace")
	}
	return doc[:i+1] + criteria
}

// decode decodes JSON text into a map.
func decode(t *testing.T, text string) map[string]any {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal([]byte(text), &m); err != nil {
		t.Fatalf("not JSON: %v\n%s", err, text)
	}
	return m
}

func TestEndToEnd(t *testing.T) {
	f := newFixture(t)

	// Validate, upload, list, and show a configuration.
	passing := smokeConfig(t, "")
	file := filepath.Join(f.dir, "smoke.yaml")
	if err := os.WriteFile(file, []byte(passing), 0o600); err != nil {
		t.Fatal(err)
	}
	v := f.fps(t, "", "config", "validate", file).expect(t, "config validate", 0)
	if !strings.Contains(v.stdout, "is valid") {
		t.Errorf("validate stdout = %q", v.stdout)
	}
	up := f.fps(t, "", "config", "upload", file).expect(t, "config upload", 0)
	configID := strings.TrimSpace(up.stdout)
	if configID == "" {
		t.Fatalf("config upload printed no identifier: %q", up.stdout)
	}
	if l := f.fps(t, "", "config", "list").expect(t, "config list", 0); !strings.Contains(l.stdout, configID) {
		t.Errorf("config list lacks %s:\n%s", configID, l.stdout)
	}
	if s := f.fps(t, "", "config", "show", configID).expect(t, "config show", 0); !strings.Contains(s.stdout, "http_smoke") {
		t.Errorf("config show lacks the name:\n%s", s.stdout)
	}
	// A document from standard input works, and a broken one is invalid.
	f.fps(t, passing, "config", "validate", "-").expect(t, "config validate -", 0)
	bad := f.fps(t, "kind: Nothing\n", "config", "validate", "-").expect(t, "config validate of a broken document", 1)
	if !strings.Contains(bad.stderr, "invalid") {
		t.Errorf("broken document stderr = %q", bad.stderr)
	}

	// Start a run, watch it to its end, and read its report.
	runID := strings.TrimSpace(f.fps(t, "", "run", "start", configID, "--agent", "local").expect(t, "run start", 0).stdout)
	if runID == "" {
		t.Fatal("run start printed no identifier")
	}
	w := f.fps(t, "", "watch", runID, "--view", "scenario", "--view", "port").expect(t, "watch", 0)
	if !strings.Contains(w.stdout, "sessions.attempted=") {
		t.Errorf("watch printed no live metrics:\n%s", w.stdout)
	}
	if !strings.Contains(w.stdout, "completed") {
		t.Errorf("watch did not print the final state:\n%s", w.stdout)
	}
	show := decode(t, f.fps(t, "", "--json", "run", "show", runID).expect(t, "run show", 0).stdout)
	if show["state"] != "completed" || show["id"] != runID {
		t.Errorf("run show = %v", show)
	}
	if l := f.fps(t, "", "run", "list"); l.code == 0 {
		if !strings.Contains(l.stdout, runID) {
			t.Errorf("run list lacks %s:\n%s", runID, l.stdout)
		}
	} else if l.code != 5 { // 5 means the controller has no listRuns yet.
		t.Errorf("run list: exit %d, stderr %q", l.code, l.stderr)
	}

	text := f.fps(t, "", "report", runID).expect(t, "report", 0)
	if !strings.Contains(text.stdout, runID) || !strings.Contains(text.stdout, "completed") {
		t.Errorf("text report:\n%s", text.stdout)
	}
	rep := decode(t, f.fps(t, "", "report", runID, "--format", "json").expect(t, "report json", 0).stdout)
	if run, _ := rep["run"].(map[string]any); run["id"] != runID {
		t.Errorf("json report = %v", rep)
	}
	csv := f.fps(t, "", "report", runID, "--format", "csv").expect(t, "report csv", 0)
	if !strings.Contains(csv.stdout, ",") || !strings.Contains(csv.stdout, "\n") {
		t.Errorf("csv report:\n%s", csv.stdout)
	}
	htmlFile := filepath.Join(f.dir, "report.html")
	f.fps(t, "", "report", runID, "--format", "html", "-o", htmlFile).expect(t, "report html", 0)
	if data, err := os.ReadFile(htmlFile); err != nil || !strings.Contains(strings.ToLower(string(data)), "<html") { //nolint:gosec // G304: a temporary file of the test
		t.Errorf("html report: %v\n%.200s", err, data)
	}

	// A run whose criterion fails exits 1 and names the criterion.
	failing := filepath.Join(f.dir, "failing.yaml")
	if err := os.WriteFile(failing, []byte(smokeConfig(t, "criteria:\n  - { metric: sessions.attempted, op: \"<\", value: 0 }\n")), 0o600); err != nil {
		t.Fatal(err)
	}
	failID := strings.TrimSpace(f.fps(t, "", "config", "upload", failing).expect(t, "upload failing config", 0).stdout)
	res := f.fps(t, "", "run", "start", failID, "--agent", "local", "--watch").expect(t, "run start --watch with a failing criterion", 1)
	if !strings.Contains(res.stdout, "sessions.attempted < 0") || !strings.Contains(res.stderr, "missed its criteria") {
		t.Errorf("failing run:\nstdout:\n%s\nstderr:\n%s", res.stdout, res.stderr)
	}

	// Stop a run before the fake agent ends it, then follow it in JSON lines.
	stopID := strings.TrimSpace(f.fps(t, "", "run", "start", configID, "--agent", "local").expect(t, "run start for a stop", 0).stdout)
	waitFor(t, "run "+stopID+" to run", func() string {
		o := f.fps(t, "", "--json", "run", "show", stopID).expect(t, "run show", 0)
		if decode(t, o.stdout)["state"] == "running" {
			return "running"
		}
		return ""
	})
	f.fps(t, "", "run", "stop", stopID).expect(t, "run stop", 0)
	jw := f.fps(t, "", "--json", "watch", stopID, "--poll", "--interval", "200ms").expect(t, "watch --json after a stop", 0)
	lines := strings.Split(strings.TrimSpace(jw.stdout), "\n")
	if last := decode(t, lines[len(lines)-1]); last["event"] != "end" {
		t.Errorf("last JSON line = %v, want the end event", last)
	}
	if st := decode(t, f.fps(t, "", "--json", "run", "show", stopID).expect(t, "run show", 0).stdout)["state"]; st != "completed" {
		t.Errorf("state after the stop = %v, want completed", st)
	}

	// Errors map to exit codes, and no output shows a token.
	f.fps(t, "", "run", "show", "no_such_run").expect(t, "unknown run", 4)
	f.fps(t, "", "report", "no_such_run").expect(t, "report of an unknown run", 4)
	f.fpsEnv(t, "", map[string]string{"FPS_TOKEN": "wrong-token-0123456789"}, "run", "show", runID).expect(t, "wrong token", 3)
	f.fpsEnv(t, "", map[string]string{}, "run", "show", runID).expect(t, "no token", 3)
	ro := f.fpsEnv(t, "", map[string]string{"FPS_TOKEN_FILE": f.readFile}, "config", "upload", file).expect(t, "upload with a read token", 3)
	if !strings.Contains(ro.stderr, "scope") {
		t.Errorf("a 403 names no scope: %q", ro.stderr)
	}
	f.fpsEnv(t, "", map[string]string{"FPS_TOKEN_FILE": f.readFile}, "run", "show", runID).expect(t, "show with a read token", 0)
	f.fps(t, "", "--url", "http://127.0.0.1:1", "run", "show", runID).expect(t, "unreachable controller", 6)
	for _, o := range []out{ro, bad} {
		for _, tok := range []string{adminToken, runToken, readToken} {
			if strings.Contains(o.stdout+o.stderr, tok) {
				t.Errorf("a token appears in the output:\n%s%s", o.stdout, o.stderr)
			}
		}
	}

	f.agent.stop(t)
	f.ctl.stop(t)
}
