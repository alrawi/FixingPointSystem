// SPDX-License-Identifier: GPL-3.0-only

// Package wiring builds the controller from its packages. It opens the
// configuration store and the module registry, creates the agent manager and
// the run orchestrator, adapts them to the interfaces that the other packages
// read, and combines every REST handler into one api.ServerInterface.
//
//	ctl, err := wiring.New(ctx, wiring.Options{Config: cfg, Logger: logger})
//	defer ctl.Close()
//	ctl.Mount(a)                 // a is the *app.App
//	gln, err := ctl.ListenGRPC(ctx)
//	go ctl.Run(ctx, gln)         // agent gRPC service and heartbeat sweeps
//	a.Run(ctx, ln)               // REST API
package wiring

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"sync"
	"time"

	"google.golang.org/grpc"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/app"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry"
	"github.com/alrawi/FixingPointSystem/controller/internal/report"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// ControllerID names this controller in every RegisterResponse.
const ControllerID = "fps-controller"

// Options configures New.
type Options struct {
	// Config is the validated controller configuration.
	Config app.Config
	// Logger receives structured logs. Nil discards them.
	Logger *slog.Logger
}

// Controller owns every component of one controller process. Its exported
// fields let later packages, such as the metrics package of CTL-08, read the
// components that they build on.
type Controller struct {
	// Store holds the test configurations.
	Store *store.SQLite
	// Registry holds the installed modules.
	Registry *registry.Registry
	// Agents is the agent manager, which serves ControllerService.
	Agents *agents.Manager
	// Runs is the run orchestrator.
	Runs *runs.Orchestrator
	// Evaluator decides the criteria of every run with the formulas of the
	// embedded metrics catalog. The orchestrator calls it when a run stops,
	// and the report handler calls it again over the merged metrics.
	Evaluator *report.Evaluator
	// API is the combined REST server.
	API *Server

	cfg     app.Config
	logger  *slog.Logger
	auth    api.Authenticator
	grpc    *grpc.Server
	tempDir string

	mu        sync.RWMutex
	observers []func(agents.Event)
}

// New opens the store and the registry, creates the agent manager and the
// run orchestrator, and builds the REST server. The caller calls Close when
// New succeeds.
func New(ctx context.Context, opts Options) (_ *Controller, err error) {
	cfg := opts.Config
	logger := opts.Logger
	if logger == nil {
		logger = slog.New(slog.DiscardHandler)
	}
	c := &Controller{cfg: cfg, logger: logger}
	defer func() {
		if err != nil {
			err = errors.Join(err, c.Close())
		}
	}()

	if c.auth, err = authenticator(cfg); err != nil {
		return nil, err
	}

	storePath := cfg.ResolvedStorePath()
	if storePath != app.MemoryStorePath {
		if err := os.MkdirAll(filepath.Dir(storePath), 0o750); err != nil {
			return nil, fmt.Errorf("open store %s: %w", storePath, err)
		}
	}
	if c.Store, err = store.Open(ctx, storePath); err != nil {
		return nil, err
	}

	regDir := cfg.RegistryDir()
	if regDir == "" {
		if c.tempDir, err = os.MkdirTemp("", "fps-registry-"); err != nil {
			return nil, fmt.Errorf("create temporary registry: %w", err)
		}
		regDir = c.tempDir
		logger.Warn("data_dir is not set, so configurations and modules end with the process", "registry", regDir)
	}
	if c.Registry, err = registry.Open(regDir, registry.Limits{}); err != nil {
		return nil, err
	}

	// The gRPC service speaks plain TCP until CTL-15 adds mutual TLS, so no
	// certificate proves an agent identity yet.
	if c.Agents, err = agents.New(agents.Config{
		ControllerID: ControllerID,
		AutoEnroll:   cfg.AgentAutoEnroll,
		Identity:     agents.ClaimedIdentity,
		Logger:       logger,
		Observer:     c.observe,
	}); err != nil {
		return nil, fmt.Errorf("create agent manager: %w", err)
	}

	if c.Evaluator, err = report.NewEvaluator(nil); err != nil {
		return nil, fmt.Errorf("create criteria evaluator: %w", err)
	}
	catalog := Catalog{Registry: c.Registry}
	inventory := Inventory{Agents: c.Agents}
	if c.Runs, err = runs.New(runs.Config{
		Store:     c.Store,
		Agents:    c.Agents,
		Catalog:   catalog,
		Evaluator: c.Evaluator.RunEvaluator(),
		Logger:    logger,
	}); err != nil {
		return nil, fmt.Errorf("create run orchestrator: %w", err)
	}
	c.Observe(c.Runs.Observe)

	configs, err := store.NewHandler(c.Store, store.Options{Modules: catalog, Agents: inventory, Logger: logger})
	if err != nil {
		return nil, fmt.Errorf("create config handler: %w", err)
	}
	c.API = NewServer(Handlers{
		Agents:       agents.NewAPI(c.Agents),
		Capabilities: api.NewCapabilitiesAPI(catalog, inventory),
		Configs:      configs,
		Modules:      registry.NewAPI(c.Registry, logger),
		Runs:         runs.NewAPI(c.Runs),
		Report:       report.NewAPI(c.Evaluator, c.Runs, nil),
	})
	return c, nil
}

// authenticator builds the token check from the token settings.
func authenticator(cfg app.Config) (api.Authenticator, error) {
	secrets, err := cfg.LoadTokens()
	if err != nil {
		return nil, err
	}
	tokens := make(map[string]api.Scope, len(secrets))
	for secret, name := range secrets {
		scope, err := api.ParseScope(name)
		if err != nil {
			return nil, fmt.Errorf("%w: %w", app.ErrInvalidConfig, err)
		}
		tokens[secret] = scope
	}
	st, err := api.NewStaticTokens(tokens)
	if err != nil {
		return nil, fmt.Errorf("%w: %w", app.ErrInvalidConfig, err)
	}
	return st, nil
}

// Observe adds fn to the observers of agent manager events. The run
// orchestrator is the first observer, and the metrics package of CTL-08 can
// add its own. fn runs on the goroutine of the event and must return quickly.
func (c *Controller) Observe(fn func(agents.Event)) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.observers = append(c.observers, fn)
}

func (c *Controller) observe(ev agents.Event) {
	c.mu.RLock()
	obs := c.observers
	c.mu.RUnlock()
	for _, fn := range obs {
		fn(ev)
	}
}

// SetMetrics routes the two metrics operations of a run to h. Call it before
// Mount. It is the hook for the metrics package of CTL-08.
func (c *Controller) SetMetrics(h MetricsHandler) {
	c.API.metrics = h
}

// SetReport routes the report operation of a run to h. Call it before Mount.
// New routes it to a report.API without merged metrics, and the binary
// replaces that handler with one that reads the metrics aggregator.
func (c *Controller) SetReport(h ReportHandler) {
	c.API.report = h
}

// Mount registers the REST API on m with the token check of the settings.
func (c *Controller) Mount(m api.Mux) {
	if len(c.cfg.Tokens) == 0 {
		c.logger.Warn("no API tokens are configured, so every /v1 operation answers 401")
	}
	api.New(c.API, c.auth, c.logger).Mount(m)
}

// ListenGRPC opens the TCP listener of the agent gRPC service.
func (c *Controller) ListenGRPC(ctx context.Context) (net.Listener, error) {
	var lc net.ListenConfig
	ln, err := lc.Listen(ctx, "tcp", c.cfg.GRPCListen)
	if err != nil {
		return nil, fmt.Errorf("listen on %s: %w", c.cfg.GRPCListen, err)
	}
	return ln, nil
}

// Run serves ControllerService on ln and sweeps heartbeat timeouts until ctx
// ends. Then it aborts the open runs, ends every agent session, and stops the
// gRPC server within the shutdown timeout. It closes ln in every case.
func (c *Controller) Run(ctx context.Context, ln net.Listener) error {
	c.grpc = grpc.NewServer()
	agentpb.RegisterControllerServiceServer(c.grpc, c.Agents)

	serveErr := make(chan error, 1)
	go func() { serveErr <- c.grpc.Serve(ln) }()
	sweepCtx, stopSweep := context.WithCancel(ctx)
	sweepDone := make(chan struct{})
	go func() {
		defer close(sweepDone)
		_ = c.Agents.Run(sweepCtx)
	}()
	c.logger.Info("agent service listening", "addr", ln.Addr().String(), "transport", "plain tcp")
	c.logger.Warn("the agent service has no TLS, so agent identities are not verified")

	var err error
	select {
	case err = <-serveErr:
		err = fmt.Errorf("serve agent service: %w", err)
	case <-ctx.Done():
	}
	stopSweep()
	<-sweepDone

	c.logger.Info("agent service shutting down")
	c.Runs.Close()
	c.Agents.Shutdown()
	stopped := make(chan struct{})
	go func() {
		c.grpc.GracefulStop()
		close(stopped)
	}()
	select {
	case <-stopped:
	case <-time.After(time.Duration(c.cfg.ShutdownTimeout)):
		c.grpc.Stop()
		<-stopped
	}
	if err == nil {
		c.logger.Info("agent service stopped")
	}
	return err
}

// Close aborts the open runs, ends every agent session, releases the store,
// and removes the temporary registry. It is safe to call more than once.
func (c *Controller) Close() error {
	if c.Runs != nil {
		c.Runs.Close()
	}
	if c.Agents != nil {
		c.Agents.Shutdown()
	}
	var errs []error
	if c.Store != nil {
		if err := c.Store.Close(); err != nil {
			errs = append(errs, fmt.Errorf("close store: %w", err))
		}
		c.Store = nil
	}
	if c.tempDir != "" {
		if err := os.RemoveAll(c.tempDir); err != nil {
			errs = append(errs, fmt.Errorf("remove temporary registry: %w", err))
		}
		c.tempDir = ""
	}
	return errors.Join(errs...)
}
