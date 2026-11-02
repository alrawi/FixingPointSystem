// SPDX-License-Identifier: GPL-3.0-only

// Command fps-controller runs the FixingPointSystem controller.
//
// Usage:
//
//	fps-controller [-config file] [-listen host:port] [-log-level level]
//	fps-controller -version
//
// Settings come from three layers, where a later layer wins. The layers are the
// built in defaults, the YAML file named by -config, and the flags that appear on
// the command line. The process logs JSON lines to standard error, serves
// GET /healthz, and on SIGINT or SIGTERM drains in flight requests for up to
// shutdown_timeout before it exits.
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
	"os"
	"os/signal"
	"runtime/debug"
	"syscall"

	"github.com/alrawi/FixingPointSystem/controller/internal/app"
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

	a := app.New(cfg, logger)
	ln, err := a.Listen(ctx)
	if err != nil {
		logger.Error("controller failed", "error", err)
		return exitFailure
	}
	if err := a.Run(ctx, ln); err != nil {
		logger.Error("controller failed", "error", err)
		return exitFailure
	}
	return exitOK
}
