// SPDX-License-Identifier: GPL-3.0-only

package agents

import (
	"context"
	"crypto/subtle"
	"errors"
	"fmt"
	"io"
	"sync"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/peer"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/durationpb"
	"google.golang.org/protobuf/types/known/timestamppb"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// Register implements ControllerService.Register. It checks the identity,
// the enrollment, and the engine contract version, stores the reported
// capabilities and state, and returns a new session token that revokes the
// previous one.
func (m *Manager) Register(ctx context.Context, req *agentpb.RegisterRequest) (*agentpb.RegisterResponse, error) {
	id := req.GetAgentId()
	if !ValidID(id) {
		return nil, status.Errorf(codes.InvalidArgument, "agent_id %q does not match %s", id, idPattern)
	}
	identity, err := m.cfg.Identity(ctx, id)
	if err != nil {
		return nil, status.Errorf(codes.Unauthenticated, "agent identity: %v", err)
	}
	if identity != id {
		return nil, status.Errorf(codes.Unauthenticated, "agent_id %s differs from the certificate identity %s", id, identity)
	}
	if err := m.checkExpiry(m.cfg.Expiry(ctx)); err != nil {
		return nil, err
	}
	token, err := m.newToken()
	if err != nil {
		return nil, status.Errorf(codes.Internal, "register agent %s: %v", id, err)
	}
	engine := req.GetCapabilities().GetEngineApi()
	m.mu.Lock()
	if m.closed {
		m.mu.Unlock()
		return nil, status.Error(codes.Unavailable, "controller shuts down")
	}
	r := m.agents[id]
	if r == nil && !m.cfg.AutoEnroll {
		m.mu.Unlock()
		return nil, status.Errorf(codes.PermissionDenied, "agent %s is not enrolled", id)
	}
	if !m.engines[engineMinor(engine)] {
		m.mu.Unlock()
		return nil, status.Errorf(codes.FailedPrecondition, "agent %s reports engine api %q; the controller drives %v", id, engine, m.cfg.EngineAPIs)
	}
	if r == nil {
		r = &record{id: id}
		m.agents[id] = r
	}
	r.caps, _ = proto.Clone(req.GetCapabilities()).(*agentpb.Capabilities)
	r.agentState = req.GetState()
	r.planID = req.GetPlanId()
	r.phase, r.message = "", ""
	r.token = token
	ev := Event{Kind: EventRegistered, AgentID: id, Time: m.cfg.Clock.Now(), AgentState: r.agentState, PlanID: r.planID}
	m.mu.Unlock()
	m.log.Info("agent registered", "agent_id", id, "state", req.GetState().String(), "plan_id", req.GetPlanId(), "engine_api", engine)
	m.emit(ev)
	return &agentpb.RegisterResponse{
		ControllerId:      m.cfg.ControllerID,
		AgentId:           id,
		SessionToken:      token,
		HeartbeatInterval: durationpb.New(m.cfg.HeartbeatInterval),
		HeartbeatTimeout:  durationpb.New(m.cfg.HeartbeatTimeout),
	}, nil
}

// session is one open Session stream.
type session struct {
	id      uint64
	agentID string
	stream  grpc.BidiStreamingServer[agentpb.SessionRequest, agentpb.SessionResponse]

	sendMu     sync.Mutex
	sendClosed bool
	// slot admits one command other than stop at a time.
	slot chan struct{}

	mu      sync.Mutex
	nextCmd uint64
	pending map[uint64]chan *agentpb.CommandResult

	// expiry is the end of the client certificate, or the zero time.
	expiry time.Time

	done    chan struct{}
	endOnce sync.Once
	endErr  error
}

func (s *session) send(resp *agentpb.SessionResponse) error {
	s.sendMu.Lock()
	defer s.sendMu.Unlock()
	if s.sendClosed {
		return ErrSessionLost
	}
	return s.stream.Send(resp)
}

func (s *session) closeSend() {
	s.sendMu.Lock()
	defer s.sendMu.Unlock()
	s.sendClosed = true
}

func (s *session) end(err error) {
	s.endOnce.Do(func() {
		s.endErr = err
		close(s.done)
	})
}

// addPending assigns the next command id and returns the channel that
// receives its result.
func (s *session) addPending() (uint64, chan *agentpb.CommandResult) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.nextCmd++
	ch := make(chan *agentpb.CommandResult, 1)
	s.pending[s.nextCmd] = ch
	return s.nextCmd, ch
}

func (s *session) dropPending(id uint64) {
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.pending, id)
}

// deliver hands a result to its waiter and reports whether one waited.
func (s *session) deliver(res *agentpb.CommandResult) bool {
	s.mu.Lock()
	ch, ok := s.pending[res.GetCommandId()]
	delete(s.pending, res.GetCommandId())
	s.mu.Unlock()
	if ok {
		ch <- res
	}
	return ok
}

// received is one Recv outcome of the receive goroutine.
type received struct {
	msg *agentpb.SessionRequest
	err error
}

// Session implements ControllerService.Session. The first message must be
// open with the token of the latest Register call. A newer session for the
// same agent ends this one with ABORTED.
func (m *Manager) Session(stream grpc.BidiStreamingServer[agentpb.SessionRequest, agentpb.SessionResponse]) error {
	ctx := stream.Context()
	first, err := stream.Recv()
	if err != nil {
		return err
	}
	s, err := m.open(ctx, stream, first)
	if err != nil {
		return err
	}
	// gRPC forbids stream calls after the handler returns, so the handler
	// waits for a send in flight and closes the stream to later sends.
	defer s.closeSend()
	msgs := make(chan received)
	go func() {
		for {
			msg, err := stream.Recv()
			select {
			case msgs <- received{msg, err}:
			case <-s.done:
				return
			}
			if err != nil {
				return
			}
		}
	}()
	for {
		select {
		case <-s.done:
			return s.endErr
		case r := <-msgs:
			if r.err != nil {
				m.detach(s, ReasonStreamClosed, nil)
				if errors.Is(r.err, io.EOF) {
					return nil
				}
				return r.err
			}
			if err := m.handle(s, r.msg); err != nil {
				m.detach(s, ReasonProtocol, err)
				return err
			}
		}
	}
}

// checkExpiry refuses a credential that expired at expiry. The zero time never
// expires.
func (m *Manager) checkExpiry(expiry time.Time) error {
	if !expiry.IsZero() && !m.cfg.Clock.Now().Before(expiry) {
		return status.Errorf(codes.Unauthenticated, "the client certificate expired at %s", expiry.UTC().Format(time.RFC3339))
	}
	return nil
}

// open checks the open message and makes the session current for its agent.
func (m *Manager) open(ctx context.Context, stream grpc.BidiStreamingServer[agentpb.SessionRequest, agentpb.SessionResponse], first *agentpb.SessionRequest) (*session, error) {
	open := first.GetOpen()
	if open == nil {
		return nil, status.Error(codes.Unauthenticated, "the first session message must be open")
	}
	id := open.GetAgentId()
	identity, err := m.cfg.Identity(ctx, id)
	if err != nil {
		return nil, status.Errorf(codes.Unauthenticated, "agent identity: %v", err)
	}
	if identity != id {
		return nil, status.Errorf(codes.Unauthenticated, "agent_id %s differs from the certificate identity %s", id, identity)
	}
	expiry := m.cfg.Expiry(ctx)
	if err := m.checkExpiry(expiry); err != nil {
		return nil, err
	}
	addr := ""
	if p, ok := peer.FromContext(ctx); ok && p.Addr != nil {
		addr = p.Addr.String()
	}
	m.mu.Lock()
	if m.closed {
		m.mu.Unlock()
		return nil, status.Error(codes.Unavailable, "controller shuts down")
	}
	r := m.agents[id]
	if r == nil || r.token == "" || subtle.ConstantTimeCompare([]byte(r.token), []byte(open.GetSessionToken())) != 1 {
		m.mu.Unlock()
		return nil, status.Errorf(codes.Unauthenticated, "agent %s presented a missing or revoked session token", id)
	}
	m.nextID++
	s := &session{
		id:      m.nextID,
		agentID: id,
		stream:  stream,
		slot:    make(chan struct{}, 1),
		pending: map[uint64]chan *agentpb.CommandResult{},
		expiry:  expiry,
		done:    make(chan struct{}),
	}
	old := r.sess
	r.sess = s
	r.address = addr
	r.lastBeat = m.cfg.Clock.Now()
	ev := Event{Kind: EventOnline, AgentID: id, Time: r.lastBeat, AgentState: r.agentState, PlanID: r.planID}
	m.mu.Unlock()
	if old != nil {
		old.end(status.Error(codes.Aborted, "a newer session for this agent replaced this one"))
		m.log.Info("agent session replaced", "agent_id", id, "session", old.id)
	}
	m.log.Info("agent online", "agent_id", id, "session", s.id, "address", addr)
	m.emit(ev)
	return s, nil
}

// handle processes one message after open.
func (m *Manager) handle(s *session, msg *agentpb.SessionRequest) error {
	switch body := msg.GetBody().(type) {
	case *agentpb.SessionRequest_Open:
		return status.Error(codes.Unauthenticated, "open must not repeat")
	case *agentpb.SessionRequest_Heartbeat:
		return m.heartbeat(s, body.Heartbeat)
	case *agentpb.SessionRequest_Result:
		if !s.deliver(body.Result) {
			m.log.Warn("result for an unknown command", "agent_id", s.agentID, "command_id", body.Result.GetCommandId())
		}
		return nil
	case *agentpb.SessionRequest_Snapshot:
		m.snapshot(s, body.Snapshot)
		return nil
	}
	return status.Error(codes.InvalidArgument, "session message has no body")
}

func (m *Manager) heartbeat(s *session, hb *agentpb.Heartbeat) error {
	now := m.cfg.Clock.Now()
	m.mu.Lock()
	r := m.agents[s.agentID]
	if r == nil || r.sess != s {
		m.mu.Unlock()
		return nil
	}
	changed := r.agentState != hb.GetState() || r.planID != hb.GetPlanId()
	r.lastBeat = now
	r.agentState = hb.GetState()
	r.planID = hb.GetPlanId()
	r.phase = hb.GetPhase()
	r.message = hb.GetMessage()
	ev := Event{Kind: EventStateChanged, AgentID: r.id, Time: now, AgentState: r.agentState, PlanID: r.planID}
	m.mu.Unlock()
	if changed {
		m.log.Info("agent state changed", "agent_id", s.agentID, "state", hb.GetState().String(), "plan_id", hb.GetPlanId())
		m.emit(ev)
	}
	ack := &agentpb.SessionResponse{Body: &agentpb.SessionResponse_HeartbeatAck{HeartbeatAck: &agentpb.HeartbeatAck{
		Sequence:       hb.GetSequence(),
		ControllerTime: timestamppb.New(now),
	}}}
	if err := s.send(ack); err != nil && !errors.Is(err, ErrSessionLost) {
		return fmt.Errorf("agent %s: send heartbeat ack: %w", s.agentID, err)
	}
	return nil
}

func (m *Manager) snapshot(s *session, snap *agentpb.CounterSnapshot) {
	m.mu.Lock()
	r := m.agents[s.agentID]
	if r == nil || r.sess != s {
		m.mu.Unlock()
		return
	}
	if snap.GetPlanId() == r.snapPlan && snap.GetSequence() <= r.snapSeq {
		m.mu.Unlock()
		return
	}
	r.snapPlan, r.snapSeq = snap.GetPlanId(), snap.GetSequence()
	ev := Event{Kind: EventSnapshot, AgentID: r.id, Time: m.cfg.Clock.Now(), AgentState: r.agentState, PlanID: r.planID, Snapshot: snap}
	m.mu.Unlock()
	m.emit(ev)
}

// CommandError is the error of a command that the agent answered with a
// CommandError.
type CommandError struct {
	// AgentID names the agent.
	AgentID string
	// Code and Message come from the agent.
	Code    agentpb.ErrorCode
	Message string
}

func (e *CommandError) Error() string {
	return fmt.Sprintf("agent %s: command failed with %s: %s", e.AgentID, e.Code, e.Message)
}

// Prepare sends a prepare command to agent id and waits for its result.
func (m *Manager) Prepare(ctx context.Context, id string, cmd *agentpb.PrepareCommand) (*agentpb.PrepareResult, error) {
	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_Prepare{Prepare: cmd}}, false)
	if err != nil {
		return nil, err
	}
	return res.GetPrepare(), nil
}

// Start sends a start command to agent id and waits for its result.
func (m *Manager) Start(ctx context.Context, id string, cmd *agentpb.StartCommand) (*agentpb.StartResult, error) {
	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_Start{Start: cmd}}, false)
	if err != nil {
		return nil, err
	}
	return res.GetStart(), nil
}

// Stop sends a stop command to agent id and waits for its result. A stop
// does not wait for an earlier command to finish.
func (m *Manager) Stop(ctx context.Context, id string, cmd *agentpb.StopCommand) (*agentpb.StopResult, error) {
	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_Stop{Stop: cmd}}, true)
	if err != nil {
		return nil, err
	}
	return res.GetStop(), nil
}

// command sends cmd on the current session of agent id and waits for the
// result. Commands other than stop wait for the previous one.
func (m *Manager) command(ctx context.Context, id string, cmd *agentpb.SessionResponse, isStop bool) (*agentpb.CommandResult, error) {
	m.mu.Lock()
	r := m.agents[id]
	var s *session
	if r != nil {
		s = r.sess
	}
	closed := m.closed
	m.mu.Unlock()
	switch {
	case closed:
		return nil, fmt.Errorf("command to agent %s: %w", id, ErrManagerClosed)
	case r == nil:
		return nil, fmt.Errorf("command to agent %s: %w", id, ErrUnknownAgent)
	case s == nil:
		return nil, fmt.Errorf("command to agent %s: %w", id, ErrAgentOffline)
	}
	if !isStop {
		select {
		case s.slot <- struct{}{}:
			defer func() { <-s.slot }()
		case <-s.done:
			return nil, fmt.Errorf("command to agent %s: %w", id, ErrSessionLost)
		case <-ctx.Done():
			return nil, fmt.Errorf("command to agent %s: %w", id, ctx.Err())
		}
	}
	if err := ctx.Err(); err != nil {
		return nil, fmt.Errorf("command to agent %s: %w", id, err)
	}
	cid, ch := s.addPending()
	cmd.CommandId = cid
	if err := s.send(cmd); err != nil {
		s.dropPending(cid)
		return nil, fmt.Errorf("command %d to agent %s: %w: %w", cid, id, ErrSessionLost, err)
	}
	select {
	case res := <-ch:
		if e := res.GetError(); e != nil {
			return res, &CommandError{AgentID: id, Code: e.GetCode(), Message: e.GetMessage()}
		}
		return res, nil
	case <-s.done:
		return nil, fmt.Errorf("command %d to agent %s: %w", cid, id, ErrSessionLost)
	case <-ctx.Done():
		s.dropPending(cid)
		return nil, fmt.Errorf("command %d to agent %s: %w", cid, id, ctx.Err())
	}
}
