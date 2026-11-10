// SPDX-License-Identifier: GPL-3.0-only

package agents_test

import (
	"context"
	"testing"
	"time"

	"google.golang.org/grpc"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// headerAgent is a session client that behaves like the Rust agent: it opens
// the session, waits for the response headers before it enters its loop, and
// only then heartbeats. tonic waits for the headers inside client.session(),
// and Stream.Header does the same here. The client of internal/fakeagent does
// not wait, so it never saw an idle session that the controller left silent
// (CTL-40, from the journals of IMG-07).
type headerAgent struct {
	conn   *grpc.ClientConn
	stream grpc.BidiStreamingClient[agentpb.SessionRequest, agentpb.SessionResponse]
	resp   *agentpb.RegisterResponse
	seq    uint64
}

// startHeaderAgent registers agent id and sends the open message of a session.
func startHeaderAgent(ctx context.Context, t *testing.T, h *harness, id string) *headerAgent {
	t.Helper()
	conn, err := h.dial(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	client := agentpb.NewControllerServiceClient(conn)
	resp, err := client.Register(ctx, &agentpb.RegisterRequest{
		AgentId:      id,
		Capabilities: &agentpb.Capabilities{AgentVersion: "header-test", EngineApi: "0.1.0", Cores: 1},
		State:        agentpb.AgentState_AGENT_STATE_IDLE,
	})
	if err != nil {
		t.Fatal(err)
	}
	sctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	stream, err := client.Session(sctx)
	if err != nil {
		t.Fatal(err)
	}
	open := &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Open{Open: &agentpb.SessionOpen{AgentId: id, SessionToken: resp.GetSessionToken()}}}
	if err := stream.Send(open); err != nil {
		t.Fatal(err)
	}
	return &headerAgent{conn: conn, stream: stream, resp: resp}
}

// waitHeaders waits up to d for the response headers and reports whether they
// arrived.
func (a *headerAgent) waitHeaders(d time.Duration) bool {
	got := make(chan error, 1)
	go func() {
		_, err := a.stream.Header()
		got <- err
	}()
	select {
	case err := <-got:
		return err == nil
	case <-time.After(d):
		return false
	}
}

// heartbeat sends one heartbeat and waits for its acknowledgement.
func (a *headerAgent) heartbeat() error {
	a.seq++
	hb := &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Heartbeat{Heartbeat: &agentpb.Heartbeat{
		Sequence: a.seq,
		State:    agentpb.AgentState_AGENT_STATE_IDLE,
	}}}
	if err := a.stream.Send(hb); err != nil {
		return err
	}
	for {
		msg, err := a.stream.Recv()
		if err != nil {
			return err
		}
		if ack := msg.GetHeartbeatAck(); ack != nil && ack.GetSequence() == a.seq {
			return nil
		}
	}
}

// TestIdleSessionSendsHeadersAtOnce is the acceptance test of CTL-40. An agent
// that waits for the response headers before it heartbeats opens a session on
// an idle controller. The controller must send the headers without a command or
// a heartbeat to answer, because otherwise the agent never heartbeats and the
// sweep ends the session within one heartbeat timeout. The test then holds the
// session open for three heartbeat timeouts, one heartbeat per interval.
func TestIdleSessionSendsHeadersAtOnce(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	a := startHeaderAgent(ctx, t, h, "agent-h")
	waitEvents(t, h, agents.EventOnline, "agent-h", 1)

	if !a.waitHeaders(2 * time.Second) {
		// Show the consequence for the controller: the silent agent is
		// ended by the next sweep after one heartbeat timeout.
		h.clock.Advance(h.mgr.HeartbeatTimeout() + time.Second)
		ended := h.mgr.Sweep()
		t.Fatalf("the controller sent no response headers within 2 s of the open message, so an agent that waits for them never heartbeats; Sweep after one heartbeat timeout ended %v", ended)
	}

	interval := h.mgr.HeartbeatTimeout() / 3
	steps := 3 * int(h.mgr.HeartbeatTimeout()/interval)
	for i := 1; i <= steps; i++ {
		h.clock.Advance(interval)
		if err := a.heartbeat(); err != nil {
			t.Fatalf("heartbeat %d: %v", i, err)
		}
		if ended := h.mgr.Sweep(); len(ended) != 0 {
			t.Fatalf("Sweep after %v of an idle session ended %v", time.Duration(i)*interval, ended)
		}
	}
	wantState(t, h, "agent-h", agents.StateOnline)
}
