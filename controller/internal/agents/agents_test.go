// SPDX-License-Identifier: GPL-3.0-only

package agents_test

import (
	"context"
	"errors"
	"slices"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

// fakeClock is a Clock that moves only when a test advances it.
type fakeClock struct {
	mu sync.Mutex
	t  time.Time
}

func newFakeClock() *fakeClock {
	return &fakeClock{t: time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)}
}

func (c *fakeClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.t
}

func (c *fakeClock) Advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.t = c.t.Add(d)
}

// events records the events of a manager.
type events struct {
	mu  sync.Mutex
	evs []agents.Event
}

func (e *events) observe(ev agents.Event) {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.evs = append(e.evs, ev)
}

func (e *events) of(kind agents.EventKind, id string) []agents.Event {
	e.mu.Lock()
	defer e.mu.Unlock()
	var out []agents.Event
	for _, ev := range e.evs {
		if ev.Kind == kind && ev.AgentID == id {
			out = append(out, ev)
		}
	}
	return out
}

// harness is one manager on an in-memory listener with a fake clock.
type harness struct {
	mgr    *agents.Manager
	clock  *fakeClock
	events *events
	dial   fakeagent.Dialer
}

func newHarness(t *testing.T, mutate func(*agents.Config)) *harness {
	t.Helper()
	h := &harness{clock: newFakeClock(), events: &events{}}
	cfg := agents.Config{
		ControllerID:      "ctl-test",
		HeartbeatInterval: time.Second,
		HeartbeatTimeout:  3 * time.Second,
		Identity:          agents.ClaimedIdentity,
		Clock:             h.clock,
		Observer:          h.events.observe,
	}
	if mutate != nil {
		mutate(&cfg)
	}
	mgr, err := agents.New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	h.mgr = mgr
	h.dial = fakeagent.Serve(t, mgr)
	return h
}

func (h *harness) start(t *testing.T, opts fakeagent.Options) *fakeagent.Agent {
	t.Helper()
	opts.Dial = h.dial
	a, err := fakeagent.Start(testContext(t), opts)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	return a
}

func (h *harness) state(t *testing.T, id string) agents.Agent {
	t.Helper()
	ag, err := h.mgr.Get(id)
	if err != nil {
		t.Fatal(err)
	}
	return ag
}

func testContext(t *testing.T) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	return ctx
}

func waitDone(t *testing.T, a *fakeagent.Agent) {
	t.Helper()
	select {
	case <-a.Done():
	case <-time.After(5 * time.Second):
		t.Fatalf("session of %s did not end", a.ID())
	}
}

// waitState waits until the inventory shows agent id in state want, because
// the manager learns of a closed stream on its own goroutine.
// waitEvents polls until the observer received n events of kind for agent id.
func waitEvents(t *testing.T, h *harness, kind agents.EventKind, id string, n int) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for len(h.events.of(kind, id)) < n {
		if time.Now().After(deadline) {
			t.Fatalf("agent %s received %d events of kind %v, want %d", id, len(h.events.of(kind, id)), kind, n)
		}
		time.Sleep(time.Millisecond)
	}
}

func waitState(t *testing.T, h *harness, id string, want agents.State) agents.Agent {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for {
		ag := h.state(t, id)
		if ag.State == want {
			return ag
		}
		if time.Now().After(deadline) {
			t.Fatalf("agent %s is %s, want %s", id, ag, want)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func wantState(t *testing.T, h *harness, id string, want agents.State) agents.Agent {
	t.Helper()
	ag := h.state(t, id)
	if ag.State != want {
		t.Fatalf("agent %s is %s, want %s", id, ag, want)
	}
	return ag
}

func plan(id string) *agentpb.PrepareCommand {
	return &agentpb.PrepareCommand{Plan: &agentpb.Plan{PlanId: id, RunId: "run-1", AgentId: "a", EngineApi: "0.1.0"}}
}

// TestThreeAgents is the acceptance test of CTL-06. Three fake agents
// register, two are reserved, one is released, one runs a plan, and one
// misses its heartbeats, goes offline, and comes back.
func TestThreeAgents(t *testing.T) {
	h := newHarness(t, nil)
	ctx := testContext(t)
	ids := []string{"agent-a", "agent-b", "agent-c"}
	for _, id := range ids {
		if _, err := h.mgr.Enroll(id, map[string]string{"rack": "r1"}); err != nil {
			t.Fatal(err)
		}
	}
	for _, id := range ids {
		if got := wantState(t, h, id, agents.StateOffline); got.Capabilities != nil {
			t.Fatalf("enrolled agent %s has capabilities before it registers", id)
		}
	}

	// Registration.
	fa := map[string]*fakeagent.Agent{}
	for _, id := range ids {
		fa[id] = h.start(t, fakeagent.Options{ID: id})
	}
	list := h.mgr.List()
	if got := len(list); got != 3 {
		t.Fatalf("List returned %d agents, want 3", got)
	}
	for i, ag := range list {
		if ag.ID != ids[i] || ag.State != agents.StateOnline || ag.Address == "" || !ag.Enrolled {
			t.Fatalf("agent %d is %+v, want %s online with an address", i, ag, ids[i])
		}
		if ag.Capabilities.GetCores() != 2 || ag.Labels["rack"] != "r1" {
			t.Fatalf("agent %s lost its capabilities or labels: %+v", ag.ID, ag)
		}
		if n := len(h.events.of(agents.EventOnline, ag.ID)); n != 1 {
			t.Fatalf("agent %s has %d online events, want 1", ag.ID, n)
		}
	}
	reg := fa["agent-a"].Registration()
	if reg.GetControllerId() != "ctl-test" || reg.GetHeartbeatTimeout().AsDuration() != 3*time.Second || reg.GetSessionToken() == "" {
		t.Fatalf("RegisterResponse = %v", reg)
	}

	// Reservation.
	for _, id := range []string{"agent-a", "agent-b"} {
		ag, err := h.mgr.Reserve(id, "run-1")
		if err != nil {
			t.Fatal(err)
		}
		if ag.State != agents.StateReserved || ag.ReservedBy != "run-1" {
			t.Fatalf("reserved agent is %s", ag)
		}
	}
	if _, err := h.mgr.Reserve("agent-a", "run-1"); err != nil {
		t.Fatalf("a repeated reservation by the same holder failed: %v", err)
	}
	if _, err := h.mgr.Reserve("agent-a", "run-2"); !errors.Is(err, agents.ErrAgentReserved) {
		t.Fatalf("Reserve by another holder returned %v, want ErrAgentReserved", err)
	}

	// Release.
	if _, err := h.mgr.Release("agent-b", "run-2"); !errors.Is(err, agents.ErrAgentReserved) {
		t.Fatalf("Release by another holder returned %v, want ErrAgentReserved", err)
	}
	if ag, err := h.mgr.Release("agent-b", "run-1"); err != nil || ag.State != agents.StateOnline || ag.ReservedBy != "" {
		t.Fatalf("Release returned %s, %v", ag, err)
	}
	if _, err := h.mgr.Reserve("agent-b", "run-2"); err != nil {
		t.Fatalf("a released agent cannot be reserved again: %v", err)
	}
	if _, err := h.mgr.Release("agent-b", ""); err != nil {
		t.Fatal(err)
	}

	// A reserved agent runs a plan and finishes it on its own.
	res, err := h.mgr.Prepare(ctx, "agent-a", plan("run-1/agent-a"))
	if err != nil {
		t.Fatal(err)
	}
	if res.GetPlanId() != "run-1/agent-a" || res.GetState() != agentpb.AgentState_AGENT_STATE_PREPARED {
		t.Fatalf("PrepareResult = %v", res)
	}
	wantState(t, h, "agent-a", agents.StateBusy)
	if _, err := h.mgr.Reserve("agent-b", "run-1"); err != nil {
		t.Fatal(err)
	}
	if _, err := h.mgr.Start(ctx, "agent-a", &agentpb.StartCommand{PlanId: "run-1/agent-a"}); err != nil {
		t.Fatal(err)
	}
	if ag := wantState(t, h, "agent-a", agents.StateBusy); ag.AgentState != agentpb.AgentState_AGENT_STATE_RUNNING || ag.Phase != "steady" {
		t.Fatalf("running agent is %+v", ag)
	}
	if err := fa["agent-a"].Finish(); err != nil {
		t.Fatal(err)
	}
	if err := fa["agent-a"].Heartbeat(ctx); err != nil {
		t.Fatal(err)
	}
	if ag := wantState(t, h, "agent-a", agents.StateReserved); ag.AgentState != agentpb.AgentState_AGENT_STATE_STOPPED {
		t.Fatalf("finished agent is %s", ag)
	}
	if snaps := h.events.of(agents.EventSnapshot, "agent-a"); len(snaps) != 1 || !snaps[0].Snapshot.GetIsFinal() {
		t.Fatalf("finished agent sent snapshots %v, want one final snapshot", snaps)
	}
	cmds := fa["agent-a"].Commands()
	if len(cmds) != 2 || cmds[0].GetCommandId() != 1 || cmds[1].GetCommandId() != 2 {
		t.Fatalf("agent received commands %v, want ids 1 and 2", cmds)
	}

	// A missed heartbeat. agent-c is reserved and falls silent while the
	// other two keep beating.
	if _, err := h.mgr.Reserve("agent-c", "run-1"); err != nil {
		t.Fatal(err)
	}
	fa["agent-c"].MissHeartbeats()
	h.clock.Advance(2 * time.Second)
	for _, id := range []string{"agent-a", "agent-b"} {
		if err := fa[id].Heartbeat(ctx); err != nil {
			t.Fatal(err)
		}
	}
	if err := fa["agent-c"].Heartbeat(ctx); !errors.Is(err, fakeagent.ErrMissingHeartbeats) {
		t.Fatalf("Heartbeat while missing returned %v", err)
	}
	if got := h.mgr.Sweep(); len(got) != 0 {
		t.Fatalf("Sweep within the timeout marked %v offline", got)
	}
	h.clock.Advance(2 * time.Second)
	if got := h.mgr.Sweep(); !slices.Equal(got, []string{"agent-c"}) {
		t.Fatalf("Sweep marked %v offline, want [agent-c]", got)
	}
	ag := wantState(t, h, "agent-c", agents.StateOffline)
	if ag.ReservedBy != "run-1" || ag.Address != "" {
		t.Fatalf("offline agent is %+v, want its reservation kept and no address", ag)
	}
	wantState(t, h, "agent-a", agents.StateReserved)
	wantState(t, h, "agent-b", agents.StateReserved)
	waitDone(t, fa["agent-c"])
	if code := status.Code(fa["agent-c"].Err()); code != codes.DeadlineExceeded {
		t.Fatalf("silent agent session ended with %v, want DeadlineExceeded", fa["agent-c"].Err())
	}
	off := h.events.of(agents.EventOffline, "agent-c")
	if len(off) != 1 || off[0].Reason != agents.ReasonHeartbeatTimeout || !off[0].Time.Equal(h.clock.Now()) {
		t.Fatalf("offline events %v, want one heartbeat timeout", off)
	}
	if _, err := h.mgr.Prepare(ctx, "agent-c", plan("run-1/agent-c")); !errors.Is(err, agents.ErrAgentOffline) {
		t.Fatalf("Prepare on an offline agent returned %v", err)
	}
	if _, err := h.mgr.Reserve("agent-c", "run-2"); !errors.Is(err, agents.ErrAgentReserved) {
		t.Fatalf("Reserve of an offline agent of another holder returned %v", err)
	}

	// The agent dials again within the run and is back with its reservation.
	fa["agent-c"].ResumeHeartbeats()
	if err := fa["agent-c"].Reconnect(ctx); err != nil {
		t.Fatal(err)
	}
	wantState(t, h, "agent-c", agents.StateReserved)
	for _, id := range ids {
		if _, err := h.mgr.Release(id, "run-1"); err != nil {
			t.Fatal(err)
		}
		wantState(t, h, id, agents.StateOnline)
	}
}

func TestRegisterRejects(t *testing.T) {
	ctx := testContext(t)
	cases := []struct {
		name   string
		mutate func(*agents.Config)
		id     string
		caps   *agentpb.Capabilities
		code   codes.Code
	}{
		{"not enrolled", nil, "stranger", nil, codes.PermissionDenied},
		{"invalid id", func(c *agents.Config) { c.AutoEnroll = true }, "a/b", nil, codes.InvalidArgument},
		{"identity differs", func(c *agents.Config) {
			c.AutoEnroll = true
			c.Identity = func(context.Context, string) (string, error) { return "other", nil }
		}, "agent-a", nil, codes.Unauthenticated},
		{"no certificate", func(c *agents.Config) {
			c.AutoEnroll = true
			c.Identity = agents.PeerCertIdentity
		}, "agent-a", nil, codes.Unauthenticated},
		{"engine api", func(c *agents.Config) { c.AutoEnroll = true }, "agent-a", &agentpb.Capabilities{EngineApi: "9.0.0"}, codes.FailedPrecondition},
		{"no engine api", func(c *agents.Config) { c.AutoEnroll = true }, "agent-a", &agentpb.Capabilities{}, codes.FailedPrecondition},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			h := newHarness(t, tc.mutate)
			_, err := fakeagent.Start(ctx, fakeagent.Options{ID: tc.id, Dial: h.dial, Capabilities: tc.caps})
			if code := status.Code(errors.Unwrap(err)); code != tc.code {
				t.Fatalf("Start returned %v, want %v", err, tc.code)
			}
			if n := len(h.mgr.List()); n != 0 {
				t.Fatalf("a rejected agent entered the inventory: %v", h.mgr.List())
			}
		})
	}
}

func TestAutoEnroll(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	h.start(t, fakeagent.Options{ID: "agent-x"})
	if ag := wantState(t, h, "agent-x", agents.StateOnline); ag.Enrolled {
		t.Fatal("an auto enrolled agent reports Enrolled")
	}
}

// rawSession registers id and opens a session stream without the fake agent,
// so a test can send messages out of order.
func rawSession(t *testing.T, h *harness, id string) (agentpb.ControllerServiceClient, *agentpb.RegisterResponse) {
	t.Helper()
	conn, err := h.dial(testContext(t))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	client := agentpb.NewControllerServiceClient(conn)
	resp, err := client.Register(testContext(t), &agentpb.RegisterRequest{
		AgentId: id, Capabilities: fakeagent.DefaultCapabilities(), State: agentpb.AgentState_AGENT_STATE_IDLE,
	})
	if err != nil {
		t.Fatal(err)
	}
	return client, resp
}

func openMsg(id, token string) *agentpb.SessionRequest {
	return &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Open{Open: &agentpb.SessionOpen{AgentId: id, SessionToken: token}}}
}

func heartbeatMsg(seq uint64) *agentpb.SessionRequest {
	return &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Heartbeat{Heartbeat: &agentpb.Heartbeat{Sequence: seq, State: agentpb.AgentState_AGENT_STATE_IDLE}}}
}

func TestSessionRejects(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	client, resp := rawSession(t, h, "agent-a")
	cases := []struct {
		name string
		msgs []*agentpb.SessionRequest
	}{
		{"heartbeat first", []*agentpb.SessionRequest{heartbeatMsg(1)}},
		{"wrong token", []*agentpb.SessionRequest{openMsg("agent-a", "not-the-token")}},
		{"unknown agent", []*agentpb.SessionRequest{openMsg("agent-z", resp.GetSessionToken())}},
		{"repeated open", []*agentpb.SessionRequest{openMsg("agent-a", resp.GetSessionToken()), openMsg("agent-a", resp.GetSessionToken())}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			stream, err := client.Session(testContext(t))
			if err != nil {
				t.Fatal(err)
			}
			for _, m := range tc.msgs {
				if err := stream.Send(m); err != nil {
					t.Fatal(err)
				}
			}
			for {
				_, err := stream.Recv()
				if err == nil {
					continue
				}
				if code := status.Code(err); code != codes.Unauthenticated {
					t.Fatalf("session ended with %v, want Unauthenticated", err)
				}
				break
			}
		})
	}
}

func TestReregisterRevokesTokenAndReplacesSession(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	first := h.start(t, fakeagent.Options{ID: "agent-a"})
	oldToken := first.Registration().GetSessionToken()

	second := h.start(t, fakeagent.Options{ID: "agent-a"})
	if second.Registration().GetSessionToken() == oldToken {
		t.Fatal("a repeated Register returned the same token")
	}
	waitDone(t, first)
	if code := status.Code(first.Err()); code != codes.Aborted {
		t.Fatalf("the older session ended with %v, want Aborted", first.Err())
	}
	wantState(t, h, "agent-a", agents.StateOnline)
	if n := len(h.events.of(agents.EventOffline, "agent-a")); n != 0 {
		t.Fatalf("a replaced session produced %d offline events", n)
	}
	if err := second.Heartbeat(ctx); err != nil {
		t.Fatal(err)
	}

	client, _ := rawSession(t, h, "agent-b")
	stream, err := client.Session(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if err := stream.Send(openMsg("agent-a", oldToken)); err != nil {
		t.Fatal(err)
	}
	if _, err := stream.Recv(); status.Code(err) != codes.Unauthenticated {
		t.Fatalf("a revoked token opened a session: %v", err)
	}
}

func TestCrashMarksOfflineAndFailsPendingCommand(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	received := make(chan struct{})
	release := make(chan struct{})
	defer close(release)
	a := h.start(t, fakeagent.Options{ID: "agent-a", OnCommand: func(*agentpb.SessionResponse) *agentpb.CommandError {
		close(received)
		<-release
		return nil
	}})
	errc := make(chan error, 1)
	go func() {
		_, err := h.mgr.Prepare(ctx, "agent-a", plan("p1"))
		errc <- err
	}()
	<-received
	a.Crash()
	if err := <-errc; !errors.Is(err, agents.ErrSessionLost) {
		t.Fatalf("Prepare during a crash returned %v, want ErrSessionLost", err)
	}
	waitState(t, h, "agent-a", agents.StateOffline)
	// The manager marks the agent offline first and then delivers the event,
	// outside its lock, so the state does not tell that the event arrived.
	waitEvents(t, h, agents.EventOffline, "agent-a", 1)
	off := h.events.of(agents.EventOffline, "agent-a")
	if len(off) != 1 || off[0].Reason != agents.ReasonStreamClosed {
		t.Fatalf("offline events %v, want one stream closed", off)
	}
}

func TestCommandErrorAndContext(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	h.start(t, fakeagent.Options{ID: "agent-a", OnCommand: func(cmd *agentpb.SessionResponse) *agentpb.CommandError {
		if cmd.GetPrepare().GetPlan().GetPlanId() == "bad" {
			return &agentpb.CommandError{Code: agentpb.ErrorCode_ERROR_CODE_RESOURCE_EXHAUSTED, Message: "too many workers"}
		}
		return nil
	}})
	_, err := h.mgr.Prepare(ctx, "agent-a", plan("bad"))
	var ce *agents.CommandError
	if !errors.As(err, &ce) || ce.Code != agentpb.ErrorCode_ERROR_CODE_RESOURCE_EXHAUSTED || ce.AgentID != "agent-a" {
		t.Fatalf("Prepare returned %v, want a RESOURCE_EXHAUSTED CommandError", err)
	}
	wantState(t, h, "agent-a", agents.StateOnline)
	if _, err := h.mgr.Start(ctx, "agent-a", &agentpb.StartCommand{PlanId: "p"}); !errors.As(err, &ce) || ce.Code != agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION {
		t.Fatalf("Start in IDLE returned %v, want FAILED_PRECONDITION", err)
	}
	if _, err := h.mgr.Prepare(ctx, "nobody", plan("p")); !errors.Is(err, agents.ErrUnknownAgent) {
		t.Fatalf("Prepare to an unknown agent returned %v", err)
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if _, err := h.mgr.Prepare(cancelled, "agent-a", plan("p")); !errors.Is(err, context.Canceled) {
		t.Fatalf("Prepare with a cancelled context returned %v", err)
	}
	// Stop releases an unstarted plan.
	if _, err := h.mgr.Prepare(ctx, "agent-a", plan("p2")); err != nil {
		t.Fatal(err)
	}
	if _, err := h.mgr.Reserve("agent-a", "run-1"); !errors.Is(err, agents.ErrAgentBusy) {
		t.Fatalf("Reserve of an unreserved busy agent returned %v", err)
	}
	stop, err := h.mgr.Stop(ctx, "agent-a", &agentpb.StopCommand{PlanId: "p2"})
	if err != nil || stop.GetState() != agentpb.AgentState_AGENT_STATE_IDLE {
		t.Fatalf("Stop returned %v, %v", stop, err)
	}
	wantState(t, h, "agent-a", agents.StateOnline)
}

func TestFailAndStopRun(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	a := h.start(t, fakeagent.Options{ID: "agent-a"})
	if _, err := h.mgr.Prepare(ctx, "agent-a", plan("p")); err != nil {
		t.Fatal(err)
	}
	if _, err := h.mgr.Start(ctx, "agent-a", &agentpb.StartCommand{PlanId: "p"}); err != nil {
		t.Fatal(err)
	}
	stop, err := h.mgr.Stop(ctx, "agent-a", &agentpb.StopCommand{PlanId: "p", Mode: agentpb.StopMode_STOP_MODE_ABORT})
	if err != nil || stop.GetState() != agentpb.AgentState_AGENT_STATE_STOPPED || !stop.GetFinalSnapshot().GetIsFinal() {
		t.Fatalf("Stop returned %v, %v", stop, err)
	}
	if err := a.Fail("port eth1 went down"); err != nil {
		t.Fatal(err)
	}
	if err := a.Heartbeat(ctx); err != nil {
		t.Fatal(err)
	}
	ag := wantState(t, h, "agent-a", agents.StateOnline)
	if ag.AgentState != agentpb.AgentState_AGENT_STATE_FAILED || ag.Message != "port eth1 went down" {
		t.Fatalf("failed agent is %+v", ag)
	}
	changes := h.events.of(agents.EventStateChanged, "agent-a")
	var states []agentpb.AgentState
	for _, ev := range changes {
		states = append(states, ev.AgentState)
	}
	want := []agentpb.AgentState{
		agentpb.AgentState_AGENT_STATE_PREPARED, agentpb.AgentState_AGENT_STATE_RUNNING,
		agentpb.AgentState_AGENT_STATE_STOPPED, agentpb.AgentState_AGENT_STATE_FAILED,
	}
	if !slices.Equal(states, want) {
		t.Fatalf("state changes %v, want %v", states, want)
	}
}

func TestSnapshotsDropDuplicates(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	a := h.start(t, fakeagent.Options{ID: "agent-a"})
	if _, err := h.mgr.Prepare(ctx, "agent-a", plan("p")); err != nil {
		t.Fatal(err)
	}
	counter := &agentpb.CounterValue{Metric: "sessions.attempted", Value: &agentpb.CounterValue_Count{Count: 7}}
	for range 2 {
		if _, err := a.Snapshot(counter); err != nil {
			t.Fatal(err)
		}
	}
	if err := a.SendSnapshot(&agentpb.CounterSnapshot{PlanId: "p", Sequence: 2}); err != nil {
		t.Fatal(err)
	}
	// A heartbeat round trip orders the snapshots before the check.
	if err := a.Heartbeat(ctx); err != nil {
		t.Fatal(err)
	}
	snaps := h.events.of(agents.EventSnapshot, "agent-a")
	if len(snaps) != 2 || snaps[0].Snapshot.GetSequence() != 1 || snaps[1].Snapshot.GetSequence() != 2 {
		t.Fatalf("snapshot events %v, want sequences 1 and 2 once each", snaps)
	}
	if snaps[0].Snapshot.GetCounters()[0].GetCount() != 7 {
		t.Fatalf("snapshot lost its counter: %v", snaps[0].Snapshot)
	}
}

func TestShutdownEndsSessions(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	ctx := testContext(t)
	a := h.start(t, fakeagent.Options{ID: "agent-a"})
	h.mgr.Shutdown()
	waitDone(t, a)
	if code := status.Code(a.Err()); code != codes.Unavailable {
		t.Fatalf("session ended with %v, want Unavailable", a.Err())
	}
	wantState(t, h, "agent-a", agents.StateOffline)
	if err := a.Reconnect(ctx); status.Code(errors.Unwrap(err)) != codes.Unavailable {
		t.Fatalf("Register after Shutdown returned %v", err)
	}
	if _, err := h.mgr.Prepare(ctx, "agent-a", plan("p")); !errors.Is(err, agents.ErrManagerClosed) {
		t.Fatalf("Prepare after Shutdown returned %v", err)
	}
}

func TestAutoHeartbeatKeepsAgentOnline(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) {
		c.AutoEnroll = true
		c.Clock = agents.SystemClock{}
		c.HeartbeatInterval = 10 * time.Millisecond
		c.HeartbeatTimeout = 500 * time.Millisecond
	})
	ctx, cancel := context.WithCancel(testContext(t))
	runErr := make(chan error, 1)
	go func() { runErr <- h.mgr.Run(ctx) }()
	h.start(t, fakeagent.Options{ID: "agent-a", AutoHeartbeat: true})
	quiet := h.start(t, fakeagent.Options{ID: "agent-b"})
	quiet.MissHeartbeats()
	waitState(t, h, "agent-b", agents.StateOffline)
	wantState(t, h, "agent-a", agents.StateOnline)
	cancel()
	if err := <-runErr; !errors.Is(err, context.Canceled) {
		t.Fatalf("Run returned %v", err)
	}
}

func TestNewRejectsConfig(t *testing.T) {
	cases := []agents.Config{
		{HeartbeatInterval: time.Second, HeartbeatTimeout: time.Second},
		{HeartbeatInterval: -time.Second},
		{EngineAPIs: []string{"0.1.0"}},
		{EngineAPIs: []string{"latest"}},
	}
	for _, cfg := range cases {
		if _, err := agents.New(cfg); err == nil {
			t.Errorf("New accepted %+v", cfg)
		}
	}
	m, err := agents.New(agents.Config{})
	if err != nil {
		t.Fatal(err)
	}
	if m.HeartbeatTimeout() != agents.DefaultHeartbeatTimeout {
		t.Fatalf("default heartbeat timeout %s", m.HeartbeatTimeout())
	}
}

func TestEnrollAndLookupErrors(t *testing.T) {
	m, err := agents.New(agents.Config{})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := m.Enroll("bad id", nil); !errors.Is(err, agents.ErrInvalidID) {
		t.Fatalf("Enroll returned %v", err)
	}
	for _, f := range []func() error{
		func() error { _, err := m.Get("x"); return err },
		func() error { _, err := m.Reserve("x", "h"); return err },
		func() error { _, err := m.Release("x", ""); return err },
	} {
		if err := f(); !errors.Is(err, agents.ErrUnknownAgent) {
			t.Fatalf("lookup returned %v, want ErrUnknownAgent", err)
		}
	}
	if _, err := m.Enroll("x", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Reserve("x", ""); err == nil {
		t.Fatal("Reserve accepted an empty holder")
	}
	if _, err := m.Reserve("x", "h"); !errors.Is(err, agents.ErrAgentOffline) {
		t.Fatalf("Reserve of an offline agent returned %v", err)
	}
	if got := agents.EventSnapshot.String(); got != "snapshot" {
		t.Fatalf("EventSnapshot.String() = %q", got)
	}
}

// TestEndSessionEndsTheOpenSession covers the revocation hook: the session
// of the identity ends with UNAUTHENTICATED, the agent goes offline, and the
// observer sees the reason.
func TestEndSessionEndsTheOpenSession(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = true })
	a := h.start(t, fakeagent.Options{ID: "agent-a"})
	b := h.start(t, fakeagent.Options{ID: "agent-b"})
	waitState(t, h, "agent-a", agents.StateOnline)

	if h.mgr.EndSession("nobody") {
		t.Error("EndSession of an unknown identity = true, want false")
	}
	if !h.mgr.EndSession("agent-a") {
		t.Fatal("EndSession of an online agent = false, want true")
	}
	waitDone(t, a)
	if got := status.Code(a.Err()); got != codes.Unauthenticated {
		t.Errorf("session error code = %v (%v), want Unauthenticated", got, a.Err())
	}
	waitState(t, h, "agent-a", agents.StateOffline)
	waitEvents(t, h, agents.EventOffline, "agent-a", 1)
	if off := h.events.of(agents.EventOffline, "agent-a"); off[0].Reason != agents.ReasonRevoked {
		t.Errorf("offline reason = %q, want %q", off[0].Reason, agents.ReasonRevoked)
	}
	if h.mgr.EndSession("agent-a") {
		t.Error("a second EndSession = true, want false")
	}
	wantState(t, h, "agent-b", agents.StateOnline)
	select {
	case <-b.Done():
		t.Error("the session of another agent ended")
	default:
	}
}

// TestExpiredCertificateEndsAnIdleSession shows that a session outlives the
// handshake check only while the certificate lasts: a busy agent keeps its
// session past the expiry, and the sweep ends it once the agent is idle.
func TestExpiredCertificateEndsAnIdleSession(t *testing.T) {
	var expiry time.Time
	h := newHarness(t, func(c *agents.Config) {
		c.AutoEnroll = true
		expiry = c.Clock.Now().Add(10 * time.Minute)
		c.Expiry = func(context.Context) time.Time { return expiry }
	})
	ctx := testContext(t)
	a := h.start(t, fakeagent.Options{ID: "agent-a"})
	heartbeat := func() {
		t.Helper()
		if err := a.Heartbeat(ctx); err != nil {
			t.Fatal(err)
		}
	}

	h.clock.Advance(9 * time.Minute)
	heartbeat()
	if ids := h.mgr.Sweep(); len(ids) != 0 {
		t.Fatalf("Sweep before the expiry ended %v", ids)
	}
	if _, err := h.mgr.Prepare(ctx, "agent-a", plan("p")); err != nil {
		t.Fatal(err)
	}
	h.clock.Advance(2 * time.Minute)
	heartbeat()
	if ids := h.mgr.Sweep(); len(ids) != 0 {
		t.Fatalf("Sweep ended %v while the agent holds a plan", ids)
	}
	wantState(t, h, "agent-a", agents.StateBusy)
	if _, err := h.mgr.Stop(ctx, "agent-a", &agentpb.StopCommand{PlanId: "p"}); err != nil {
		t.Fatal(err)
	}
	heartbeat()
	if ids := h.mgr.Sweep(); !slices.Equal(ids, []string{"agent-a"}) {
		t.Fatalf("Sweep of an idle agent with an expired certificate ended %v, want agent-a", ids)
	}
	waitDone(t, a)
	if got := status.Code(a.Err()); got != codes.Unauthenticated {
		t.Errorf("session error code = %v (%v), want Unauthenticated", got, a.Err())
	}
	waitEvents(t, h, agents.EventOffline, "agent-a", 1)
	if off := h.events.of(agents.EventOffline, "agent-a"); off[0].Reason != agents.ReasonCertExpired {
		t.Errorf("offline reason = %q, want %q", off[0].Reason, agents.ReasonCertExpired)
	}
}

// TestExpiredCertificateIsRefused shows that Register refuses a certificate
// whose end has passed, even when the TLS layer let it through.
func TestExpiredCertificateIsRefused(t *testing.T) {
	h := newHarness(t, func(c *agents.Config) {
		c.AutoEnroll = true
		past := c.Clock.Now().Add(-time.Second)
		c.Expiry = func(context.Context) time.Time { return past }
	})
	_, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: "agent-a", Dial: h.dial})
	if got := status.Code(err); got != codes.Unauthenticated {
		t.Fatalf("Register with an expired certificate = %v, want Unauthenticated", err)
	}
	if _, err := h.mgr.Get("agent-a"); !errors.Is(err, agents.ErrUnknownAgent) {
		t.Errorf("the refused agent entered the inventory: %v", err)
	}
}
