// SPDX-License-Identifier: GPL-3.0-only

// Package fakeagent provides fake agents that dial a ControllerService in
// tests. A fake agent registers, holds a Session stream, answers prepare,
// start, and stop commands by the agent state machine of the contract, and
// can finish a run, fail, crash, or miss heartbeats on command. Serve starts a
// ControllerService on an in-memory listener for one test.
//
//	mgr, _ := agents.New(agents.Config{Identity: agents.ClaimedIdentity, AutoEnroll: true})
//	dial := fakeagent.Serve(t, mgr)
//	a, err := fakeagent.Start(ctx, fakeagent.Options{ID: "agent-a", Dial: dial})
//
// A fake agent also takes UpdateRates in AGENT_STATE_RUNNING, all entries or
// none, and keeps the session rate that it runs (Agent.Rates). With
// Options.Traffic it simulates sessions at that rate and reports their bytes,
// so a calibration loop closes against a real Manager.
//
// The fake agent reads the wall clock only for its agent_time fields and for
// automatic heartbeats, which Options.AutoHeartbeat turns on.
package fakeagent

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
	"google.golang.org/grpc/test/bufconn"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/timestamppb"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// Dialer opens a client connection to the controller.
type Dialer func(ctx context.Context) (*grpc.ClientConn, error)

// Serve serves srv over an in-memory listener until the test ends and returns
// a Dialer for it.
func Serve(tb testing.TB, srv agentpb.ControllerServiceServer) Dialer {
	tb.Helper()
	lis := bufconn.Listen(1 << 20)
	gs := grpc.NewServer()
	agentpb.RegisterControllerServiceServer(gs, srv)
	go func() { _ = gs.Serve(lis) }()
	tb.Cleanup(gs.Stop)
	return func(context.Context) (*grpc.ClientConn, error) {
		return grpc.NewClient("passthrough:///fakeagent",
			grpc.WithContextDialer(func(ctx context.Context, _ string) (net.Conn, error) { return lis.DialContext(ctx) }),
			grpc.WithTransportCredentials(insecure.NewCredentials()))
	}
}

// DefaultCapabilities returns the capabilities of a fake agent with two cores,
// the kernel backend, one port eth1, and engine contract 0.1.0.
func DefaultCapabilities() *agentpb.Capabilities {
	return &agentpb.Capabilities{
		AgentVersion: "fake-0.0.0",
		EngineApi:    "0.1.0",
		Backends:     []agentpb.Backend{agentpb.Backend_BACKEND_KERNEL},
		Cores:        2,
		MemoryBytes:  1 << 30,
		Ports:        []*agentpb.PortInfo{{Name: "eth1", Driver: "veth", SpeedMbps: 10000, Mac: "02:00:00:00:00:01", Mtu: 1500}},
	}
}

// Options configures a fake agent.
type Options struct {
	// ID is the agent identity.
	ID string
	// Dial opens the connection to the controller.
	Dial Dialer
	// Capabilities defaults to DefaultCapabilities.
	Capabilities *agentpb.Capabilities
	// AutoHeartbeat sends a heartbeat once per heartbeat interval of the
	// RegisterResponse. Without it, the test sends heartbeats with Heartbeat.
	AutoHeartbeat bool
	// OnCommand runs before the agent applies a command. A non-nil error
	// answers the command with it and leaves the state unchanged.
	OnCommand func(*agentpb.SessionResponse) *agentpb.CommandError
	// Traffic turns on the traffic simulation, which reports counters while the
	// agent runs and follows UpdateRates. Without it, the agent reports no
	// counters of its own.
	Traffic *Traffic
}

// Errors of the fake agent.
var (
	// ErrMissingHeartbeats reports a Heartbeat call while MissHeartbeats holds.
	ErrMissingHeartbeats = errors.New("fake agent misses heartbeats")
	// ErrDisconnected reports a call without an open session.
	ErrDisconnected = errors.New("fake agent disconnected")
	// ErrNotRunning reports Finish outside AGENT_STATE_RUNNING.
	ErrNotRunning = errors.New("fake agent not running")
)

// Agent is one fake agent.
type Agent struct {
	opts Options

	mu       sync.Mutex
	state    agentpb.AgentState
	planID   string
	phase    string
	message  string
	hbSeq    uint64
	snapSeq  uint64
	missing  bool
	commands []*agentpb.SessionResponse
	acks     map[uint64]chan struct{}
	conn     *grpc.ClientConn
	cancel   context.CancelFunc
	stream   grpc.BidiStreamingClient[agentpb.SessionRequest, agentpb.SessionResponse]
	resp     *agentpb.RegisterResponse
	done     chan struct{}
	endErr   error
	sim      trafficSim

	sendMu sync.Mutex
}

// Start dials the controller, registers in AGENT_STATE_IDLE, opens the
// session, and sends a first heartbeat.
func Start(ctx context.Context, opts Options) (*Agent, error) {
	if opts.Capabilities == nil {
		opts.Capabilities = DefaultCapabilities()
	}
	a := &Agent{opts: opts, state: agentpb.AgentState_AGENT_STATE_IDLE, acks: map[uint64]chan struct{}{}}
	if err := a.connect(ctx); err != nil {
		return nil, err
	}
	return a, nil
}

// ID returns the agent identity.
func (a *Agent) ID() string { return a.opts.ID }

// Registration returns the latest RegisterResponse.
func (a *Agent) Registration() *agentpb.RegisterResponse {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.resp
}

// Reconnect dials again after a crash or a lost session, registers with the
// current state and plan, and opens a new session, as an agent does after a
// lost connection.
func (a *Agent) Reconnect(ctx context.Context) error {
	a.Crash()
	return a.connect(ctx)
}

func (a *Agent) connect(ctx context.Context) error {
	conn, err := a.opts.Dial(ctx)
	if err != nil {
		return fmt.Errorf("fake agent %s: dial: %w", a.opts.ID, err)
	}
	client := agentpb.NewControllerServiceClient(conn)
	a.mu.Lock()
	state, plan := a.state, a.planID
	a.mu.Unlock()
	caps, _ := proto.Clone(a.opts.Capabilities).(*agentpb.Capabilities)
	resp, err := client.Register(ctx, &agentpb.RegisterRequest{AgentId: a.opts.ID, Capabilities: caps, State: state, PlanId: plan})
	if err != nil {
		_ = conn.Close()
		return fmt.Errorf("fake agent %s: register: %w", a.opts.ID, err)
	}
	sctx, cancel := context.WithCancel(context.Background())
	stream, err := client.Session(sctx)
	if err != nil {
		cancel()
		_ = conn.Close()
		return fmt.Errorf("fake agent %s: session: %w", a.opts.ID, err)
	}
	done := make(chan struct{})
	a.mu.Lock()
	a.conn, a.cancel, a.stream, a.resp, a.done, a.endErr = conn, cancel, stream, resp, done, nil
	a.mu.Unlock()
	open := &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Open{Open: &agentpb.SessionOpen{AgentId: a.opts.ID, SessionToken: resp.GetSessionToken()}}}
	if err := a.send(stream, open); err != nil {
		cancel()
		_ = conn.Close()
		return fmt.Errorf("fake agent %s: open: %w", a.opts.ID, err)
	}
	go a.receive(stream, done)
	if a.opts.AutoHeartbeat {
		go a.beat(sctx, resp.GetHeartbeatInterval().AsDuration())
	}
	if err := a.Heartbeat(ctx); err != nil && !errors.Is(err, ErrMissingHeartbeats) {
		return err
	}
	return nil
}

func (a *Agent) send(stream grpc.BidiStreamingClient[agentpb.SessionRequest, agentpb.SessionResponse], msg *agentpb.SessionRequest) error {
	a.sendMu.Lock()
	defer a.sendMu.Unlock()
	return stream.Send(msg)
}

// current returns the open stream.
func (a *Agent) current() (grpc.BidiStreamingClient[agentpb.SessionRequest, agentpb.SessionResponse], chan struct{}, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.stream == nil {
		return nil, nil, ErrDisconnected
	}
	select {
	case <-a.done:
		return nil, nil, ErrDisconnected
	default:
	}
	return a.stream, a.done, nil
}

func (a *Agent) beat(ctx context.Context, every time.Duration) {
	if every <= 0 {
		return
	}
	t := time.NewTicker(every)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			_ = a.sendHeartbeat()
		}
	}
}

// heartbeatMsg returns the next heartbeat, or nil while heartbeats are missed.
func (a *Agent) heartbeatMsg() (*agentpb.SessionRequest, uint64) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.missing {
		return nil, 0
	}
	a.hbSeq++
	return &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Heartbeat{Heartbeat: &agentpb.Heartbeat{
		Sequence:  a.hbSeq,
		State:     a.state,
		PlanId:    a.planID,
		Phase:     a.phase,
		AgentTime: timestamppb.Now(),
		Message:   a.message,
	}}}, a.hbSeq
}

// sendHeartbeat sends one heartbeat without waiting for its acknowledgement.
func (a *Agent) sendHeartbeat() error {
	stream, _, err := a.current()
	if err != nil {
		return err
	}
	msg, _ := a.heartbeatMsg()
	if msg == nil {
		return ErrMissingHeartbeats
	}
	return a.send(stream, msg)
}

// Heartbeat sends one heartbeat and waits for its acknowledgement.
func (a *Agent) Heartbeat(ctx context.Context) error {
	stream, done, err := a.current()
	if err != nil {
		return err
	}
	msg, seq := a.heartbeatMsg()
	if msg == nil {
		return ErrMissingHeartbeats
	}
	ack := make(chan struct{})
	a.mu.Lock()
	a.acks[seq] = ack
	a.mu.Unlock()
	defer func() {
		a.mu.Lock()
		delete(a.acks, seq)
		a.mu.Unlock()
	}()
	if err := a.send(stream, msg); err != nil {
		return fmt.Errorf("fake agent %s: heartbeat %d: %w", a.opts.ID, seq, err)
	}
	select {
	case <-ack:
		return nil
	case <-done:
		return fmt.Errorf("fake agent %s: heartbeat %d: %w", a.opts.ID, seq, ErrDisconnected)
	case <-ctx.Done():
		return fmt.Errorf("fake agent %s: heartbeat %d: %w", a.opts.ID, seq, ctx.Err())
	}
}

// MissHeartbeats stops all heartbeats while the session stays open, so the
// controller sees a silent agent.
func (a *Agent) MissHeartbeats() {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.missing = true
}

// ResumeHeartbeats ends MissHeartbeats.
func (a *Agent) ResumeHeartbeats() {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.missing = false
}

// Crash closes the connection at once without a message, as a killed agent
// process does. The agent keeps its state for Reconnect.
func (a *Agent) Crash() {
	a.mu.Lock()
	conn, cancel := a.conn, a.cancel
	a.conn, a.cancel = nil, nil
	a.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	if conn != nil {
		_ = conn.Close()
	}
}

// Close ends the session cleanly and closes the connection.
func (a *Agent) Close() {
	a.mu.Lock()
	stream := a.stream
	a.mu.Unlock()
	if stream != nil {
		a.sendMu.Lock()
		_ = stream.CloseSend()
		a.sendMu.Unlock()
	}
	a.Crash()
}

// Done is closed when the current session ends.
func (a *Agent) Done() <-chan struct{} {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.done
}

// Err returns the status that ended the current session, or nil while it is open.
func (a *Agent) Err() error {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.endErr
}

// State returns the agent state and the loaded plan.
func (a *Agent) State() (agentpb.AgentState, string) {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.state, a.planID
}

// Commands returns copies of every command that the agent received.
func (a *Agent) Commands() []*agentpb.SessionResponse {
	a.mu.Lock()
	defer a.mu.Unlock()
	out := make([]*agentpb.SessionResponse, len(a.commands))
	for i, c := range a.commands {
		out[i], _ = proto.Clone(c).(*agentpb.SessionResponse)
	}
	return out
}

// Snapshot sends one counter snapshot of the loaded plan with the next
// sequence number and returns the sequence.
func (a *Agent) Snapshot(counters ...*agentpb.CounterValue) (uint64, error) {
	stream, _, err := a.current()
	if err != nil {
		return 0, err
	}
	snap := a.nextSnapshot(false, counters)
	return snap.GetSequence(), a.send(stream, &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Snapshot{Snapshot: snap}})
}

// SendSnapshot sends snap unchanged, for example to repeat a sequence number.
func (a *Agent) SendSnapshot(snap *agentpb.CounterSnapshot) error {
	stream, _, err := a.current()
	if err != nil {
		return err
	}
	return a.send(stream, &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Snapshot{Snapshot: snap}})
}

func (a *Agent) nextSnapshot(final bool, counters []*agentpb.CounterValue) *agentpb.CounterSnapshot {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.snapSeq++
	return &agentpb.CounterSnapshot{
		PlanId:    a.planID,
		Sequence:  a.snapSeq,
		Time:      timestamppb.Now(),
		Phase:     a.phase,
		Aggregate: true,
		IsFinal:   final,
		Counters:  counters,
	}
}

// Finish ends a running plan on its own, as an agent does after the last
// load phase. It sends a final snapshot and a heartbeat.
func (a *Agent) Finish() error {
	stream, _, err := a.current()
	if err != nil {
		return err
	}
	a.mu.Lock()
	if a.state != agentpb.AgentState_AGENT_STATE_RUNNING {
		a.mu.Unlock()
		return ErrNotRunning
	}
	a.mu.Unlock()
	a.StopTraffic()
	snap := a.nextSnapshot(true, nil)
	a.setState(agentpb.AgentState_AGENT_STATE_STOPPED, "", "")
	if err := a.send(stream, &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Snapshot{Snapshot: snap}}); err != nil {
		return err
	}
	return a.notify()
}

// Fail moves the agent to AGENT_STATE_FAILED with message and sends a heartbeat.
func (a *Agent) Fail(message string) error {
	a.setState(agentpb.AgentState_AGENT_STATE_FAILED, "", message)
	return a.notify()
}

// notify sends the heartbeat that follows a state change, unless heartbeats
// are missed.
func (a *Agent) notify() error {
	if err := a.sendHeartbeat(); err != nil && !errors.Is(err, ErrMissingHeartbeats) {
		return err
	}
	return nil
}

func (a *Agent) setState(s agentpb.AgentState, phase, message string) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.state, a.phase, a.message = s, phase, message
}

// receive reads the session until it ends.
func (a *Agent) receive(stream grpc.BidiStreamingClient[agentpb.SessionRequest, agentpb.SessionResponse], done chan struct{}) {
	for {
		msg, err := stream.Recv()
		if err != nil {
			a.mu.Lock()
			if errors.Is(err, io.EOF) {
				err = nil
			}
			if st, ok := status.FromError(err); ok && err != nil {
				err = st.Err()
			}
			if a.done == done {
				a.endErr = err
			}
			a.mu.Unlock()
			close(done)
			return
		}
		if ack := msg.GetHeartbeatAck(); ack != nil {
			a.mu.Lock()
			if ch, ok := a.acks[ack.GetSequence()]; ok {
				close(ch)
				delete(a.acks, ack.GetSequence())
			}
			a.mu.Unlock()
			continue
		}
		res := a.apply(msg)
		_ = a.send(stream, &agentpb.SessionRequest{Body: &agentpb.SessionRequest_Result{Result: res}})
	}
}

// apply runs one command by the agent state machine, sends the heartbeat of
// a state change, and returns the result.
func (a *Agent) apply(cmd *agentpb.SessionResponse) *agentpb.CommandResult {
	a.mu.Lock()
	cp, _ := proto.Clone(cmd).(*agentpb.SessionResponse)
	a.commands = append(a.commands, cp)
	state, plan := a.state, a.planID
	a.mu.Unlock()
	res := &agentpb.CommandResult{CommandId: cmd.GetCommandId()}
	fail := func(code agentpb.ErrorCode, format string, args ...any) *agentpb.CommandResult {
		res.Error = &agentpb.CommandError{Code: code, Message: fmt.Sprintf(format, args...)}
		return res
	}
	if a.opts.OnCommand != nil {
		if e := a.opts.OnCommand(cmd); e != nil {
			res.Error = e
			if e.GetCode() == agentpb.ErrorCode_ERROR_CODE_INTERNAL {
				a.setState(agentpb.AgentState_AGENT_STATE_FAILED, "", e.GetMessage())
				_ = a.notify()
			}
			return res
		}
	}
	switch body := cmd.GetBody().(type) {
	case *agentpb.SessionResponse_Prepare:
		switch state {
		case agentpb.AgentState_AGENT_STATE_IDLE, agentpb.AgentState_AGENT_STATE_STOPPED, agentpb.AgentState_AGENT_STATE_FAILED:
		default:
			return fail(agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION, "prepare in state %s", state)
		}
		id := body.Prepare.GetPlan().GetPlanId()
		a.mu.Lock()
		a.state, a.planID, a.phase, a.message, a.snapSeq = agentpb.AgentState_AGENT_STATE_PREPARED, id, "", "", 0
		a.mu.Unlock()
		a.sim.load(body.Prepare.GetPlan())
		_ = a.notify()
		res.Result = &agentpb.CommandResult_Prepare{Prepare: &agentpb.PrepareResult{PlanId: id, State: agentpb.AgentState_AGENT_STATE_PREPARED}}
	case *agentpb.SessionResponse_Start:
		if state != agentpb.AgentState_AGENT_STATE_PREPARED {
			return fail(agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION, "start in state %s", state)
		}
		if body.Start.GetPlanId() != plan {
			return fail(agentpb.ErrorCode_ERROR_CODE_NOT_FOUND, "plan %s is not loaded", body.Start.GetPlanId())
		}
		a.setState(agentpb.AgentState_AGENT_STATE_RUNNING, "steady", "")
		a.startTraffic()
		_ = a.notify()
		res.Result = &agentpb.CommandResult_Start{Start: &agentpb.StartResult{StartedAt: timestamppb.Now()}}
	case *agentpb.SessionResponse_Stop:
		if body.Stop.GetPlanId() != plan {
			return fail(agentpb.ErrorCode_ERROR_CODE_NOT_FOUND, "plan %s is not loaded", body.Stop.GetPlanId())
		}
		stop := &agentpb.StopResult{}
		switch state {
		case agentpb.AgentState_AGENT_STATE_PREPARED:
			a.mu.Lock()
			a.state, a.planID, a.phase = agentpb.AgentState_AGENT_STATE_IDLE, "", ""
			a.mu.Unlock()
			stop.State = agentpb.AgentState_AGENT_STATE_IDLE
		case agentpb.AgentState_AGENT_STATE_RUNNING, agentpb.AgentState_AGENT_STATE_STOPPING:
			a.StopTraffic()
			stop.FinalSnapshot = a.nextSnapshot(true, nil)
			a.setState(agentpb.AgentState_AGENT_STATE_STOPPED, "", "")
			stop.State = agentpb.AgentState_AGENT_STATE_STOPPED
		case agentpb.AgentState_AGENT_STATE_STOPPED:
			stop.State = agentpb.AgentState_AGENT_STATE_STOPPED
		default:
			return fail(agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION, "stop in state %s", state)
		}
		_ = a.notify()
		res.Result = &agentpb.CommandResult_Stop{Stop: stop}
	case *agentpb.SessionResponse_UpdateRates:
		if state != agentpb.AgentState_AGENT_STATE_RUNNING {
			return fail(agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION, "update rates in state %s", state)
		}
		if body.UpdateRates.GetPlanId() != plan {
			return fail(agentpb.ErrorCode_ERROR_CODE_NOT_FOUND, "plan %s is not loaded", body.UpdateRates.GetPlanId())
		}
		if err := a.sim.update(body.UpdateRates); err != nil {
			return fail(agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT, "update rates: %v", err)
		}
		res.Result = &agentpb.CommandResult_UpdateRates{UpdateRates: &agentpb.UpdateRatesResult{AppliedAt: timestamppb.Now()}}
	case *agentpb.SessionResponse_StartCapture, *agentpb.SessionResponse_StopCapture, *agentpb.SessionResponse_FetchCapture:
		return a.applyCapture(cmd, res, state, plan) // see capture.go
	default:
		return fail(agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT, "command %d has no body", cmd.GetCommandId())
	}
	return res
}
