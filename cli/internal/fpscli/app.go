// SPDX-License-Identifier: GPL-3.0-only

// Package fpscli implements the fps command line client of the controller REST
// API in contracts/openapi/fps-api-v1.yaml. It uses the standard library only.
package fpscli

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"strings"
	"time"
)

// Exit codes of the fps command. docs/wiki/CLI.md documents them.
const (
	// ExitOK means the command succeeded and, for a command that waits for a
	// run, that the run completed and its verdict passed.
	ExitOK = 0
	// ExitFailed means a negative result: the configuration is invalid, or the
	// run failed, was aborted, or did not pass its criteria.
	ExitFailed = 1
	// ExitUsage means a bad command line.
	ExitUsage = 2
	// ExitAuth means the controller answered 401 or 403.
	ExitAuth = 3
	// ExitNotFound means the controller answered 404.
	ExitNotFound = 4
	// ExitAPI means any other error response of the controller.
	ExitAPI = 5
	// ExitNetwork means the client could not reach the controller, for example
	// a refused connection, a TLS failure, or a timeout.
	ExitNetwork = 6
	// ExitInterrupted means SIGINT or SIGTERM ended the command.
	ExitInterrupted = 130
)

// Defaults of the global settings.
const (
	defaultURL     = "https://localhost:8443"
	defaultTimeout = 30 * time.Second
)

// Env carries the process environment of a command, so tests run it without
// touching the real one.
type Env struct {
	Stdin  io.Reader
	Stdout io.Writer
	Stderr io.Writer
	// Getenv reads an environment variable. It defaults to os.Getenv.
	Getenv func(string) string
	// IsTerminal reports whether w is an interactive terminal. It defaults to a
	// check of the character device mode of an *os.File.
	IsTerminal func(w io.Writer) bool
	// Now returns the current time. It defaults to time.Now.
	Now func() time.Time
}

// Globals are the settings that every command shares. Each can also appear
// after the command name.
type Globals struct {
	URL          string
	TokenFile    string
	CAFile       string
	JSON         bool
	Timeout      time.Duration
	InsecureHTTP bool
	HistoryFile  string
}

// app is one invocation of the command.
type app struct {
	env Env
	g   Globals
}

// usageError marks a bad command line.
type usageError struct{ msg string }

func (e *usageError) Error() string { return e.msg }

func usagef(format string, args ...any) error {
	return &usageError{msg: fmt.Sprintf(format, args...)}
}

// outcomeError marks a negative result. The command printed its details, so
// msg may be empty.
type outcomeError struct{ msg string }

func (e *outcomeError) Error() string { return e.msg }

// Execute runs the command line args, which exclude the program name, and
// returns the exit code.
func Execute(ctx context.Context, args []string, env Env) int {
	if env.Getenv == nil {
		env.Getenv = os.Getenv
	}
	if env.IsTerminal == nil {
		env.IsTerminal = isTerminal
	}
	if env.Now == nil {
		env.Now = time.Now
	}
	if env.Stdin == nil {
		env.Stdin = strings.NewReader("")
	}
	a := &app{env: env}
	a.g = Globals{
		URL:          firstNonEmpty(env.Getenv("FPS_URL"), defaultURL),
		TokenFile:    "",
		CAFile:       env.Getenv("FPS_CA_FILE"),
		JSON:         truthy(env.Getenv("FPS_JSON")),
		Timeout:      defaultTimeout,
		InsecureHTTP: truthy(env.Getenv("FPS_INSECURE_HTTP")),
		HistoryFile:  env.Getenv("FPS_HISTORY_FILE"),
	}
	err := a.dispatch(ctx, args)
	return a.exitCode(ctx, err)
}

// exitCode prints err and maps it to an exit code.
func (a *app) exitCode(ctx context.Context, err error) int {
	if err == nil {
		return ExitOK
	}
	if errors.Is(err, flag.ErrHelp) {
		return ExitOK
	}
	var out *outcomeError
	var use *usageError
	var apiErr *APIError
	var netErr *NetError
	switch {
	case errors.As(err, &out):
		if out.msg != "" {
			a.eprintf("fps: %s\n", out.msg)
		}
		return ExitFailed
	case errors.As(err, &use):
		a.eprintf("fps: %s\n", use.msg)
		a.eprintf("Run 'fps help' for usage.\n")
		return ExitUsage
	case ctx.Err() != nil && (errors.Is(err, context.Canceled) || errors.As(err, &netErr)):
		a.eprintf("fps: interrupted\n")
		return ExitInterrupted
	case errors.As(err, &apiErr):
		a.eprintf("fps: %s\n", apiErr.Error())
		apiErr.printFindings(a.env.Stderr)
		if hint := apiErr.hint(); hint != "" {
			a.eprintf("fps: %s\n", hint)
		}
		return apiErr.exitCode()
	case errors.As(err, &netErr):
		a.eprintf("fps: %s\n", netErr.Error())
		return ExitNetwork
	default:
		a.eprintf("fps: %v\n", err)
		return ExitFailed
	}
}

func (a *app) eprintf(format string, args ...any) {
	_, _ = fmt.Fprintf(a.env.Stderr, format, args...)
}

func (a *app) printf(format string, args ...any) {
	_, _ = fmt.Fprintf(a.env.Stdout, format, args...)
}

// registerGlobals adds the global flags to fs. The values that g holds at the
// time become the defaults, so a flag before the command name stays in force
// when the command's own flag set registers the same flags.
func (a *app) registerGlobals(fs *flag.FlagSet) {
	g := &a.g
	fs.StringVar(&g.URL, "url", g.URL, "controller base URL (env FPS_URL)")
	fs.StringVar(&g.TokenFile, "token-file", g.TokenFile, "file that holds the API token (env FPS_TOKEN_FILE, or FPS_TOKEN for the token itself)")
	fs.StringVar(&g.CAFile, "ca-file", g.CAFile, "PEM file of CA certificates to trust for HTTPS (env FPS_CA_FILE)")
	fs.BoolVar(&g.JSON, "json", g.JSON, "print JSON instead of text (env FPS_JSON)")
	fs.DurationVar(&g.Timeout, "timeout", g.Timeout, "timeout of one request, not of a metrics stream")
	fs.BoolVar(&g.InsecureHTTP, "insecure-http", g.InsecureHTTP, "send the token over plain http to a non loopback host (env FPS_INSECURE_HTTP)")
	fs.StringVar(&g.HistoryFile, "history-file", g.HistoryFile, "file that lists the configurations uploaded from this machine (env FPS_HISTORY_FILE)")
}

// newFlagSet returns a flag set with the global flags for the named command.
func (a *app) newFlagSet(name string) *flag.FlagSet {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	fs.SetOutput(a.env.Stderr)
	a.registerGlobals(fs)
	return fs
}

// parse parses args with fs and lets flags and positional arguments mix, so
// both `fps run stop r1 --force` and `fps run stop --force r1` work. A bare
// double dash ends the flags.
func parse(fs *flag.FlagSet, args []string) ([]string, error) {
	var pos []string
	for len(args) > 0 {
		if err := fs.Parse(args); err != nil {
			if errors.Is(err, flag.ErrHelp) {
				return nil, err
			}
			return nil, &usageError{msg: strings.TrimSpace(err.Error())}
		}
		rest := fs.Args()
		consumed := len(args) - len(rest)
		if consumed > 0 && args[consumed-1] == "--" {
			pos = append(pos, rest...)
			return pos, nil
		}
		if len(rest) == 0 {
			break
		}
		pos = append(pos, rest[0])
		args = rest[1:]
	}
	return pos, nil
}

const usageText = `fps is the command line client of the FixingPointSystem controller.

Usage:
  fps [global flags] <command> [arguments]

Commands:
  config validate [--id ID] [FILE|-]   upload a test configuration and validate it, or validate a stored one
  config upload FILE|-                 store a test configuration
  config list                          list the stored configurations, newest first
  config show ID                       print a stored configuration
  run start CONFIG_ID                  start a run
  run list                             list runs, newest first
  run show ID                          show a run, its totals and its verdict
  run stop ID [--force]                stop a run
  watch RUN_ID                         follow the live metrics of a run
  report RUN_ID [--format FORMAT]      print the report of a run
  help                                 print this text

Global flags (also accepted after the command name):
  --url URL           controller base URL            env FPS_URL            default https://localhost:8443
  --token-file FILE   file that holds the API token  env FPS_TOKEN_FILE
                      the token itself               env FPS_TOKEN
  --ca-file FILE      PEM CA certificates for HTTPS  env FPS_CA_FILE
  --json              print JSON, one document (or one line per event for watch)  env FPS_JSON
  --timeout DURATION  timeout of one request         default 30s
  --insecure-http     allow the token over plain http to a non loopback host
  --history-file FILE list of uploaded configurations env FPS_HISTORY_FILE

Exit codes:
  0    success; a command that waits for a run exits 0 when it completed and passed
  1    negative result: invalid configuration, or a run that failed, aborted, or missed its criteria
  2    usage error
  3    authentication or permission error (HTTP 401 or 403)
  4    not found (HTTP 404)
  5    any other error response of the controller
  6    controller unreachable, TLS failure, or timeout
  130  interrupted

The token never appears on the command line or in any output.
Use 'fps <command> -h' for the flags of one command.
`

// dispatch routes args to a command.
func (a *app) dispatch(ctx context.Context, args []string) error {
	top := flag.NewFlagSet("fps", flag.ContinueOnError)
	top.SetOutput(io.Discard)
	a.registerGlobals(top)
	// Only the flags before the command name belong to the top level, so this
	// stops at the first positional argument.
	if err := top.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			a.printf("%s", usageText)
			return nil
		}
		return &usageError{msg: strings.TrimSpace(err.Error())}
	}
	rest := top.Args()
	if len(rest) == 0 {
		a.eprintf("%s", usageText)
		return usagef("no command given")
	}
	cmd, rest := rest[0], rest[1:]
	switch cmd {
	case "help", "-h", "--help":
		a.printf("%s", usageText)
		return nil
	case "config":
		return a.configCmd(ctx, rest)
	case "run":
		return a.runCmd(ctx, rest)
	case "watch":
		return a.watchCmd(ctx, rest)
	case "report":
		return a.reportCmd(ctx, rest)
	default:
		return usagef("unknown command %q", cmd)
	}
}

// subcommand splits the subcommand off args of a group command.
func subcommand(group string, args []string, known ...string) (string, []string, error) {
	if len(args) == 0 {
		return "", nil, usagef("fps %s needs a subcommand: %s", group, strings.Join(known, ", "))
	}
	for _, k := range known {
		if args[0] == k {
			return k, args[1:], nil
		}
	}
	return "", nil, usagef("unknown subcommand %q of fps %s, want one of %s", args[0], group, strings.Join(known, ", "))
}

// wantArgs checks the number of positional arguments.
func wantArgs(cmd string, pos []string, names ...string) error {
	if len(pos) < len(names) {
		return usagef("fps %s needs %s", cmd, strings.Join(names, " "))
	}
	if len(pos) > len(names) {
		return usagef("fps %s takes %s only, got extra argument %q", cmd, strings.Join(names, " "), pos[len(names)])
	}
	return nil
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if v != "" {
			return v
		}
	}
	return ""
}

func truthy(s string) bool {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "1", "true", "yes", "on":
		return true
	}
	return false
}

func isTerminal(w io.Writer) bool {
	f, ok := w.(*os.File)
	if !ok {
		return false
	}
	st, err := f.Stat()
	return err == nil && st.Mode()&os.ModeCharDevice != 0
}
