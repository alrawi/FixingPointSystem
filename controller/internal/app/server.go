// SPDX-License-Identifier: GPL-3.0-only

package app

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"time"
)

// HealthPath is the path of the liveness endpoint. It sits outside the /v1 API
// of the OpenAPI contract, so probes need no token.
const HealthPath = "/healthz"

// App is one controller process. It owns the HTTP server and holds every
// dependency that handlers need, so the package keeps no mutable state of its own.
type App struct {
	cfg    Config
	logger *slog.Logger
	mux    *http.ServeMux
}

// New returns an App for a validated configuration. Later packages mount their
// handlers with Handle before Run starts.
func New(cfg Config, logger *slog.Logger) *App {
	a := &App{cfg: cfg, logger: logger, mux: http.NewServeMux()}
	a.mux.HandleFunc("GET "+HealthPath, a.handleHealth)
	return a
}

// Handle registers a handler for a ServeMux pattern such as "GET /v1/capabilities".
func (a *App) Handle(pattern string, h http.Handler) {
	a.mux.Handle(pattern, h)
}

// Handler returns the root handler of the server, for tests that call it directly.
func (a *App) Handler() http.Handler {
	return a.mux
}

// healthResponse is the body of a successful health check.
type healthResponse struct {
	Status string `json:"status"`
}

func (a *App) handleHealth(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	if err := json.NewEncoder(w).Encode(healthResponse{Status: "ok"}); err != nil {
		a.logger.Warn("write health response", "error", err)
	}
}

// Listen opens the TCP listener for the configured address.
func (a *App) Listen(ctx context.Context) (net.Listener, error) {
	var lc net.ListenConfig
	ln, err := lc.Listen(ctx, "tcp", a.cfg.Listen)
	if err != nil {
		return nil, fmt.Errorf("listen on %s: %w", a.cfg.Listen, err)
	}
	return ln, nil
}

// Run serves HTTP on ln until ctx ends, then stops accepting connections and
// waits up to the shutdown timeout for in flight requests. It returns nil after a
// clean shutdown and closes ln in every case. When requests outlive the timeout,
// Run closes their connections and returns an error that wraps
// context.DeadlineExceeded.
func (a *App) Run(ctx context.Context, ln net.Listener) error {
	srv := &http.Server{
		Handler:           a.mux,
		ReadHeaderTimeout: time.Duration(a.cfg.ReadHeaderTimeout),
		ErrorLog:          slog.NewLogLogger(a.logger.Handler(), slog.LevelWarn),
		BaseContext:       func(net.Listener) context.Context { return context.WithoutCancel(ctx) },
	}

	serveErr := make(chan error, 1)
	go func() { serveErr <- srv.Serve(ln) }()
	a.logger.Info("controller listening", "addr", ln.Addr().String())

	select {
	case err := <-serveErr:
		return fmt.Errorf("serve: %w", err)
	case <-ctx.Done():
	}

	timeout := time.Duration(a.cfg.ShutdownTimeout)
	a.logger.Info("controller shutting down", "timeout", timeout.String())
	shutdownCtx, cancel := context.WithTimeout(context.WithoutCancel(ctx), timeout)
	defer cancel()
	if err := srv.Shutdown(shutdownCtx); err != nil {
		closeErr := srv.Close()
		<-serveErr
		return errors.Join(fmt.Errorf("shutdown: %w", err), closeErr)
	}
	if err := <-serveErr; !errors.Is(err, http.ErrServerClosed) {
		return fmt.Errorf("serve: %w", err)
	}
	a.logger.Info("controller stopped")
	return nil
}
