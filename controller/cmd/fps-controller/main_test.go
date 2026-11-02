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
	go func() { done <- run(ctx, []string{"-listen", "127.0.0.1:0"}, io.Discard, &stderr) }()

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

	cfg := writeConfig(t, "listen: 127.0.0.1:0\nshutdown_timeout: 5s\n")
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
	for _, want := range []string{"controller starting", "controller listening", "controller shutting down", "controller stopped"} {
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
