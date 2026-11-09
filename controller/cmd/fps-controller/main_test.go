// SPDX-License-Identifier: GPL-3.0-only

package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
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

	"github.com/alrawi/FixingPointSystem/controller/internal/app"
)

func writeConfig(t *testing.T, body string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "controller.yaml")
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestLoadConfigLayersDefaultsFileAndFlags(t *testing.T) {
	path := writeConfig(t, "listen: 127.0.0.1:9001\nlog_level: debug\n")
	tests := []struct {
		name       string
		args       []string
		wantListen string
		wantLevel  string
	}{
		{"defaults only", nil, app.DefaultListen, app.DefaultLogLevel},
		{"file over defaults", []string{"-config", path}, "127.0.0.1:9001", "debug"},
		{"flag over file", []string{"-config", path, "-listen", "127.0.0.1:9002"}, "127.0.0.1:9002", "debug"},
		{"flag over defaults", []string{"-log-level", "error"}, app.DefaultListen, "error"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			o, err := parseFlags(tc.args, io.Discard)
			if err != nil {
				t.Fatalf("parseFlags error = %v", err)
			}
			cfg, err := loadConfig(o)
			if err != nil {
				t.Fatalf("loadConfig error = %v", err)
			}
			if cfg.Listen != tc.wantListen || cfg.LogLevel != tc.wantLevel {
				t.Errorf("config = %s %s, want %s %s", cfg.Listen, cfg.LogLevel, tc.wantListen, tc.wantLevel)
			}
		})
	}
}

func TestLoadConfigLayersServiceSettings(t *testing.T) {
	path := writeConfig(t, "grpc_listen: 127.0.0.1:7001\ndata_dir: /from/file\n")
	tests := []struct {
		name     string
		args     []string
		wantGRPC string
		wantData string
	}{
		{"defaults only", nil, app.DefaultGRPCListen, ""},
		{"file over defaults", []string{"-config", path}, "127.0.0.1:7001", "/from/file"},
		{"flags over file", []string{"-config", path, "-grpc-listen", "127.0.0.1:7002", "-data-dir", "/from/flag"}, "127.0.0.1:7002", "/from/flag"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			o, err := parseFlags(tc.args, io.Discard)
			if err != nil {
				t.Fatal(err)
			}
			cfg, err := loadConfig(o)
			if err != nil {
				t.Fatal(err)
			}
			if cfg.GRPCListen != tc.wantGRPC || cfg.DataDir != tc.wantData {
				t.Errorf("config = %s %q, want %s %q", cfg.GRPCListen, cfg.DataDir, tc.wantGRPC, tc.wantData)
			}
		})
	}
}

func TestLoadConfigValidatesFlagValues(t *testing.T) {
	o, err := parseFlags([]string{"-log-level", "chatty"}, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := loadConfig(o); !errors.Is(err, app.ErrInvalidConfig) {
		t.Fatalf("loadConfig error = %v, want ErrInvalidConfig", err)
	}
}

func TestRunExitCodes(t *testing.T) {
	bad := writeConfig(t, "shutdown_timeout: soon\n")
	tokenFile := writeConfig(t, "tokens: [{name: ops, scope: admin, token_file: "+filepath.Join(t.TempDir(), "absent")+"}]\n")
	tests := []struct {
		name       string
		args       []string
		want       int
		wantStdout string
		wantStderr string
	}{
		{"version", []string{"-version"}, exitOK, "fps-controller ", ""},
		{"help", []string{"-h"}, exitOK, "", "-config file"},
		{"unknown flag", []string{"-bogus"}, exitUsage, "", "flag provided but not defined"},
		{"positional argument", []string{"serve"}, exitUsage, "", `unexpected argument "serve"`},
		{"missing config file", []string{"-config", filepath.Join(t.TempDir(), "absent.yaml")}, exitUsage, "", "no such file"},
		{"invalid config file", []string{"-config", bad}, exitUsage, "", "invalid duration"},
		{"port out of range", []string{"-listen", "127.0.0.1:99999", "-log-level", "error"}, exitFailure, "", "controller failed"},
		{"grpc port out of range", []string{"-listen", "127.0.0.1:0", "-grpc-listen", "127.0.0.1:99999", "-log-level", "error"}, exitFailure, "", "controller failed"},
		{"grpc listen without port", []string{"-grpc-listen", "localhost"}, exitUsage, "", "grpc_listen"},
		{"enroll listen without port", []string{"-enroll-listen", "localhost"}, exitUsage, "", "enroll_listen"},
		{"enroll port out of range", []string{"-listen", "127.0.0.1:0", "-grpc-listen", "127.0.0.1:0", "-enroll-listen", "127.0.0.1:99999", "-log-level", "error"}, exitFailure, "", "controller failed"},
		{"token without an agent id", []string{"token"}, exitUsage, "", "-agent-id is required"},
		{"revoke without an agent id", []string{"revoke", "-admin-socket", "/nowhere.sock"}, exitUsage, "", "-agent-id is required"},
		{"token without a socket", []string{"token", "-agent-id", "a"}, exitUsage, "", "no admin socket"},
		{"token with a stray argument", []string{"token", "-agent-id", "a", "extra"}, exitUsage, "", `unexpected argument "extra"`},
		{"token with no controller", []string{"token", "-agent-id", "a", "-admin-socket", filepath.Join(t.TempDir(), "absent.sock")}, exitFailure, "", "start the controller first"},
		{"revoke with no controller", []string{"revoke", "-agent-id", "a", "-admin-socket", filepath.Join(t.TempDir(), "absent.sock")}, exitFailure, "", "start the controller first"},
		{"missing token file", []string{"-config", tokenFile, "-listen", "127.0.0.1:0", "-grpc-listen", "127.0.0.1:0"}, exitUsage, "", "token ops"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			got := run(context.Background(), tc.args, &stdout, &stderr)
			if got != tc.want {
				t.Fatalf("run = %d, want %d; stderr:\n%s", got, tc.want, stderr.String())
			}
			if !strings.Contains(stdout.String(), tc.wantStdout) {
				t.Errorf("stdout = %q, want it to contain %q", stdout.String(), tc.wantStdout)
			}
			if !strings.Contains(stderr.String(), tc.wantStderr) {
				t.Errorf("stderr = %q, want it to contain %q", stderr.String(), tc.wantStderr)
			}
		})
	}
}

func TestRunStopsWhenContextEnds(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	var stderr lockedBuffer
	done := make(chan int, 1)
	go func() {
		done <- run(ctx, []string{"-listen", "127.0.0.1:0", "-grpc-listen", "127.0.0.1:0", "-enroll-listen", "127.0.0.1:0"}, io.Discard, &stderr)
	}()

	addr := waitForListen(t, &stderr)
	resp, err := http.Get("http://" + addr + app.HealthPath)
	if err != nil {
		t.Fatalf("GET healthz error = %v", err)
	}
	_ = resp.Body.Close()
	cancel()
	select {
	case code := <-done:
		if code != exitOK {
			t.Fatalf("run = %d, want 0; stderr:\n%s", code, stderr.String())
		}
	case <-time.After(10 * time.Second):
		t.Fatal("run did not return after cancel")
	}
}

// startInProcess runs the controller in process on loopback ports and returns
// its HTTP address and the captured logs. The controller stops with the test.
func startInProcess(t *testing.T, extra ...string) (string, *lockedBuffer) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	var stderr lockedBuffer
	done := make(chan int, 1)
	args := append([]string{"-listen", "127.0.0.1:0", "-grpc-listen", "127.0.0.1:0", "-enroll-listen", "127.0.0.1:0"}, extra...)
	go func() { done <- run(ctx, args, io.Discard, &stderr) }()
	t.Cleanup(func() {
		cancel()
		select {
		case <-done:
		case <-time.After(10 * time.Second):
			t.Error("controller did not stop")
		}
	})
	return waitForListen(t, &stderr), &stderr
}

func getMetrics(t *testing.T, addr, authorization string) int {
	t.Helper()
	req, err := http.NewRequestWithContext(context.Background(), http.MethodGet, "http://"+addr+"/metrics", nil)
	if err != nil {
		t.Fatal(err)
	}
	if authorization != "" {
		req.Header.Set("Authorization", authorization)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("GET /metrics error = %v", err)
	}
	_ = resp.Body.Close()
	return resp.StatusCode
}

func TestMetricsEndpointIsOpenByDefault(t *testing.T) {
	addr, _ := startInProcess(t)
	if got := getMetrics(t, addr, ""); got != http.StatusOK {
		t.Fatalf("GET /metrics without a token = %d, want 200", got)
	}
}

func TestMetricsEndpointRequiresConfiguredToken(t *testing.T) {
	const token = "scrape-token-0123456789"
	tokenFile := filepath.Join(t.TempDir(), "metrics.token")
	if err := os.WriteFile(tokenFile, []byte(token+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	addr, logs := startInProcess(t, "-metrics-token-file", tokenFile)

	tests := []struct {
		name   string
		header string
		want   int
	}{
		{"no token", "", http.StatusUnauthorized},
		{"wrong token", "Bearer wrong-token-0123456789", http.StatusUnauthorized},
		{"right token", "Bearer " + token, http.StatusOK},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := getMetrics(t, addr, tc.header); got != tc.want {
				t.Fatalf("GET /metrics = %d, want %d", got, tc.want)
			}
		})
	}
	if strings.Contains(logs.String(), token) {
		t.Errorf("the logs contain the metrics token:\n%s", logs.String())
	}
	// Health probes and the open API surface stay outside the token.
	resp, err := http.Get("http://" + addr + app.HealthPath)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Errorf("healthz = %d, want 200", resp.StatusCode)
	}
}

func TestRunRejectsUnreadableMetricsTokenFile(t *testing.T) {
	var stderr bytes.Buffer
	args := []string{"-listen", "127.0.0.1:0", "-grpc-listen", "127.0.0.1:0", "-metrics-token-file", filepath.Join(t.TempDir(), "absent")}
	if got := run(context.Background(), args, io.Discard, &stderr); got != exitUsage {
		t.Fatalf("run = %d, want %d; stderr:\n%s", got, exitUsage, stderr.String())
	}
	if !strings.Contains(stderr.String(), "metrics_token_file") {
		t.Errorf("stderr = %q, want it to name metrics_token_file", stderr.String())
	}
}

func TestLoadConfigLayersMetricsTokenFile(t *testing.T) {
	path := writeConfig(t, "metrics_token_file: /from/file\n")
	for _, tc := range []struct {
		args []string
		want string
	}{
		{nil, ""},
		{[]string{"-config", path}, "/from/file"},
		{[]string{"-config", path, "-metrics-token-file", "/from/flag"}, "/from/flag"},
	} {
		o, err := parseFlags(tc.args, io.Discard)
		if err != nil {
			t.Fatal(err)
		}
		cfg, err := loadConfig(o)
		if err != nil {
			t.Fatal(err)
		}
		if cfg.MetricsTokenFile != tc.want {
			t.Errorf("args %v: MetricsTokenFile = %q, want %q", tc.args, cfg.MetricsTokenFile, tc.want)
		}
	}
}

func TestLoadConfigLayersTrustStoreSettings(t *testing.T) {
	path := writeConfig(t, "trust_store_file: /from/file.json\nallow_unsigned_modules: true\n")
	for _, tc := range []struct {
		name      string
		args      []string
		wantStore string
		wantAllow bool
	}{
		{"defaults", nil, "", false},
		{"file", []string{"-config", path}, "/from/file.json", true},
		{"flags", []string{"-trust-store-file", "/from/flag.json", "-allow-unsigned-modules"}, "/from/flag.json", true},
		{"flags over file", []string{"-config", path, "-trust-store-file", "/from/flag.json", "-allow-unsigned-modules=false"}, "/from/flag.json", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			o, err := parseFlags(tc.args, io.Discard)
			if err != nil {
				t.Fatal(err)
			}
			cfg, err := loadConfig(o)
			if err != nil {
				t.Fatal(err)
			}
			if cfg.TrustStoreFile != tc.wantStore || cfg.AllowUnsignedModules != tc.wantAllow {
				t.Errorf("config = %q %v, want %q %v", cfg.TrustStoreFile, cfg.AllowUnsignedModules, tc.wantStore, tc.wantAllow)
			}
		})
	}
}

func TestLoadConfigLayersCaptureRetention(t *testing.T) {
	path := writeConfig(t, "capture_retention: 48h\n")
	for _, tc := range []struct {
		name string
		args []string
		want time.Duration
	}{
		{"defaults", nil, 7 * 24 * time.Hour},
		{"file", []string{"-config", path}, 48 * time.Hour},
		{"flag", []string{"-capture-retention", "1h30m"}, 90 * time.Minute},
		{"flag over file", []string{"-config", path, "-capture-retention", "2h"}, 2 * time.Hour},
	} {
		t.Run(tc.name, func(t *testing.T) {
			o, err := parseFlags(tc.args, io.Discard)
			if err != nil {
				t.Fatal(err)
			}
			cfg, err := loadConfig(o)
			if err != nil {
				t.Fatal(err)
			}
			if got := time.Duration(cfg.CaptureRetention); got != tc.want {
				t.Errorf("CaptureRetention = %v, want %v", got, tc.want)
			}
		})
	}
	o, err := parseFlags([]string{"-capture-retention", "0s"}, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := loadConfig(o); err == nil {
		t.Error("loadConfig accepted -capture-retention 0s")
	}
}

// TestBinaryShutsDownOnSIGTERM builds the real binary, starts it, checks the
// health endpoint, sends SIGTERM, and expects exit code 0 with JSON logs.
func TestBinaryShutsDownOnSIGTERM(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGTERM is not deliverable on Windows")
	}
	if testing.Short() {
		t.Skip("builds the binary")
	}
	bin := filepath.Join(t.TempDir(), "fps-controller")
	build := exec.Command("go", "build", "-o", bin, ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}

	cfg := writeConfig(t, "listen: 127.0.0.1:0\ngrpc_listen: 127.0.0.1:0\nenroll_listen: 127.0.0.1:0\nshutdown_timeout: 5s\n")
	var stderr lockedBuffer
	cmd := exec.Command(bin, "-config", cfg)
	cmd.Stderr = &stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill() })

	addr := waitForListen(t, &stderr)
	resp, err := http.Get("http://" + addr + app.HealthPath)
	if err != nil {
		t.Fatalf("GET healthz error = %v", err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("healthz status = %d, want 200", resp.StatusCode)
	}

	if err := cmd.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	waitErr := make(chan error, 1)
	go func() { waitErr <- cmd.Wait() }()
	select {
	case err := <-waitErr:
		if err != nil {
			t.Fatalf("process exit = %v, want 0; stderr:\n%s", err, stderr.String())
		}
	case <-time.After(10 * time.Second):
		t.Fatal("process did not exit within 10s of SIGTERM")
	}
	msgs := logMessages(t, stderr.String())
	for _, want := range []string{
		"controller starting", "controller listening", "controller shutting down", "controller stopped",
		"agent service listening", "agent service shutting down", "agent service stopped",
		"certificate authority open", "enrollment service listening",
	} {
		if !msgs[want] {
			t.Errorf("logs lack %q:\n%s", want, stderr.String())
		}
	}
}

// waitForListen reads JSON log lines until the "controller listening" record
// appears and returns its addr attribute.
func waitForListen(t *testing.T, logs *lockedBuffer) string {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		sc := bufio.NewScanner(strings.NewReader(logs.String()))
		for sc.Scan() {
			var rec struct {
				Msg  string `json:"msg"`
				Addr string `json:"addr"`
			}
			if json.Unmarshal(sc.Bytes(), &rec) == nil && rec.Msg == "controller listening" {
				return rec.Addr
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("no listening record within 10s; logs:\n%s", logs.String())
	return ""
}

// logMessages parses every line of logs as JSON and returns the set of msg values.
func logMessages(t *testing.T, logs string) map[string]bool {
	t.Helper()
	msgs := make(map[string]bool)
	for _, line := range strings.Split(strings.TrimSpace(logs), "\n") {
		var rec map[string]any
		if err := json.Unmarshal([]byte(line), &rec); err != nil {
			t.Fatalf("log line %q is not JSON: %v", line, err)
		}
		if msg, ok := rec["msg"].(string); ok {
			msgs[msg] = true
		}
	}
	return msgs
}

// lockedBuffer is a bytes.Buffer that one goroutine writes while another reads.
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

// TestVersionStamp builds the command with -ldflags -X main.version and checks
// that -version prints the stamped value.
func TestVersionStamp(t *testing.T) {
	if testing.Short() {
		t.Skip("builds the binary")
	}
	const stamp = "1.2.3-test"
	bin := filepath.Join(t.TempDir(), "fps-controller")
	build := exec.Command("go", "build", "-ldflags", "-X main.version="+stamp, "-o", bin, ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}
	out, err := exec.Command(bin, "-version").Output()
	if err != nil {
		t.Fatalf("-version: %v", err)
	}
	if got, want := strings.TrimSpace(string(out)), "fps-controller "+stamp; got != want {
		t.Fatalf("-version printed %q, want %q", got, want)
	}
}

// TestBuildVersionFallback checks that an empty stamp falls back to the build
// information and that a set stamp wins.
func TestBuildVersionFallback(t *testing.T) {
	old := version
	t.Cleanup(func() { version = old })
	version = ""
	if got := buildVersion(); got == "" {
		t.Fatal("buildVersion is empty without a stamp")
	}
	version = "9.9.9"
	if got := buildVersion(); got != "9.9.9" {
		t.Fatalf("buildVersion = %q, want the stamp", got)
	}
}
