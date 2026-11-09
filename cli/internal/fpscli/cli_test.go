// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"bytes"
	"context"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

const testToken = "unit-test-token-0123456789"

// result is the outcome of one command.
type result struct {
	code   int
	stdout string
	stderr string
}

// cli runs the command with a private environment that holds the token, the
// URL, and a history file in a temporary directory.
func cli(t *testing.T, url string, stdin string, args ...string) result {
	t.Helper()
	return cliEnv(t, map[string]string{"FPS_URL": url, "FPS_TOKEN": testToken}, stdin, args...)
}

func cliEnv(t *testing.T, env map[string]string, stdin string, args ...string) result {
	t.Helper()
	if env["FPS_HISTORY_FILE"] == "" {
		env["FPS_HISTORY_FILE"] = filepath.Join(t.TempDir(), "history.json")
	}
	var out, errb bytes.Buffer
	code := Execute(context.Background(), args, Env{
		Stdin:  strings.NewReader(stdin),
		Stdout: &out,
		Stderr: &errb,
		Getenv: func(k string) string { return env[k] },
	})
	return result{code: code, stdout: out.String(), stderr: errb.String()}
}

func problem(w http.ResponseWriter, status int, detail string, findings ...Finding) {
	w.Header().Set("Content-Type", "application/problem+json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"type": "about:blank", "title": http.StatusText(status), "status": status, "detail": detail, "errors": findings,
	})
}

func writeJSON(w http.ResponseWriter, status int, body string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_, _ = io.WriteString(w, body) //nolint:gosec // G705: a JSON test body from the test itself, served as application/json
}

// stub is a recording API server.
type stub struct {
	*httptest.Server
	mu   sync.Mutex
	reqs []recorded
}

type recorded struct {
	method, path, query, ctype, auth, body string
}

func newStub(t *testing.T, h http.HandlerFunc) *stub {
	t.Helper()
	s := &stub{}
	s.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		s.mu.Lock()
		s.reqs = append(s.reqs, recorded{r.Method, r.URL.Path, r.URL.RawQuery, r.Header.Get("Content-Type"), r.Header.Get("Authorization"), string(body)})
		s.mu.Unlock()
		h(w, r)
	}))
	t.Cleanup(s.Close)
	return s
}

func (s *stub) last() recorded {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.reqs[len(s.reqs)-1]
}

func TestTokenIsSentAndNeverPrinted(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer "+testToken {
			problem(w, http.StatusUnauthorized, "bad token")
			return
		}
		writeJSON(w, 200, `{"id":"r1","config_id":"c1","state":"running"}`)
	})
	res := cli(t, srv.URL, "", "run", "show", "r1")
	if res.code != ExitOK {
		t.Fatalf("exit %d, stderr %s", res.code, res.stderr)
	}
	if got := srv.last().auth; got != "Bearer "+testToken {
		t.Errorf("Authorization = %q", got)
	}
	if strings.Contains(res.stdout+res.stderr, testToken) {
		t.Errorf("the token appears in the output")
	}

	// A server that echoes the token in its error must not make it appear.
	echo := newStub(t, func(w http.ResponseWriter, _ *http.Request) {
		problem(w, http.StatusForbidden, "token "+testToken+" lacks scope")
	})
	res = cli(t, echo.URL, "", "run", "show", "r1")
	if res.code != ExitAuth || strings.Contains(res.stderr, testToken) || !strings.Contains(res.stderr, "[redacted]") {
		t.Errorf("exit %d, stderr %q", res.code, res.stderr)
	}
}

func TestTokenSourcesAndPrecedence(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, 200, `{"id":"r1","config_id":"c1","state":"running"}`)
	})
	dir := t.TempDir()
	fileA := filepath.Join(dir, "a.token")
	fileB := filepath.Join(dir, "b.token")
	if err := os.WriteFile(fileA, []byte("  file-token-a-0123456789\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(fileB, []byte("file-token-b-0123456789\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	env := map[string]string{"FPS_URL": srv.URL, "FPS_TOKEN": "env-token", "FPS_TOKEN_FILE": fileA}
	if res := cliEnv(t, env, "", "run", "show", "r1"); res.code != 0 || srv.last().auth != "Bearer file-token-a-0123456789" {
		t.Errorf("FPS_TOKEN_FILE: exit %d, auth %q", res.code, srv.last().auth)
	}
	// The flag wins over the environment, before or after the command.
	if res := cliEnv(t, env, "", "--token-file", fileB, "run", "show", "r1"); res.code != 0 || srv.last().auth != "Bearer file-token-b-0123456789" {
		t.Errorf("--token-file before: exit %d, auth %q", res.code, srv.last().auth)
	}
	if res := cliEnv(t, env, "", "run", "show", "r1", "--token-file", fileB); res.code != 0 || srv.last().auth != "Bearer file-token-b-0123456789" {
		t.Errorf("--token-file after: exit %d, auth %q", res.code, srv.last().auth)
	}
	env = map[string]string{"FPS_URL": srv.URL, "FPS_TOKEN": "env-token"}
	if res := cliEnv(t, env, "", "run", "show", "r1"); res.code != 0 || srv.last().auth != "Bearer env-token" {
		t.Errorf("FPS_TOKEN: exit %d, auth %q", res.code, srv.last().auth)
	}
	// No token sends no header, and a 401 explains it.
	none := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "" {
			t.Errorf("an Authorization header was sent")
		}
		problem(w, http.StatusUnauthorized, "no token")
	})
	res := cliEnv(t, map[string]string{"FPS_URL": none.URL}, "", "run", "show", "r1")
	if res.code != ExitAuth || !strings.Contains(res.stderr, "no API token was sent") {
		t.Errorf("exit %d, stderr %q", res.code, res.stderr)
	}
	// A missing or empty token file is a usage error.
	if res := cliEnv(t, map[string]string{"FPS_URL": srv.URL}, "", "--token-file", filepath.Join(dir, "none"), "run", "show", "r1"); res.code != ExitUsage {
		t.Errorf("missing file: exit %d", res.code)
	}
}

func TestPlainHTTPToRemoteHostIsRefused(t *testing.T) {
	res := cliEnv(t, map[string]string{"FPS_URL": "http://controller.example.invalid:8443", "FPS_TOKEN": testToken}, "", "run", "show", "r1")
	if res.code != ExitUsage || !strings.Contains(res.stderr, "--insecure-http") || strings.Contains(res.stderr, testToken) {
		t.Errorf("exit %d, stderr %q", res.code, res.stderr)
	}
}

func TestExitCodes(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/runs/missing":
			problem(w, 404, "no such run")
		case "/v1/runs/boom":
			problem(w, 500, "internal")
		case "/v1/runs/redirect":
			http.Redirect(w, r, "http://elsewhere.invalid/", http.StatusFound)
		default:
			writeJSON(w, 200, `{"id":"ok","config_id":"c","state":"running"}`)
		}
	})
	for _, tc := range []struct {
		args []string
		want int
	}{
		{[]string{"run", "show", "missing"}, ExitNotFound},
		{[]string{"run", "show", "boom"}, ExitAPI},
		{[]string{"run", "show", "redirect"}, ExitAPI},
		{[]string{"run", "show", "bad id"}, ExitUsage},
		{[]string{"run", "show"}, ExitUsage},
		{[]string{"run", "bogus"}, ExitUsage},
		{[]string{"nope"}, ExitUsage},
		{nil, ExitUsage},
		{[]string{"--bogus", "run"}, ExitUsage},
		{[]string{"run", "list", "--state", "weird"}, ExitUsage},
		{[]string{"report", "r1", "--format", "pdf"}, ExitUsage},
		{[]string{"help"}, ExitOK},
		{[]string{"run", "show", "-h"}, ExitOK},
	} {
		if res := cli(t, srv.URL, "", tc.args...); res.code != tc.want {
			t.Errorf("%v: exit %d, want %d; stderr %q", tc.args, res.code, tc.want, res.stderr)
		}
	}
	dead := httptest.NewServer(nil)
	url := dead.URL
	dead.Close()
	if res := cli(t, url, "", "run", "show", "r1"); res.code != ExitNetwork {
		t.Errorf("unreachable: exit %d, stderr %q", res.code, res.stderr)
	}
	if res := cli(t, srv.URL, "", "--timeout", "1ms", "run", "show", "r1"); res.code != ExitOK && res.code != ExitNetwork {
		t.Errorf("tiny timeout: exit %d", res.code)
	}
}

func TestTLSNeedsTheCAFile(t *testing.T) {
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, 200, `{"id":"r1","config_id":"c1","state":"completed"}`)
	}))
	defer srv.Close()
	ca := filepath.Join(t.TempDir(), "ca.pem")
	pemData := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: srv.Certificate().Raw})
	if err := os.WriteFile(ca, pemData, 0o600); err != nil {
		t.Fatal(err)
	}
	res := cli(t, srv.URL, "", "run", "show", "r1")
	if res.code != ExitNetwork || !strings.Contains(res.stderr, "--ca-file") {
		t.Errorf("without CA: exit %d, stderr %q", res.code, res.stderr)
	}
	for _, args := range [][]string{{"--ca-file", ca, "run", "show", "r1"}, {"run", "show", "r1", "--ca-file", ca}} {
		if res := cli(t, srv.URL, "", args...); res.code != ExitOK {
			t.Errorf("%v: exit %d, stderr %q", args, res.code, res.stderr)
		}
	}
	if res := cliEnv(t, map[string]string{"FPS_URL": srv.URL, "FPS_CA_FILE": ca}, "", "run", "show", "r1"); res.code != ExitOK {
		t.Errorf("FPS_CA_FILE: exit %d, stderr %q", res.code, res.stderr)
	}
	if res := cli(t, srv.URL, "", "--ca-file", filepath.Join(t.TempDir(), "none.pem"), "run", "show", "r1"); res.code != ExitUsage {
		t.Errorf("missing CA file: exit %d", res.code)
	}
}

func TestConfigUploadListShow(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/configs":
			writeJSON(w, 201, `{"id":"cfg_1","name":"http_smoke","created_at":"2026-11-09T10:00:00Z"}`)
		case r.Method == http.MethodGet && r.URL.Path == "/v1/configs/cfg_1":
			writeJSON(w, 200, `{"kind":"Test","metadata":{"name":"http_smoke"}}`)
		default:
			problem(w, 404, "no")
		}
	})
	hist := filepath.Join(t.TempDir(), "state", "h.json")
	env := map[string]string{"FPS_URL": srv.URL, "FPS_TOKEN": testToken, "FPS_HISTORY_FILE": hist}
	dir := t.TempDir()
	yml := filepath.Join(dir, "smoke.yaml")
	if err := os.WriteFile(yml, []byte("kind: Test\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	res := cliEnv(t, env, "", "config", "upload", yml)
	if res.code != 0 || strings.TrimSpace(res.stdout) != "cfg_1" {
		t.Fatalf("upload: exit %d, stdout %q, stderr %q", res.code, res.stdout, res.stderr)
	}
	if r := srv.last(); r.ctype != "application/yaml" || r.body != "kind: Test\n" {
		t.Errorf("upload request = %+v", r)
	}
	js := filepath.Join(dir, "smoke.json")
	if err := os.WriteFile(js, []byte(`{"kind":"Test"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if res := cliEnv(t, env, "", "config", "upload", js); res.code != 0 || srv.last().ctype != "application/json" {
		t.Errorf("json upload: exit %d, ctype %q", res.code, srv.last().ctype)
	}
	// Standard input is sniffed by its first character, and --format overrides.
	if res := cliEnv(t, env, `{"kind":"Test"}`, "config", "upload", "-"); res.code != 0 || srv.last().ctype != "application/json" {
		t.Errorf("stdin json: exit %d, ctype %q", res.code, srv.last().ctype)
	}
	if res := cliEnv(t, env, "kind: Test", "config", "upload", "-"); res.code != 0 || srv.last().ctype != "application/yaml" {
		t.Errorf("stdin yaml: exit %d, ctype %q", res.code, srv.last().ctype)
	}
	if res := cliEnv(t, env, "{}", "config", "upload", "-", "--format", "yaml"); res.code != 0 || srv.last().ctype != "application/yaml" {
		t.Errorf("--format: exit %d, ctype %q", res.code, srv.last().ctype)
	}
	if res := cliEnv(t, env, "", "config", "upload", "-"); res.code != ExitUsage {
		t.Errorf("empty stdin: exit %d", res.code)
	}
	if res := cliEnv(t, env, "", "--json", "config", "upload", yml); res.code != 0 || !strings.Contains(res.stdout, `"id": "cfg_1"`) {
		t.Errorf("--json upload: exit %d, stdout %q", res.code, res.stdout)
	}

	// The history holds one entry per configuration of the controller.
	res = cliEnv(t, env, "", "config", "list")
	if res.code != 0 || !strings.Contains(res.stdout, "cfg_1") || !strings.Contains(res.stdout, "http_smoke") || strings.Count(res.stdout, "cfg_1") != 1 {
		t.Errorf("list: exit %d, stdout %q", res.code, res.stdout)
	}
	if st, err := os.Stat(hist); err != nil || st.Mode().Perm() != 0o600 {
		t.Errorf("history file: %v %v", st, err)
	}
	res = cliEnv(t, env, "", "--json", "config", "list")
	var listed struct{ Items []historyEntry }
	if err := json.Unmarshal([]byte(res.stdout), &listed); err != nil || len(listed.Items) != 1 {
		t.Errorf("--json list: %q %v", res.stdout, err)
	}
	other := map[string]string{"FPS_URL": "http://localhost:1", "FPS_HISTORY_FILE": hist}
	if res := cliEnv(t, other, "", "config", "list"); res.code != 0 || strings.Contains(res.stdout, "cfg_1") {
		t.Errorf("list of another controller: %q", res.stdout)
	}
	if res := cliEnv(t, other, "", "config", "list", "--all-controllers"); !strings.Contains(res.stdout, "cfg_1") {
		t.Errorf("list --all-controllers: %q", res.stdout)
	}

	res = cliEnv(t, env, "", "config", "show", "cfg_1")
	if res.code != 0 || !strings.Contains(res.stdout, `"kind": "Test"`) {
		t.Errorf("show: exit %d, stdout %q", res.code, res.stdout)
	}
	if res := cliEnv(t, env, "", "config", "show", "nope"); res.code != ExitNotFound {
		t.Errorf("show unknown: exit %d", res.code)
	}
}

func TestConfigValidate(t *testing.T) {
	var invalidUpload bool
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/configs":
			if invalidUpload {
				problem(w, 422, "schema", Finding{"error", "/load", "load is required"})
				return
			}
			writeJSON(w, 201, `{"id":"cfg_2","name":"n"}`)
		case "/v1/configs/cfg_2:validate":
			writeJSON(w, 200, `{"valid":true,"findings":[{"severity":"warning","path":"/x","message":"odd"}],"capacity":{"agents_required":1,"agents_available":2}}`)
		case "/v1/configs/cfg_bad:validate":
			writeJSON(w, 200, `{"valid":false,"findings":[{"severity":"error","path":"/profile","message":"unknown scenario"}]}`)
		default:
			problem(w, 404, "no")
		}
	})
	res := cli(t, srv.URL, "kind: Test\n", "config", "validate", "-")
	if res.code != 0 || !strings.Contains(res.stdout, "is valid") || !strings.Contains(res.stdout, "1 required, 2 available") || !strings.Contains(res.stdout, "odd") {
		t.Errorf("valid: exit %d, stdout %q, stderr %q", res.code, res.stdout, res.stderr)
	}
	res = cli(t, srv.URL, "", "config", "validate", "--id", "cfg_bad")
	if res.code != ExitFailed || !strings.Contains(res.stdout, "unknown scenario") {
		t.Errorf("invalid report: exit %d, stdout %q", res.code, res.stdout)
	}
	invalidUpload = true
	res = cli(t, srv.URL, "kind: Test\n", "config", "validate", "-")
	if res.code != ExitFailed || !strings.Contains(res.stderr, "load is required") {
		t.Errorf("invalid upload: exit %d, stderr %q", res.code, res.stderr)
	}
	if res := cli(t, srv.URL, "", "config", "validate"); res.code != ExitUsage {
		t.Errorf("no argument: exit %d", res.code)
	}
	if res := cli(t, srv.URL, "", "config", "validate", "--id", "cfg_2", "x.yaml"); res.code != ExitUsage {
		t.Errorf("id and file: exit %d", res.code)
	}
	if res := cli(t, srv.URL, "x", "config", "upload", "-"); res.code != ExitAPI || !strings.Contains(res.stderr, "load is required") {
		t.Errorf("upload 422: exit %d, stderr %q", res.code, res.stderr)
	}
}

func TestRunStartStopShowList(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/runs":
			writeJSON(w, 202, `{"id":"run_9","config_id":"cfg_1","state":"pending"}`)
		case r.URL.Path == "/v1/runs/run_9:stop":
			writeJSON(w, 202, `{"id":"run_9","config_id":"cfg_1","state":"stopping"}`)
		case r.URL.Path == "/v1/runs/run_9":
			writeJSON(w, 200, `{"id":"run_9","config_id":"cfg_1","state":"failed","reason":"agent a1 failed","started_at":"2026-11-09T10:00:00Z",
"summary":{"sessions_attempted":120,"throughput_bps":1234.56789},
"verdict":{"passed":false,"criteria":[{"metric":"sessions.failed_ratio","op":"<","value":0.01,"observed":0.042,"passed":false,"reason":"observed 0.042 is not < 0.01"},{"metric":"x.y","op":">","value":1,"reason":"metric x.y has no value"}]}}`)
		case r.URL.Path == "/v1/runs" && r.URL.Query().Get("page_token") == "":
			writeJSON(w, 200, `{"items":[{"id":"r2","config_id":"c","state":"running"}],"next_page_token":"tok2"}`)
		case r.URL.Path == "/v1/runs":
			writeJSON(w, 200, `{"items":[{"id":"r1","config_id":"c","state":"completed","verdict":{"passed":true}}]}`)
		default:
			problem(w, 404, "no")
		}
	})
	res := cli(t, srv.URL, "", "run", "start", "cfg_1", "--agent", "a1", "--agent", "a2", "--label", "site=lab", "--label", "k=v=w")
	if res.code != 0 || strings.TrimSpace(res.stdout) != "run_9" {
		t.Fatalf("start: exit %d, stdout %q, stderr %q", res.code, res.stdout, res.stderr)
	}
	var body struct {
		ConfigID string            `json:"config_id"`
		Agents   []string          `json:"agents"`
		Labels   map[string]string `json:"labels"`
	}
	if err := json.Unmarshal([]byte(srv.last().body), &body); err != nil {
		t.Fatal(err)
	}
	if body.ConfigID != "cfg_1" || strings.Join(body.Agents, ",") != "a1,a2" || body.Labels["site"] != "lab" || body.Labels["k"] != "v=w" {
		t.Errorf("start body = %s", srv.last().body)
	}
	if res := cli(t, srv.URL, "", "run", "start", "cfg_1", "--label", "novalue"); res.code != ExitUsage {
		t.Errorf("bad label: exit %d", res.code)
	}

	res = cli(t, srv.URL, "", "run", "stop", "run_9")
	if res.code != 0 || !strings.Contains(res.stdout, "stopping") || srv.last().body != "{}" {
		t.Errorf("stop: exit %d, stdout %q, body %q", res.code, res.stdout, srv.last().body)
	}
	if res := cli(t, srv.URL, "", "run", "stop", "--force", "run_9"); res.code != 0 || srv.last().body != `{"force":true}` {
		t.Errorf("stop --force: exit %d, body %q", res.code, srv.last().body)
	}

	res = cli(t, srv.URL, "", "run", "show", "run_9")
	for _, want := range []string{"failed", "agent a1 failed", "sessions_attempted  120", "1235", "sessions.failed_ratio < 0.01", "observed 0.042 is not < 0.01", "no result", "x.y > 1"} {
		if !strings.Contains(res.stdout, want) {
			t.Errorf("show lacks %q:\n%s", want, res.stdout)
		}
	}
	res = cli(t, srv.URL, "", "--json", "run", "show", "run_9")
	var raw map[string]any
	if err := json.Unmarshal([]byte(res.stdout), &raw); err != nil || raw["id"] != "run_9" {
		t.Errorf("--json show: %q %v", res.stdout, err)
	}

	res = cli(t, srv.URL, "", "run", "list")
	if res.code != 0 || !strings.Contains(res.stdout, "r2") || strings.Contains(res.stdout, "r1") || !strings.Contains(res.stderr, "tok2") {
		t.Errorf("list: exit %d, stdout %q, stderr %q", res.code, res.stdout, res.stderr)
	}
	res = cli(t, srv.URL, "", "run", "list", "--all", "--state", "running", "--page-size", "1")
	if res.code != 0 || !strings.Contains(res.stdout, "r2") || !strings.Contains(res.stdout, "r1") || !strings.Contains(res.stdout, "passed") {
		t.Errorf("list --all: exit %d, stdout %q", res.code, res.stdout)
	}
	res = cli(t, srv.URL, "", "--json", "run", "list", "--all")
	var listed struct{ Items []map[string]any }
	if err := json.Unmarshal([]byte(res.stdout), &listed); err != nil || len(listed.Items) != 2 {
		t.Errorf("--json list: %q %v", res.stdout, err)
	}
}

func TestRunListNotImplemented(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, _ *http.Request) { problem(w, 501, "listRuns is not implemented") })
	res := cli(t, srv.URL, "", "run", "list")
	if res.code != ExitAPI || !strings.Contains(res.stderr, "does not implement") {
		t.Errorf("exit %d, stderr %q", res.code, res.stderr)
	}
}

// sse writes events and flushes them.
func sse(w http.ResponseWriter, parts ...string) {
	w.Header().Set("Content-Type", "text/event-stream")
	w.WriteHeader(200)
	for _, p := range parts {
		_, _ = io.WriteString(w, p)
		_ = http.NewResponseController(w).Flush()
		time.Sleep(5 * time.Millisecond)
	}
}

func sample(id, ts, key string, attempted int) string {
	return fmt.Sprintf("id: %s\ndata: {\"ts\":%q,\"view\":\"scenario\",\"key\":%q,\"values\":{\"sessions.attempted\":%d,\"sessions.rate\":%d.5,\"tcp.rtt.p99\":0.001}}\n\n", id, ts, key, attempted, attempted)
}

func runBody(state string, passed string) string {
	v := ""
	if passed != "" {
		v = `,"verdict":{"passed":` + passed + `}`
	}
	return `{"id":"r1","config_id":"c1","state":"` + state + `"` + v + `,"summary":{"sessions_attempted":9}}`
}

func TestWatchStream(t *testing.T) {
	var gets int
	var mu sync.Mutex
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/runs/r1":
			mu.Lock()
			gets++
			n := gets
			mu.Unlock()
			if n == 1 {
				writeJSON(w, 200, runBody("running", ""))
			} else {
				writeJSON(w, 200, runBody("completed", "true"))
			}
		case "/v1/runs/r1/metrics:stream":
			sse(w, "retry: 1000\n\n",
				sample("1", "2026-11-09T10:00:01Z", "web_get", 3),
				sample("1", "2026-11-09T10:00:01Z", "web_post", 1),
				": keep alive\n\n",
				sample("2", "2026-11-09T10:00:02Z", "web_get", 7),
				"event: end\ndata: {\"state\":\"completed\"}\n\n")
		default:
			problem(w, 404, "no")
		}
	})
	res := cli(t, srv.URL, "", "watch", "r1", "--view", "scenario", "--view", "tcp", "--side", "server")
	if res.code != ExitOK {
		t.Fatalf("exit %d, stderr %q", res.code, res.stderr)
	}
	for _, r := range srv.reqs {
		if strings.HasSuffix(r.path, ":stream") {
			if r.query != "side=server&view=scenario&view=tcp" || !strings.HasPrefix(r.auth, "Bearer ") {
				t.Errorf("stream request = %+v", r)
			}
		}
	}
	for _, want := range []string{
		"2026-11-09T10:00:01Z scenario web_get sessions.attempted=3 sessions.rate=3.5",
		"2026-11-09T10:00:01Z scenario web_post sessions.attempted=1",
		"2026-11-09T10:00:02Z scenario web_get sessions.attempted=7",
		"completed", "Verdict:", "passed",
	} {
		if !strings.Contains(res.stdout, want) {
			t.Errorf("output lacks %q:\n%s", want, res.stdout)
		}
	}
	if strings.Contains(res.stdout, "tcp.rtt") {
		t.Errorf("a non headline metric appears:\n%s", res.stdout)
	}
}

func TestWatchMetricSelectionAndJSON(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/runs/r1":
			writeJSON(w, 200, runBody("completed", "true"))
		case "/v1/runs/r1/metrics:stream":
			sse(w, sample("1", "2026-11-09T10:00:01Z", "web_get", 3), "event: end\ndata: {\"state\":\"completed\"}\n\n")
		}
	})
	// A finished run prints its end without a stream.
	res := cli(t, srv.URL, "", "watch", "r1", "--metric", "tcp.rtt")
	if res.code != 0 || strings.Contains(res.stdout, "sessions.attempted") {
		t.Errorf("finished run: exit %d, stdout %q", res.code, res.stdout)
	}
	running := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/runs/r1":
			writeJSON(w, 200, runBody("running", ""))
		case "/v1/runs/r1/metrics:stream":
			sse(w, sample("1", "2026-11-09T10:00:01Z", "web_get", 3), "event: end\ndata: {\"state\":\"completed\"}\n\n")
		}
	})
	// The run stays running for the reads, so only the stream decides; the
	// final read returns running and the outcome is a success.
	res = cli(t, running.URL, "", "watch", "r1", "--metric", "tcp.rtt")
	if res.code != 0 || !strings.Contains(res.stdout, "tcp.rtt.p99=0.001") || strings.Contains(res.stdout, "sessions.attempted") {
		t.Errorf("--metric: exit %d, stdout %q", res.code, res.stdout)
	}
	res = cli(t, running.URL, "", "watch", "r1", "--all-metrics")
	if !strings.Contains(res.stdout, "sessions.attempted=3") || !strings.Contains(res.stdout, "tcp.rtt.p99=0.001") {
		t.Errorf("--all-metrics: %q", res.stdout)
	}
	res = cli(t, running.URL, "", "--json", "watch", "r1")
	lines := strings.Split(strings.TrimSpace(res.stdout), "\n")
	if res.code != 0 || len(lines) != 2 {
		t.Fatalf("--json: exit %d, stdout %q", res.code, res.stdout)
	}
	var ev struct {
		Event  string
		Sample MetricSample
		Run    Run
	}
	if err := json.Unmarshal([]byte(lines[0]), &ev); err != nil || ev.Event != "sample" || ev.Sample.Key != "web_get" || ev.Sample.Values["sessions.attempted"] != 3 {
		t.Errorf("sample line %q: %v", lines[0], err)
	}
	ev = struct {
		Event  string
		Sample MetricSample
		Run    Run
	}{}
	if err := json.Unmarshal([]byte(lines[1]), &ev); err != nil || ev.Event != "end" || ev.Run.ID != "r1" {
		t.Errorf("end line %q: %v", lines[1], err)
	}
}

func TestWatchExitCodes(t *testing.T) {
	for _, tc := range []struct {
		state, passed string
		want          int
	}{
		{"completed", "true", ExitOK},
		{"completed", "", ExitOK},
		{"completed", "false", ExitFailed},
		{"failed", "", ExitFailed},
		{"aborted", "", ExitFailed},
	} {
		srv := newStub(t, func(w http.ResponseWriter, _ *http.Request) { writeJSON(w, 200, runBody(tc.state, tc.passed)) })
		res := cli(t, srv.URL, "", "watch", "r1")
		if res.code != tc.want {
			t.Errorf("%s/%s: exit %d, want %d, stderr %q", tc.state, tc.passed, res.code, tc.want, res.stderr)
		}
	}
	if res := cli(t, "http://127.0.0.1:1", "", "watch", "r1", "--view", "bogus"); res.code != ExitUsage {
		t.Errorf("bad view: exit %d", res.code)
	}
}

func TestWatchReconnectsAndFallsBack(t *testing.T) {
	// The first stream drops without an end event, and the second ends.
	var streams int
	var mu sync.Mutex
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/runs/r1":
			writeJSON(w, 200, runBody("running", ""))
		case "/v1/runs/r1/metrics:stream":
			mu.Lock()
			streams++
			n := streams
			mu.Unlock()
			if n == 1 {
				sse(w, sample("1", "2026-11-09T10:00:01Z", "a", 1))
				return
			}
			sse(w, sample("1", "2026-11-09T10:00:02Z", "a", 2), "event: end\ndata: {\"state\":\"completed\"}\n\n")
		}
	})
	res := cli(t, srv.URL, "", "watch", "r1")
	if res.code != 0 || !strings.Contains(res.stdout, "sessions.attempted=1") || !strings.Contains(res.stdout, "sessions.attempted=2") {
		t.Errorf("reconnect: exit %d, stdout %q, stderr %q", res.code, res.stdout, res.stderr)
	}

	// A controller without the stream falls back to polling.
	var polls int
	poll := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/runs/r1":
			mu.Lock()
			polls++
			n := polls
			mu.Unlock()
			if n < 3 {
				writeJSON(w, 200, runBody("running", ""))
			} else {
				writeJSON(w, 200, runBody("completed", "true"))
			}
		case "/v1/runs/r1/metrics:stream":
			problem(w, 501, "no stream")
		case "/v1/runs/r1/metrics":
			if r.URL.Query().Get("view") != "scenario" {
				writeJSON(w, 200, `{"view":"`+r.URL.Query().Get("view")+`","samples":[]}`)
				return
			}
			writeJSON(w, 200, `{"view":"scenario","samples":[
{"ts":"2026-11-09T10:00:01Z","view":"scenario","key":"a","values":{"sessions.attempted":1}},
{"ts":"2026-11-09T10:00:02Z","view":"scenario","key":"a","values":{"sessions.attempted":2}}]}`)
		}
	})
	res = cli(t, poll.URL, "", "watch", "r1", "--interval", "100ms")
	if res.code != 0 || !strings.Contains(res.stderr, "polling") || !strings.Contains(res.stdout, "scenario a sessions.attempted=2") {
		t.Errorf("fallback: exit %d, stdout %q, stderr %q", res.code, res.stdout, res.stderr)
	}
	// The repeated newest sample prints once.
	if n := strings.Count(res.stdout, "sessions.attempted=2"); n != 1 {
		t.Errorf("the repeated sample printed %d times:\n%s", n, res.stdout)
	}
	if res := cli(t, poll.URL, "", "watch", "r1", "--poll", "--interval", "100ms"); res.code != 0 || strings.Contains(res.stderr, "no metrics stream") {
		t.Errorf("--poll: exit %d, stderr %q", res.code, res.stderr)
	}
}

func TestRunStartWatch(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost:
			writeJSON(w, 202, `{"id":"r1","config_id":"c1","state":"pending"}`)
		case r.URL.Path == "/v1/runs/r1":
			writeJSON(w, 200, runBody("completed", "false"))
		}
	})
	res := cli(t, srv.URL, "", "run", "start", "c1", "--watch")
	if res.code != ExitFailed || !strings.Contains(res.stdout, "started run r1") || !strings.Contains(res.stderr, "missed its criteria") {
		t.Errorf("exit %d, stdout %q, stderr %q", res.code, res.stdout, res.stderr)
	}
}

func TestReport(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/runs/r1/report" {
			problem(w, 404, "no")
			return
		}
		switch r.URL.Query().Get("format") {
		case "csv":
			w.Header().Set("Content-Type", "text/csv")
			_, _ = io.WriteString(w, "metric,value\nsessions.attempted,9\n")
		case "html":
			w.Header().Set("Content-Type", "text/html")
			_, _ = io.WriteString(w, "<html><body>report</body></html>")
		default:
			writeJSON(w, 200, `{"run":`+runBody("completed", "true")+`,"realism":[{"property":"mix","observed":0.5,"target":"<= 1","passed":true}]}`)
		}
	})
	res := cli(t, srv.URL, "", "report", "r1")
	for _, want := range []string{"Run:", "completed", "Realism:", "mix", "pass"} {
		if res.code != 0 || !strings.Contains(res.stdout, want) {
			t.Errorf("text report lacks %q (exit %d):\n%s", want, res.code, res.stdout)
		}
	}
	if q := srv.last().query; q != "format=json" {
		t.Errorf("query = %q", q)
	}
	res = cli(t, srv.URL, "", "report", "r1", "--format", "csv")
	if res.stdout != "metric,value\nsessions.attempted,9\n" || srv.last().query != "format=csv" {
		t.Errorf("csv: %q, query %q", res.stdout, srv.last().query)
	}
	out := filepath.Join(t.TempDir(), "r.html")
	res = cli(t, srv.URL, "", "report", "r1", "--format", "html", "-o", out)
	data, err := os.ReadFile(out) //nolint:gosec // G304: the path is a temporary file of the test
	if err != nil || string(data) != "<html><body>report</body></html>" || res.stdout != "" {
		t.Errorf("html file: %q %v, stdout %q, stderr %q", data, err, res.stdout, res.stderr)
	}
	res = cli(t, srv.URL, "", "report", "r1", "--format", "json")
	if !strings.Contains(res.stdout, "\"realism\": [") {
		t.Errorf("json: %q", res.stdout)
	}
	res = cli(t, srv.URL, "", "--json", "report", "r1")
	if !strings.HasPrefix(res.stdout, "{") || srv.last().query != "format=json" {
		t.Errorf("--json: %q", res.stdout)
	}
	if res := cli(t, srv.URL, "", "report", "nope"); res.code != ExitNotFound {
		t.Errorf("unknown run: exit %d", res.code)
	}
}

func TestInterruptedWatch(t *testing.T) {
	srv := newStub(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/runs/r1":
			writeJSON(w, 200, runBody("running", ""))
		case "/v1/runs/r1/metrics:stream":
			sse(w, sample("1", "2026-11-09T10:00:01Z", "a", 1))
			<-r.Context().Done()
		}
	})
	ctx, cancel := context.WithCancel(context.Background())
	var out, errb bytes.Buffer
	done := make(chan int, 1)
	env := map[string]string{"FPS_URL": srv.URL}
	go func() {
		done <- Execute(ctx, []string{"watch", "r1"}, Env{Stdout: &out, Stderr: &errb, Getenv: func(k string) string { return env[k] }})
	}()
	time.Sleep(300 * time.Millisecond)
	cancel()
	select {
	case code := <-done:
		if code != ExitInterrupted || !strings.Contains(errb.String(), "keeps running") {
			t.Errorf("exit %d, stderr %q", code, errb.String())
		}
	case <-time.After(5 * time.Second):
		t.Fatal("watch did not end after the interrupt")
	}
}
