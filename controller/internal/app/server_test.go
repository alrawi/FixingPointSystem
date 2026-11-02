// SPDX-License-Identifier: GPL-3.0-only

package app

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// testConfig returns a configuration with short timeouts for tests.
func testConfig() Config {
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	cfg.ShutdownTimeout = Duration(5 * time.Second)
	return cfg
}

// startApp runs a on a loopback listener and returns its base URL, a cancel
// function that stops it, and a channel that yields the result of Run.
func startApp(t *testing.T, a *App) (string, context.CancelFunc, <-chan error) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	ln, err := a.Listen(ctx)
	if err != nil {
		cancel()
		t.Fatalf("Listen error = %v", err)
	}
	done := make(chan error, 1)
	finished := make(chan struct{})
	go func() {
		defer close(finished)
		done <- a.Run(ctx, ln)
	}()
	t.Cleanup(func() {
		cancel()
		select {
		case <-finished:
		case <-time.After(10 * time.Second):
			t.Error("Run did not return after cancel")
		}
	})
	return "http://" + ln.Addr().String(), cancel, done
}

func waitRun(t *testing.T, done <-chan error) error {
	t.Helper()
	select {
	case err := <-done:
		return err
	case <-time.After(10 * time.Second):
		t.Fatal("Run did not return within 10s")
		return nil
	}
}

func TestHealthzReturnsOK(t *testing.T) {
	a := New(testConfig(), slog.New(slog.DiscardHandler))
	rec := httptest.NewRecorder()
	a.Handler().ServeHTTP(rec, httptest.NewRequest(http.MethodGet, HealthPath, nil))

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200", rec.Code)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "application/json" {
		t.Errorf("Content-Type = %q, want application/json", ct)
	}
	var body healthResponse
	if err := json.NewDecoder(rec.Body).Decode(&body); err != nil || body.Status != "ok" {
		t.Errorf("body = %+v, %v, want status ok", body, err)
	}
}

func TestHealthzRejectsOtherMethods(t *testing.T) {
	a := New(testConfig(), slog.New(slog.DiscardHandler))
	rec := httptest.NewRecorder()
	a.Handler().ServeHTTP(rec, httptest.NewRequest(http.MethodPost, HealthPath, nil))
	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("status = %d, want 405", rec.Code)
	}
}

func TestRunServesHealthzAndStopsCleanly(t *testing.T) {
	var logs bytes.Buffer
	// The JSON handler serializes writes, and the test reads logs only after Run
	// returned, so a plain buffer is safe here.
	logger, err := NewLogger(&logs, "info")
	if err != nil {
		t.Fatal(err)
	}
	base, cancel, done := startApp(t, New(testConfig(), logger))

	resp, err := http.Get(base + HealthPath)
	if err != nil {
		t.Fatalf("GET %s error = %v", HealthPath, err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d, want 200", resp.StatusCode)
	}

	cancel()
	if err := waitRun(t, done); err != nil {
		t.Fatalf("Run error = %v, want nil", err)
	}
	for _, msg := range []string{"controller listening", "controller shutting down", "controller stopped"} {
		if !strings.Contains(logs.String(), `"msg":"`+msg+`"`) {
			t.Errorf("logs lack %q:\n%s", msg, logs.String())
		}
	}
}

func TestRunDrainsInFlightRequests(t *testing.T) {
	a := New(testConfig(), slog.New(slog.DiscardHandler))
	entered, release := make(chan struct{}), make(chan struct{})
	a.Handle("GET /slow", http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		close(entered)
		<-release
		_, _ = io.WriteString(w, "done")
	}))
	base, cancel, done := startApp(t, a)

	type result struct {
		body string
		err  error
	}
	got := make(chan result, 1)
	go func() {
		resp, err := http.Get(base + "/slow")
		if err != nil {
			got <- result{err: err}
			return
		}
		defer func() { _ = resp.Body.Close() }()
		b, err := io.ReadAll(resp.Body)
		got <- result{string(b), err}
	}()
	<-entered
	cancel()

	// The listener closes at once, while the slow request keeps running.
	deadline := time.Now().Add(5 * time.Second)
	for {
		c, err := net.DialTimeout("tcp", strings.TrimPrefix(base, "http://"), 100*time.Millisecond)
		if err != nil {
			break
		}
		_ = c.Close()
		if time.Now().After(deadline) {
			t.Fatal("listener still accepts connections after cancel")
		}
		time.Sleep(10 * time.Millisecond)
	}
	select {
	case err := <-done:
		t.Fatalf("Run returned %v before the in flight request finished", err)
	default:
	}

	close(release)
	if r := <-got; r.err != nil || r.body != "done" {
		t.Fatalf("slow request = %q, %v, want done", r.body, r.err)
	}
	if err := waitRun(t, done); err != nil {
		t.Fatalf("Run error = %v, want nil", err)
	}
}

func TestRunReportsShutdownTimeout(t *testing.T) {
	cfg := testConfig()
	cfg.ShutdownTimeout = Duration(50 * time.Millisecond)
	a := New(cfg, slog.New(slog.DiscardHandler))
	entered, release := make(chan struct{}), make(chan struct{})
	defer close(release)
	a.Handle("GET /stuck", http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		close(entered)
		<-release
	}))
	base, cancel, done := startApp(t, a)

	go func() {
		if resp, err := http.Get(base + "/stuck"); err == nil {
			_ = resp.Body.Close()
		}
	}()
	<-entered
	cancel()
	if err := waitRun(t, done); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("Run error = %v, want one that wraps context.DeadlineExceeded", err)
	}
}

func TestRunReturnsServeError(t *testing.T) {
	a := New(testConfig(), slog.New(slog.DiscardHandler))
	ln, err := a.Listen(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_ = ln.Close()
	if err := a.Run(context.Background(), ln); !errors.Is(err, net.ErrClosed) {
		t.Fatalf("Run error = %v, want one that wraps net.ErrClosed", err)
	}
}

func TestListenReportsBadAddress(t *testing.T) {
	cfg := testConfig()
	cfg.Listen = "127.0.0.1:99999"
	if _, err := New(cfg, slog.New(slog.DiscardHandler)).Listen(context.Background()); err == nil {
		t.Fatal("Listen error = nil, want an error for port 99999")
	}
}
