// SPDX-License-Identifier: GPL-3.0-only

package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/wiring"
)

// The two admin subcommands. They never open the PKI directory themselves.
// They ask the running controller through its admin socket, so the controller
// stays the only writer of the PKI state.
const (
	cmdToken  = "token"
	cmdRevoke = "revoke"
)

// runAdmin runs the token or the revoke subcommand and returns the exit code.
//
// token prints the enrollment token on standard output and nothing else, so
// that a script can capture it, and prints the expiry on standard error. The
// token is a secret: hand it to the agent through a file or the environment of
// the agent, never through a command line or a log.
func runAdmin(ctx context.Context, cmd string, args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("fps-controller "+cmd, flag.ContinueOnError)
	fs.SetOutput(stderr)
	var (
		o     options
		id    string
		ttl   time.Duration
		extra = cmd == cmdToken
	)
	fs.StringVar(&id, "agent-id", "", "identity of the agent")
	if extra {
		fs.DurationVar(&ttl, "ttl", 0, "validity of the token, for example 30m; zero takes the default of one hour")
	}
	fs.StringVar(&o.configPath, "config", "", "path of the YAML configuration `file`, to find the admin socket")
	fs.StringVar(&o.dataDir, "data-dir", "", "`directory` of the controller state, to find the admin socket")
	fs.StringVar(&o.pkiDir, "pki-dir", "", "`directory` of the controller CA, to find the admin socket")
	fs.StringVar(&o.adminSocket, "admin-socket", "", "unix socket `path` of the running controller")
	if err := fs.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return exitOK
		}
		return exitUsage
	}
	if fs.NArg() > 0 {
		_, _ = fmt.Fprintf(stderr, "fps-controller %s: unexpected argument %q\n", cmd, fs.Arg(0))
		return exitUsage
	}
	if id == "" {
		_, _ = fmt.Fprintf(stderr, "fps-controller %s: -agent-id is required\n", cmd)
		return exitUsage
	}
	o.set = make(map[string]bool)
	fs.Visit(func(f *flag.Flag) { o.set[f.Name] = true })
	cfg, err := loadConfig(o)
	if err != nil {
		_, _ = fmt.Fprintln(stderr, "fps-controller:", err)
		return exitUsage
	}
	socket := cfg.ResolvedAdminSocket()
	if socket == "" {
		_, _ = fmt.Fprintf(stderr, "fps-controller %s: no admin socket; set -admin-socket, -pki-dir, -data-dir, or a -config file that sets one\n", cmd)
		return exitUsage
	}

	client := wiring.AdminClient{Socket: socket}
	switch cmd {
	case cmdToken:
		out, err := client.MintToken(ctx, id, ttl)
		if err != nil {
			return adminFailure(stderr, cmd, err)
		}
		_, _ = fmt.Fprintln(stdout, out.Token)
		_, _ = fmt.Fprintf(stderr, "enrollment token for agent %s, valid until %s, usable once\n", out.AgentID, out.Expires.UTC().Format(time.RFC3339))
	default:
		out, err := client.Revoke(ctx, id)
		if err != nil {
			return adminFailure(stderr, cmd, err)
		}
		_, _ = fmt.Fprintf(stdout, "revoked %d certificate(s) of agent %s and ended its session\n", out.Certificates, out.AgentID)
	}
	return exitOK
}

func adminFailure(stderr io.Writer, cmd string, err error) int {
	_, _ = fmt.Fprintf(stderr, "fps-controller %s: %v\n", cmd, err)
	if errors.Is(err, wiring.ErrControllerNotRunning) {
		_, _ = fmt.Fprintln(stderr, "start the controller first: it is the only process that writes the PKI state")
	}
	return exitFailure
}
