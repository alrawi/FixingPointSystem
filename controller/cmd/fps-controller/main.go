// SPDX-License-Identifier: GPL-3.0-only

// Command fps-controller runs the FixingPointSystem controller.
//
// Usage:
//
//	fps-controller [-config file] [-listen host:port] [-grpc-listen host:port]
//	               [-enroll-listen host:port] [-data-dir dir] [-pki-dir dir]
//	               [-tls-names name,...] [-admin-socket path]
//	               [-metrics-token-file file] [-trust-store-file file]
//	               [-allow-unsigned-modules] [-capture-retention duration]
//	               [-log-level level]
//	               [-insecure-plain-grpc]
//	fps-controller token  -agent-id id [-ttl duration] [-config file | -data-dir dir | -admin-socket path]
//	fps-controller revoke -agent-id id [-config file | -data-dir dir | -admin-socket path]
//	fps-controller -version
//
// Settings come from three layers, where a later layer wins. The layers are the
// built in defaults, the YAML file named by -config, and the flags that appear on
// the command line. The process logs JSON lines to standard error, serves
// GET /healthz, the /v1 REST API, and the Prometheus exposition at
// GET /metrics on the listen address, and serves the agent gRPC service on the
// grpc_listen address. The gRPC service requires mutual TLS. The controller
// opens its certificate authority from pki_dir (default data_dir/pki), serves
// enrollment and renewal of agent certificates over HTTPS on enroll_listen, and
// serves an admin socket for the token and revoke subcommands, which ask the
// running controller to mint an enrollment token or to revoke an agent
// identity. -insecure-plain-grpc serves the gRPC service without TLS for
// development and tests. On SIGINT or SIGTERM it aborts the open runs, ends the
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
	"net"
	"os"
	"os/signal"
	"runtime/debug"
	"strings"
	"syscall"
	"time"

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
	configPath       string
	listen           string
	grpcListen       string
	enrollListen     string
	dataDir          string
	pkiDir           string
	tlsNames         string
	adminSocket      string
	insecurePlain    bool
	metricsTokenFile string
	trustStoreFile   string
	allowUnsigned    bool
	captureRetention time.Duration
	logLevel         string
	version          bool
	set              map[string]bool
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
	fs.StringVar(&o.enrollListen, "enroll-listen", app.DefaultEnrollListen, "TCP `address` of the HTTPS enrollment and renewal service")
	fs.StringVar(&o.dataDir, "data-dir", "", "`directory` of the module registry and the configuration store")
	fs.StringVar(&o.pkiDir, "pki-dir", "", "`directory` of the controller CA; empty means data-dir/pki")
	fs.StringVar(&o.tlsNames, "tls-names", "", "comma separated host names and IP addresses of the controller certificate")
	fs.StringVar(&o.adminSocket, "admin-socket", "", "unix socket `path` of the admin API; empty means pki-dir/admin.sock")
	fs.BoolVar(&o.insecurePlain, "insecure-plain-grpc", false, "serve the agent gRPC service without TLS and trust agent ids (development and tests only)")
	fs.StringVar(&o.metricsTokenFile, "metrics-token-file", "", "`file` that holds the bearer token for GET /metrics; empty leaves the endpoint open")
	fs.StringVar(&o.trustStoreFile, "trust-store-file", "", "`file` of the module signing trust store; empty means the registry does not require signatures")
	fs.BoolVar(&o.allowUnsigned, "allow-unsigned-modules", false, "install modules without a signature even with a trust store (development only)")
	fs.DurationVar(&o.captureRetention, "capture-retention", app.DefaultCaptureRetention, "how long the controller keeps a stored capture file, as a Go `duration` such as 168h")
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
	if o.set["enroll-listen"] {
		cfg.EnrollListen = o.enrollListen
	}
	if o.set["data-dir"] {
		cfg.DataDir = o.dataDir
	}
	if o.set["pki-dir"] {
		cfg.PKIDir = o.pkiDir
	}
	if o.set["tls-names"] {
		cfg.TLSNames = splitNames(o.tlsNames)
	}
	if o.set["admin-socket"] {
		cfg.AdminSocket = o.adminSocket
	}
	if o.set["insecure-plain-grpc"] {
		cfg.InsecurePlainGRPC = o.insecurePlain
	}
	if o.set["metrics-token-file"] {
		cfg.MetricsTokenFile = o.metricsTokenFile
	}
	if o.set["trust-store-file"] {
		cfg.TrustStoreFile = o.trustStoreFile
	}
	if o.set["allow-unsigned-modules"] {
		cfg.AllowUnsignedModules = o.allowUnsigned
	}
	if o.set["capture-retention"] {
		cfg.CaptureRetention = app.Duration(o.captureRetention)
	}
	if o.set["log-level"] {
		cfg.LogLevel = o.logLevel
	}
	if err := cfg.Validate(); err != nil {
		return app.Config{}, err
	}
	return cfg, nil
}

// splitNames splits a comma separated list and drops empty items.
func splitNames(list string) []string {
	var out []string
	for _, n := range strings.Split(list, ",") {
		if n = strings.TrimSpace(n); n != "" {
			out = append(out, n)
		}
	}
	return out
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
	if len(args) > 0 && (args[0] == cmdToken || args[0] == cmdRevoke) {
		return runAdmin(ctx, args[0], args[1:], stdout, stderr)
	}
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
	if err := mountMetrics(a, cfg, ctl, logger); err != nil {
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
	eln, err := ctl.ListenEnroll(ctx)
	if err != nil {
		return errors.Join(err, ln.Close(), gln.Close())
	}
	aln, err := ctl.ListenAdmin(ctx)
	if err != nil {
		return errors.Join(err, ln.Close(), gln.Close(), closeListener(eln))
	}

	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	grpcDone := make(chan error, 1)
	go func() {
		err := ctl.Run(runCtx, gln)
		cancel() // a failing agent service stops the REST server too
		grpcDone <- err
	}()
	var pkiDone chan error
	if eln != nil || aln != nil {
		pkiDone = make(chan error, 1)
		go func() {
			err := ctl.RunPKI(runCtx, eln, aln)
			cancel()
			pkiDone <- err
		}()
	}
	httpErr := a.Run(runCtx, ln)
	cancel()
	err = errors.Join(httpErr, <-grpcDone)
	if pkiDone != nil {
		err = errors.Join(err, <-pkiDone)
	}
	return err
}

// closeListener closes ln when it is not nil.
func closeListener(ln net.Listener) error {
	if ln == nil {
		return nil
	}
	return ln.Close()
}

// mountMetrics creates the metrics aggregator, feeds it every agent manager
// event, routes the run metrics operations to it, builds run reports over its
// merged metrics with the criteria evaluator of the controller, and serves
// the Prometheus exposition at GET /metrics. The endpoint is open unless
// metrics_token_file names a token, which scrapers then send as a bearer
// token. Call it before ctl.Mount.
func mountMetrics(a *app.App, cfg app.Config, ctl *wiring.Controller, logger *slog.Logger) error {
	agg, err := metrics.New(metrics.Config{Logger: logger})
	if err != nil {
		return fmt.Errorf("create metrics aggregator: %w", err)
	}
	ctl.Observe(agg.Observe)
	ctl.SetMetrics(metrics.NewAPI(agg, ctl.Runs))
	ctl.SetReport(report.NewAPI(ctl.Evaluator, ctl.Runs, agg))
	token, err := cfg.LoadMetricsToken()
	if err != nil {
		return err
	}
	if token == "" {
		logger.Info("metrics endpoint is open", "path", "/metrics")
	} else {
		logger.Info("metrics endpoint requires a bearer token", "path", "/metrics")
	}
	a.Handle("GET /metrics", app.RequireBearer(token, metrics.NewPrometheus(agg)))
	return nil
}
