// SPDX-License-Identifier: GPL-3.0-only

// Command fps-controller runs the FixingPointSystem controller.
//
// Usage:
//
//	fps-controller [-config file] [-listen host:port] [-grpc-listen host:port]
//	               [-data-dir dir] [-log-level level]
//	fps-controller -version
//
// Settings come from three layers, where a later layer wins. The layers are the
// built in defaults, the YAML file named by -config, and the flags that appear on
// the command line. The process logs JSON lines to standard error, serves
// GET /healthz, the /v1 REST API, and the Prometheus exposition at
// GET /metrics on the listen address, and serves the agent gRPC service on the
// grpc_listen address. On SIGINT or SIGTERM it aborts the open runs, ends the
// agent sessions, and drains in flight requests for up to shutdown_timeout
// before it exits.
//
// Exit codes are 0 after a clean shutdown, 1 when the controller fails at run
// time, and 2 for invalid flags or configuration.
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"os"
	"os/signal"
	"runtime/debug"
	"syscall"

	"github.com/alrawi/FixingPointSystem/controller/internal/app"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/report"
	"github.com/alrawi/FixingPointSystem/controller/internal/wiring"
)

// Exit codes of the process.
const (
	exitOK      = 0
	exitFailure = 1
	exitUsage   = 2
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	code := run(ctx, os.Args[1:], os.Stdout, os.Stderr)
	stop()
	os.Exit(code)
}

// options holds the parsed command line.
type options struct {
	configPath string
	listen     string
	grpcListen string
	dataDir    string
	logLevel   string
	version    bool
	set        map[string]bool
}

// parseFlags parses args into options. The set map records which flags appeared,
// so that only those override the configuration file.
func parseFlags(args []string, stderr io.Writer) (options, error) {
	var o options
	fs := flag.NewFlagSet("fps-controller", flag.ContinueOnError)
	fs.SetOutput(stderr)
	fs.StringVar(&o.configPath, "config", "", "path of the YAML configuration `file`")
	fs.StringVar(&o.listen, "listen", app.DefaultListen, "TCP `address` of the HTTP server")
	fs.StringVar(&o.grpcListen, "grpc-listen", app.DefaultGRPCListen, "TCP `address` of the agent gRPC service")
	fs.StringVar(&o.dataDir, "data-dir", "", "`directory` of the module registry and the configuration store")
	fs.StringVar(&o.logLevel, "log-level", app.DefaultLogLevel, "log `level`: debug, info, warn, or error")
	fs.BoolVar(&o.version, "version", false, "print the version and exit")
	if err := fs.Parse(args); err != nil {
		return options{}, err
	}
	if fs.NArg() > 0 {
		return options{}, fmt.Errorf("unexpected argument %q", fs.Arg(0))
	}
	o.set = make(map[string]bool)
	fs.Visit(func(f *flag.Flag) { o.set[f.Name] = true })
	return o, nil
}

// loadConfig applies the defaults, the configuration file, and the flags in that
// order and validates the result.
func loadConfig(o options) (app.Config, error) {
	cfg := app.DefaultConfig()
	if o.configPath != "" {
		var err error
		if cfg, err = app.LoadConfigFile(o.configPath); err != nil {
			return app.Config{}, err
		}
	}
	if o.set["listen"] {
		cfg.Listen = o.listen
	}
	if o.set["grpc-listen"] {
		cfg.GRPCListen = o.grpcListen
	}
	if o.set["data-dir"] {
		cfg.DataDir = o.dataDir
	}
	if o.set["log-level"] {
		cfg.LogLevel = o.logLevel
	}
	if err := cfg.Validate(); err != nil {
		return app.Config{}, err
	}
	return cfg, nil
}

// version returns the module version that the Go toolchain stamped into the binary.
func version() string {
	if info, ok := debug.ReadBuildInfo(); ok && info.Main.Version != "" {
		return info.Main.Version
	}
	return "(devel)"
}

// run is the testable body of main. It returns the process exit code and stops
// the controller when ctx ends.
func run(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	o, err := parseFlags(args, stderr)
	if errors.Is(err, flag.ErrHelp) {
		return exitOK
	}
	if err != nil {
		_, _ = fmt.Fprintln(stderr, "fps-controller:", err)
		return exitUsage
	}
	if o.version {
		_, _ = fmt.Fprintln(stdout, "fps-controller", version())
		return exitOK
	}
	cfg, err := loadConfig(o)
	if err != nil {
		_, _ = fmt.Fprintln(stderr, "fps-controller:", err)
		return exitUsage
	}
	logger, err := app.NewLogger(stderr, cfg.LogLevel)
	if err != nil {
		_, _ = fmt.Fprintln(stderr, "fps-controller:", err)
		return exitUsage
	}
	logger.Info("controller starting", "version", version(), "config", o.configPath)

	if err := serve(ctx, cfg, logger); err != nil {
		logger.Error("controller failed", "error", err)
		if errors.Is(err, app.ErrInvalidConfig) {
			return exitUsage
		}
		return exitFailure
	}
	return exitOK
}

// serve builds the controller, serves the REST API and the agent gRPC service
// until ctx ends or one of them fails, and then shuts both down.
func serve(ctx context.Context, cfg app.Config, logger *slog.Logger) (err error) {
	ctl, err := wiring.New(ctx, wiring.Options{Config: cfg, Logger: logger})
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, ctl.Close()) }()

	a := app.New(cfg, logger)
	if err := mountMetrics(a, ctl, logger); err != nil {
		return err
	}
	ctl.Mount(a)

	ln, err := a.Listen(ctx)
	if err != nil {
		return err
	}
	gln, err := ctl.ListenGRPC(ctx)
	if err != nil {
		return errors.Join(err, ln.Close())
	}

	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	grpcDone := make(chan error, 1)
	go func() {
		err := ctl.Run(runCtx, gln)
		cancel() // a failing agent service stops the REST server too
		grpcDone <- err
	}()
	httpErr := a.Run(runCtx, ln)
	cancel()
	return errors.Join(httpErr, <-grpcDone)
}

// mountMetrics creates the metrics aggregator, feeds it every agent manager
// event, routes the run metrics operations to it, builds run reports over its
// merged metrics with the criteria evaluator of the controller, and serves
// the Prometheus exposition at GET /metrics without a token, as scrapers
// expect. Call it before ctl.Mount.
func mountMetrics(a *app.App, ctl *wiring.Controller, logger *slog.Logger) error {
	agg, err := metrics.New(metrics.Config{Logger: logger})
	if err != nil {
		return fmt.Errorf("create metrics aggregator: %w", err)
	}
	ctl.Observe(agg.Observe)
	ctl.SetMetrics(metrics.NewAPI(agg, ctl.Runs))
	ctl.SetReport(report.NewAPI(ctl.Evaluator, ctl.Runs, agg))
	a.Handle("GET /metrics", metrics.NewPrometheus(agg))
	return nil
}
