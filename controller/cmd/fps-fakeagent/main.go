// SPDX-License-Identifier: GPL-3.0-only

// Command fps-fakeagent connects fake agents to a running controller. It is a
// thin wrapper around internal/fakeagent for tests of other modules, such as
// the end to end tests of the fps command line client in cli/, which cannot
// import the internal packages of the controller. It is not a product: the
// agents speak plain gRPC, so the controller must run with insecure_plain_grpc,
// and they simulate traffic instead of sending any.
//
//	fps-fakeagent -controller 127.0.0.1:8444 -agents local -ports lo -finish-after 3s
//
// Every agent registers in the idle state, follows the commands of the
// controller, and reports simulated counters while it runs a plan. With
// -finish-after an agent ends its plan on its own after that time, as an agent
// does after its last load phase, so a run completes without a stop request.
// After every agent has registered, the command prints the line
// "fake agents ready" on standard output. It runs until SIGINT or SIGTERM, or
// until the controller ends every session, and then exits with 0.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

func main() {
	os.Exit(run(os.Args[1:]))
}

// run returns the exit code: 0 after a clean end, 1 when an agent fails to
// register, and 2 for a bad command line.
func run(args []string) int {
	fs := flag.NewFlagSet("fps-fakeagent", flag.ContinueOnError)
	addr := fs.String("controller", "", "host:port of the agent gRPC service of the controller (required)")
	ids := fs.String("agents", "local", "comma separated agent identities, one fake agent each")
	ports := fs.String("ports", "lo", "comma separated port names that every agent reports")
	finishAfter := fs.Duration("finish-after", 0, "end the plan of an agent on its own after it ran this long, 0 never")
	bytesPer := fs.Float64("bytes-per-session", 1000, "payload bytes of one simulated session")
	speed := fs.Float64("speed", 1, "simulated seconds per real second")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	if *addr == "" || *bytesPer <= 0 || *speed <= 0 || *finishAfter < 0 {
		fmt.Fprintln(os.Stderr, "fps-fakeagent: -controller is required, and the byte, speed, and finish settings must be positive")
		return 2
	}
	var agentIDs, portNames []string
	for _, s := range strings.Split(*ids, ",") {
		if s = strings.TrimSpace(s); s != "" {
			agentIDs = append(agentIDs, s)
		}
	}
	for _, s := range strings.Split(*ports, ",") {
		if s = strings.TrimSpace(s); s != "" {
			portNames = append(portNames, s)
		}
	}
	if len(agentIDs) == 0 {
		fmt.Fprintln(os.Stderr, "fps-fakeagent: -agents names no agent")
		return 2
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	dial := func(context.Context) (*grpc.ClientConn, error) {
		return grpc.NewClient(*addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	}
	var agents []*fakeagent.Agent
	defer func() {
		for _, a := range agents {
			a.Close()
		}
	}()
	for _, id := range agentIDs {
		caps := fakeagent.DefaultCapabilities()
		caps.EngineApi = api.EngineAPIVersion
		caps.Ports = nil
		for _, p := range portNames {
			caps.Ports = append(caps.Ports, &agentpb.PortInfo{Name: p, Driver: "loopback", SpeedMbps: 10000, Mtu: 65536})
		}
		a, err := fakeagent.Start(ctx, fakeagent.Options{
			ID:            id,
			Capabilities:  caps,
			AutoHeartbeat: true,
			Dial:          dial,
			Traffic:       &fakeagent.Traffic{BytesPerSession: *bytesPer, Speed: *speed, Tick: 100 * time.Millisecond},
		})
		if err != nil {
			fmt.Fprintf(os.Stderr, "fps-fakeagent: agent %s: %v\n", id, err)
			return 1
		}
		agents = append(agents, a)
	}
	fmt.Println("fake agents ready")

	runningSince := make([]time.Time, len(agents))
	tick := time.NewTicker(50 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return 0
		case <-tick.C:
		}
		open := 0
		for i, a := range agents {
			select {
			case <-a.Done():
				continue
			default:
			}
			open++
			state, _ := a.State()
			if state != agentpb.AgentState_AGENT_STATE_RUNNING {
				runningSince[i] = time.Time{}
				continue
			}
			if runningSince[i].IsZero() {
				runningSince[i] = time.Now()
			}
			if *finishAfter > 0 && time.Since(runningSince[i]) >= *finishAfter {
				if err := a.Finish(); err != nil {
					fmt.Fprintf(os.Stderr, "fps-fakeagent: agent %s: finish: %v\n", a.ID(), err)
				}
				runningSince[i] = time.Time{}
			}
		}
		if open == 0 {
			return 0
		}
	}
}
