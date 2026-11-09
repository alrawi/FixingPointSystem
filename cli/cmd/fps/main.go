// SPDX-License-Identifier: GPL-3.0-only

// Command fps is the command line client of the FixingPointSystem controller.
package main

import (
	"context"
	"os"
	"os/signal"
	"syscall"

	"github.com/alrawi/FixingPointSystem/cli/internal/fpscli"
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	code := fpscli.Execute(ctx, os.Args[1:], fpscli.Env{Stdin: os.Stdin, Stdout: os.Stdout, Stderr: os.Stderr})
	stop()
	os.Exit(code)
}
