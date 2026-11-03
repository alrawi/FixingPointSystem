// SPDX-License-Identifier: GPL-3.0-only

package agents

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"maps"
	"slices"
	"sync"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// State is the inventory state of an agent, as the Agent schema of the REST
// contract names it.
type State = api.AgentState

// The four inventory states.
const (
	StateOffline  = api.AgentStateOffline
	StateOnline   = api.AgentStateOnline
	StateReserved = api.AgentStateReserved
	StateBusy     = api.AgentStateBusy
)

// Errors that callers test with errors.Is.
var (
	// ErrUnknownAgent reports that no agent has the requested identity.
	ErrUnknownAgent = errors.New("unknown agent")
	// ErrInvalidID reports an identity outside the Id pattern of the contract.
	ErrInvalidID = errors.New("invalid agent id")
	// ErrAgentOffline reports that the agent has no open session.
	ErrAgentOffline = errors.New("agent offline")
	// ErrAgentReserved reports that another holder reserves the agent.
	ErrAgentReserved = errors.New("agent reserved by another holder")
	// ErrAgentBusy reports that the agent holds a plan without a reservation.
	ErrAgentBusy = errors.New("agent busy")
	// ErrSessionLost reports that the session ended before a command result arrived.
	ErrSessionLost = errors.New("agent session lost")
	// ErrManagerClosed reports that Shutdown ran.
	ErrManagerClosed = errors.New("agent manager closed")
)

// Agent is a copy of one inventory entry.
type Agent struct {
	// ID is the agent identity.
	ID string
	// State is the inventory state.
	State State
	// Enrolled is true when an operator enrolled the identity ahead of time.
	Enrolled bool
	// Labels holds the reported labels, overridden by the enrolled labels.
	Labels map[string]string
	// Capabilities is the latest registration, or nil before the first one.
	Capabilities *agentpb.Capabilities
	// AgentState, PlanID, Phase, and Message come from the latest heartbeat
	// or registration.
	AgentState agentpb.AgentState
	PlanID     string
	Phase      string
	Message    string
	// Address is the peer address of the open session.
	Address string
	// ReservedBy names the reservation holder, or is empty.
	ReservedBy string
	// LastHeartbeat is the controller time of the latest heartbeat or session open.
	LastHeartbeat time.Time
}

// EventKind classifies an Event.
type EventKind int

// The event kinds.
const (
	// EventRegistered follows a successful Register call.
	EventRegistered EventKind = iota + 1
	// EventOnline follows the open message of a session.
	EventOnline
	// EventOffline follows the end of the current session of an agent.
	EventOffline
	// EventStateChanged follows a heartbeat that reports a new agent state.
	EventStateChanged
	// EventSnapshot carries a counter snapshot that is not a duplicate.
	EventSnapshot
)

// String returns the name of the kind.
func (k EventKind) String() string {
	switch k {
	case EventRegistered:
		return "registered"
	case EventOnline:
		return "online"
	case EventOffline:
		return "offline"
	case EventStateChanged:
		return "state_changed"
	case EventSnapshot:
		return "snapshot"
	}
	return fmt.Sprintf("EventKind(%d)", int(k))
}

// Event reports a change of one agent to Config.Observer.
type Event struct {
	// Kind classifies the event.
	Kind EventKind
	// AgentID names the agent.
	AgentID string
	// Time is the controller time of the event.
	Time time.Time
	// AgentState and PlanID are the agent state and plan after the event.
	AgentState agentpb.AgentState
	PlanID     string
	// Reason explains EventOffline, such as heartbeat timeout.
	Reason string
	// Snapshot is the snapshot of EventSnapshot.
	Snapshot *agentpb.CounterSnapshot
}

// Reasons of EventOffline.
const (
	ReasonHeartbeatTimeout = "heartbeat timeout"
	ReasonStreamClosed     = "stream closed"
	ReasonReplaced         = "replaced by a newer session"
	ReasonShutdown         = "controller shutdown"
	ReasonProtocol         = "protocol violation"
)

// record is the inventory entry of one agent. The manager lock guards it.
type record struct {
	id         string
	enrolled   bool
	labels     map[string]string
	caps       *agentpb.Capabilities
	agentState agentpb.AgentState
	planID     string
	phase      string
	message    string
	token      string
	reservedBy string
	sess       *session
	address    string
	lastBeat   time.Time
	// snapPlan and snapSeq hold the newest snapshot, so duplicates after a
	// reconnection are dropped.
	snapPlan string
	snapSeq  uint64
}

func (r *record) state() State {
	switch {
	case r.sess == nil:
		return StateOffline
	case holdsPlan(r.agentState):
		return StateBusy
	case r.reservedBy != "":
		return StateReserved
	}
	return StateOnline
}

// holdsPlan reports whether an agent in state s holds a plan that is not finished.
func holdsPlan(s agentpb.AgentState) bool {
	switch s {
	case agentpb.AgentState_AGENT_STATE_PREPARING, agentpb.AgentState_AGENT_STATE_PREPARED,
		agentpb.AgentState_AGENT_STATE_RUNNING, agentpb.AgentState_AGENT_STATE_STOPPING:
		return true
	}
	return false
}

func (r *record) snapshot() Agent {
	labels := map[string]string{}
	if r.caps != nil {
		maps.Copy(labels, r.caps.GetLabels())
	}
	maps.Copy(labels, r.labels)
	a := Agent{
		ID:            r.id,
		State:         r.state(),
		Enrolled:      r.enrolled,
		Labels:        labels,
		AgentState:    r.agentState,
		PlanID:        r.planID,
		Phase:         r.phase,
		Message:       r.message,
		Address:       r.address,
		ReservedBy:    r.reservedBy,
		LastHeartbeat: r.lastBeat,
	}
	if r.caps != nil {
		a.Capabilities, _ = proto.Clone(r.caps).(*agentpb.Capabilities)
	}
	return a
}

// Manager is the agent manager. It implements agentpb.ControllerServiceServer.
type Manager struct {
	agentpb.UnimplementedControllerServiceServer

	cfg     Config
	engines map[string]bool
	log     *slog.Logger

	mu     sync.Mutex
	agents map[string]*record
	nextID uint64
	closed bool
}

var _ agentpb.ControllerServiceServer = (*Manager)(nil)

// New returns a Manager. A zero ControllerID becomes fps-controller, zero
// heartbeat settings become DefaultHeartbeatInterval and
// DefaultHeartbeatTimeout, and empty EngineAPIs become DefaultEngineAPIs.
func New(cfg Config) (*Manager, error) {
	if cfg.ControllerID == "" {
		cfg.ControllerID = "fps-controller"
	}
	if cfg.HeartbeatInterval == 0 {
		cfg.HeartbeatInterval = DefaultHeartbeatInterval
	}
	if cfg.HeartbeatTimeout == 0 {
		cfg.HeartbeatTimeout = DefaultHeartbeatTimeout
	}
	if cfg.HeartbeatInterval < 0 || cfg.HeartbeatTimeout <= cfg.HeartbeatInterval {
		return nil, fmt.Errorf("agent manager config: heartbeat timeout %s must exceed heartbeat interval %s",
			cfg.HeartbeatTimeout, cfg.HeartbeatInterval)
	}
	if len(cfg.EngineAPIs) == 0 {
		cfg.EngineAPIs = DefaultEngineAPIs
	}
	engines := map[string]bool{}
	for _, v := range cfg.EngineAPIs {
		mm := engineMinor(v + ".0")
		if mm == "" || mm != v {
			return nil, fmt.Errorf("agent manager config: engine api %q is not major.minor", v)
		}
		engines[mm] = true
	}
	if cfg.Identity == nil {
		cfg.Identity = PeerCertIdentity
	}
	if cfg.Clock == nil {
		cfg.Clock = SystemClock{}
	}
	if cfg.Logger == nil {
		cfg.Logger = slog.New(slog.DiscardHandler)
	}
	if cfg.Rand == nil {
		cfg.Rand = rand.Reader
	}
	return &Manager{cfg: cfg, engines: engines, log: cfg.Logger, agents: map[string]*record{}}, nil
}

// HeartbeatTimeout returns the configured heartbeat timeout.
func (m *Manager) HeartbeatTimeout() time.Duration { return m.cfg.HeartbeatTimeout }

func (m *Manager) emit(ev Event) {
	if m.cfg.Observer != nil {
		m.cfg.Observer(ev)
	}
}

func (m *Manager) newToken() (string, error) {
	b := make([]byte, 32)
	if _, err := io.ReadFull(m.cfg.Rand, b); err != nil {
		return "", fmt.Errorf("session token: %w", err)
	}
	return hex.EncodeToString(b), nil
}

// Enroll accepts the identity id ahead of its registration and sets its
// operator labels. Enrolling a known identity replaces its labels.
func (m *Manager) Enroll(id string, labels map[string]string) (Agent, error) {
	if !ValidID(id) {
		return Agent{}, fmt.Errorf("enroll agent %q: %w", id, ErrInvalidID)
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	r := m.agents[id]
	if r == nil {
		r = &record{id: id}
		m.agents[id] = r
	}
	r.enrolled = true
	r.labels = maps.Clone(labels)
	m.log.Info("agent enrolled", "agent_id", id)
	return r.snapshot(), nil
}

// Get returns the agent with identity id.
func (m *Manager) Get(id string) (Agent, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	r := m.agents[id]
	if r == nil {
		return Agent{}, fmt.Errorf("get agent %s: %w", id, ErrUnknownAgent)
	}
	return r.snapshot(), nil
}

// List returns every agent, ordered by identity.
func (m *Manager) List() []Agent {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]Agent, 0, len(m.agents))
	for _, id := range slices.Sorted(maps.Keys(m.agents)) {
		out = append(out, m.agents[id].snapshot())
	}
	return out
}

// Reserve reserves the agent id for holder, such as a run id. Reserving an
// agent again for the same holder succeeds. An offline agent, an agent of
// another holder, and an unreserved agent that holds a plan cannot be
// reserved.
func (m *Manager) Reserve(id, holder string) (Agent, error) {
	if holder == "" {
		return Agent{}, fmt.Errorf("reserve agent %s: empty holder", id)
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	r := m.agents[id]
	switch {
	case r == nil:
		return Agent{}, fmt.Errorf("reserve agent %s: %w", id, ErrUnknownAgent)
	case r.reservedBy == holder:
		return r.snapshot(), nil
	case r.reservedBy != "":
		return Agent{}, fmt.Errorf("reserve agent %s for %s: held by %s: %w", id, holder, r.reservedBy, ErrAgentReserved)
	case r.sess == nil:
		return Agent{}, fmt.Errorf("reserve agent %s: %w", id, ErrAgentOffline)
	case holdsPlan(r.agentState):
		return Agent{}, fmt.Errorf("reserve agent %s: plan %s loaded: %w", id, r.planID, ErrAgentBusy)
	}
	r.reservedBy = holder
	m.log.Info("agent reserved", "agent_id", id, "holder", holder)
	return r.snapshot(), nil
}

// Release ends the reservation of the agent id. An empty holder releases any
// reservation, which is what an operator does. Releasing an unreserved agent
// succeeds, and a reservation of another holder stays.
func (m *Manager) Release(id, holder string) (Agent, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	r := m.agents[id]
	switch {
	case r == nil:
		return Agent{}, fmt.Errorf("release agent %s: %w", id, ErrUnknownAgent)
	case holder != "" && r.reservedBy != "" && r.reservedBy != holder:
		return Agent{}, fmt.Errorf("release agent %s for %s: held by %s: %w", id, holder, r.reservedBy, ErrAgentReserved)
	}
	if r.reservedBy != "" {
		m.log.Info("agent released", "agent_id", id, "holder", r.reservedBy)
	}
	r.reservedBy = ""
	return r.snapshot(), nil
}

// Sweep marks every agent offline whose latest heartbeat is older than the
// heartbeat timeout, ends its session with DEADLINE_EXCEEDED, and returns
// the identities it marked. A reservation stays, so its holder decides what
// to do.
func (m *Manager) Sweep() []string {
	now := m.cfg.Clock.Now()
	var expired []*session
	m.mu.Lock()
	for _, id := range slices.Sorted(maps.Keys(m.agents)) {
		r := m.agents[id]
		if r.sess != nil && now.Sub(r.lastBeat) > m.cfg.HeartbeatTimeout {
			expired = append(expired, r.sess)
		}
	}
	m.mu.Unlock()
	ids := make([]string, 0, len(expired))
	for _, s := range expired {
		if m.detach(s, ReasonHeartbeatTimeout, status.Error(codes.DeadlineExceeded, "no heartbeat within the heartbeat timeout")) {
			ids = append(ids, s.agentID)
		}
	}
	return ids
}

// Run calls Sweep once per heartbeat interval until ctx ends. It is the only
// place that reads the wall clock for scheduling, and tests call Sweep
// directly instead.
func (m *Manager) Run(ctx context.Context) error {
	t := time.NewTicker(m.cfg.HeartbeatInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-t.C:
			m.Sweep()
		}
	}
}

// Shutdown ends every session with UNAVAILABLE, so agents dial again, and
// makes later calls fail.
func (m *Manager) Shutdown() {
	m.mu.Lock()
	m.closed = true
	var open []*session
	for _, r := range m.agents {
		if r.sess != nil {
			open = append(open, r.sess)
		}
	}
	m.mu.Unlock()
	for _, s := range open {
		m.detach(s, ReasonShutdown, status.Error(codes.Unavailable, "controller shuts down"))
	}
}

// detach ends session s with err and marks its agent offline when s is still
// its current session. It reports whether it marked the agent offline.
func (m *Manager) detach(s *session, reason string, err error) bool {
	m.mu.Lock()
	r := m.agents[s.agentID]
	current := r != nil && r.sess == s
	var ev Event
	if current {
		r.sess = nil
		r.address = ""
		ev = Event{Kind: EventOffline, AgentID: r.id, Time: m.cfg.Clock.Now(), AgentState: r.agentState, PlanID: r.planID, Reason: reason}
	}
	m.mu.Unlock()
	s.end(err)
	if current {
		m.log.Warn("agent offline", "agent_id", s.agentID, "session", s.id, "reason", reason)
		m.emit(ev)
	}
	return current
}
