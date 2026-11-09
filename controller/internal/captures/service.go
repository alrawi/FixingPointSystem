// SPDX-License-Identifier: GPL-3.0-only

package captures

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// ContentType is the media type of a capture download, from the contract.
const ContentType = "application/vnd.tcpdump.pcap"

// Defaults of Config and of a start request.
const (
	// DefaultMaxBytes is the max_bytes of a start request that names none.
	DefaultMaxBytes uint64 = 104857600
	// DefaultCommandTimeout bounds one start, stop, or fetch command.
	DefaultCommandTimeout = 30 * time.Second
	// DefaultTransferTimeout bounds one upload attempt.
	DefaultTransferTimeout = 10 * time.Minute
	// DefaultMaxResumes is the number of times that a download resumes a
	// broken transfer before it gives up.
	DefaultMaxResumes = 5
	// DefaultMaxFileBytes is the largest capture file that the service
	// stores per agent.
	DefaultMaxFileBytes uint64 = 8 << 30
	// DefaultSweepInterval is the time between two retention sweeps.
	DefaultSweepInterval = time.Hour
	// maxBusyRetries bounds the retries after an agent that still serves
	// a broken transfer.
	maxBusyRetries = 10
	// maxFilterBytes bounds the length of a BPF expression.
	maxFilterBytes = 4096
)

// Errors that callers test with errors.Is.
var (
	// ErrCaptureNotFound reports that the run has no capture with that id.
	ErrCaptureNotFound = errors.New("capture not found")
	// ErrInvalidRequest reports a start request that breaks the contract or
	// that every agent refused as invalid.
	ErrInvalidRequest = errors.New("invalid capture request")
	// ErrRunNotRunning reports a capture start on a run that is not running.
	ErrRunNotRunning = errors.New("run is not running")
	// ErrStartFailed reports that no agent of the run started the capture.
	ErrStartFailed = errors.New("no agent started the capture")
	// ErrStopIncomplete reports that an agent did not end its capture, so the
	// capture still runs. The stop is idempotent, and the caller may repeat it.
	ErrStopIncomplete = errors.New("an agent did not end its capture")
	// ErrNoFile reports that no agent of the capture could deliver its file.
	ErrNoFile = errors.New("no capture file available")
	// ErrDigestMismatch reports that the bytes of a transfer have another
	// size or SHA-256 digest than the agent reported at the end of the capture.
	ErrDigestMismatch = errors.New("capture digest mismatch")
)

// Agents is the part of the agent manager that captures need.
// *agents.Manager satisfies it.
type Agents interface {
	StartCapture(ctx context.Context, id string, cmd *agentpb.StartCaptureCommand) (*agentpb.StartCaptureResult, error)
	StopCapture(ctx context.Context, id string, cmd *agentpb.StopCaptureCommand) (*agentpb.StopCaptureResult, error)
	FetchCapture(ctx context.Context, id string, cmd *agentpb.FetchCaptureCommand, sink agents.CaptureSink) (*agentpb.FetchCaptureResult, *agents.Transfer, error)
}

// RunSource returns runs by id. *runs.Orchestrator satisfies it.
type RunSource interface {
	Get(id string) (runs.Run, error)
}

var (
	_ Agents    = (*agents.Manager)(nil)
	_ RunSource = (*runs.Orchestrator)(nil)
)

// Clock tells the time to the retention sweep. agents.Clock and the clocks of
// the tests satisfy it.
type Clock interface {
	Now() time.Time
}

type wallClock struct{}

func (wallClock) Now() time.Time { return time.Now() }

// Config configures a Service. Agents, Runs, and Dir are required.
type Config struct {
	// Agents sends the capture commands.
	Agents Agents
	// Runs looks runs up.
	Runs RunSource
	// Dir is the capture directory. The service creates it with mode 0700.
	Dir string
	// Logger receives structured logs. Nil discards them.
	Logger *slog.Logger
	// NewID returns a fresh capture id of 1 to 128 letters, digits, dots,
	// underscores, and hyphens. It defaults to "cap-" and 16 random hex digits.
	NewID func() (string, error)
	// CommandTimeout bounds one start, stop, or fetch command. Zero means
	// DefaultCommandTimeout.
	CommandTimeout time.Duration
	// TransferTimeout bounds one upload attempt. Zero means
	// DefaultTransferTimeout.
	TransferTimeout time.Duration
	// MaxResumes is the number of resumes after a broken transfer. Zero means
	// DefaultMaxResumes.
	MaxResumes int
	// MaxFileBytes refuses a capture file of an agent that is larger. Zero
	// means DefaultMaxFileBytes.
	MaxFileBytes uint64
	// Retention is how long the service keeps a capture. A sweep deletes the
	// files of a capture, and forgets it, when nothing below its directory
	// changed for longer than Retention. Zero or less keeps every capture and
	// starts no sweep.
	Retention time.Duration
	// SweepInterval is the time between two sweeps. Zero means
	// DefaultSweepInterval.
	SweepInterval time.Duration
	// Clock tells the time that the sweep compares the age of a capture with.
	// Nil reads the wall clock.
	Clock Clock
}

// StartRequest is the body of POST /v1/runs/{id}/captures.
type StartRequest struct {
	// Port names a port of the plan, as the contract requires.
	Port string
	// BPF is a Berkeley Packet Filter expression, or empty.
	BPF string
	// MaxBytes ends the capture on each agent after this many bytes. Zero
	// means DefaultMaxBytes.
	MaxBytes uint64
	// ExportTLSKeys asks for the TLS key log in the file.
	ExportTLSKeys bool
}

// AgentFile is the part of a capture that one agent holds.
type AgentFile struct {
	// Agent and PlanID name the agent and its plan.
	Agent  string
	PlanID string
	// StartError says why the agent did not start the capture. The agent
	// then takes no part in it.
	StartError string
	// Started is true when the agent started the capture.
	Started bool
	// Stopped is true when the agent reported the end of its capture, and
	// Packets, Bytes, Dropped, and Digest come from that report.
	Stopped bool
	Packets uint64
	Bytes   uint64
	Dropped uint64
	Digest  string
	// Lost is true when the agent cannot report an end, because it is gone
	// or does not know the capture, so the capture has no file of it.
	Lost bool
	// Fetched is true when the controller holds the verified file.
	Fetched bool
	// Error is the latest stop or fetch error.
	Error string
}

// Capture is a copy of one capture.
type Capture struct {
	ID            string
	RunID         string
	Port          string
	Filter        string
	MaxBytes      uint64
	ExportTLSKeys bool
	// State is running while an agent still captures, failed when no agent
	// delivered an end, and completed otherwise.
	State api.CaptureState
	// Bytes is the sum of the file sizes of the agents. It is meaningful in
	// the completed state.
	Bytes uint64
	// Agents lists the agents that were asked, ordered by identity.
	Agents []AgentFile
}

// capture is the state of one capture. mu guards files, and work serializes
// the stop and fetch work of one capture, so two downloads fetch a file once.
type capture struct {
	id, runID, port, filter string
	maxBytes                uint64
	tlsKeys                 bool
	dir                     string
	// started is when the service started the capture, by its clock.
	started time.Time

	work sync.Mutex
	mu   sync.Mutex
	// files is ordered by agent identity.
	files []*AgentFile
	// users counts the downloads that hold files of the capture open, and
	// removed is true once the sweep deleted the capture. mu guards both.
	users   int
	removed bool
}

func (c *capture) view() Capture {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := Capture{ID: c.id, RunID: c.runID, Port: c.port, Filter: c.filter, MaxBytes: c.maxBytes, ExportTLSKeys: c.tlsKeys}
	running, live := false, false
	for _, f := range c.files {
		out.Agents = append(out.Agents, *f)
		if !f.Started {
			continue
		}
		switch {
		case f.Stopped:
			live = true
			out.Bytes += f.Bytes
		case !f.Lost:
			running = true
		}
	}
	switch {
	case running:
		out.State = api.CaptureStateRunning
	case live:
		out.State = api.CaptureStateCompleted
	default:
		out.State = api.CaptureStateFailed
	}
	return out
}

// Service starts captures and serves their files. It is safe for concurrent
// use.
type Service struct {
	cfg Config
	log *slog.Logger

	mu    sync.Mutex
	byRun map[string][]*capture

	stop      context.CancelFunc
	sweepDone chan struct{}
	closeOnce sync.Once
}

// New returns a Service and creates its capture directory.
func New(cfg Config) (*Service, error) {
	if cfg.Agents == nil || cfg.Runs == nil || cfg.Dir == "" {
		return nil, errors.New("captures config: Agents, Runs, and Dir are required")
	}
	if cfg.NewID == nil {
		cfg.NewID = randomID
	}
	if cfg.CommandTimeout <= 0 {
		cfg.CommandTimeout = DefaultCommandTimeout
	}
	if cfg.TransferTimeout <= 0 {
		cfg.TransferTimeout = DefaultTransferTimeout
	}
	if cfg.MaxResumes <= 0 {
		cfg.MaxResumes = DefaultMaxResumes
	}
	if cfg.MaxFileBytes == 0 {
		cfg.MaxFileBytes = DefaultMaxFileBytes
	}
	if cfg.Logger == nil {
		cfg.Logger = slog.New(slog.DiscardHandler)
	}
	if err := os.MkdirAll(cfg.Dir, 0o700); err != nil {
		return nil, fmt.Errorf("create capture directory %s: %w", cfg.Dir, err)
	}
	if cfg.SweepInterval <= 0 {
		cfg.SweepInterval = DefaultSweepInterval
	}
	if cfg.Clock == nil {
		cfg.Clock = wallClock{}
	}
	s := &Service{cfg: cfg, log: cfg.Logger, byRun: map[string][]*capture{}}
	if cfg.Retention > 0 {
		var ctx context.Context
		ctx, s.stop = context.WithCancel(context.Background())
		s.sweepDone = make(chan struct{})
		go s.sweepLoop(ctx)
	}
	return s, nil
}

// Close stops the retention sweep and waits for a sweep in progress to end. It
// leaves the files alone and is safe to call more than once.
func (s *Service) Close() {
	s.closeOnce.Do(func() {
		if s.stop != nil {
			s.stop()
			<-s.sweepDone
		}
	})
}

func randomID() (string, error) {
	b := make([]byte, 8)
	if _, err := io.ReadFull(rand.Reader, b); err != nil {
		return "", fmt.Errorf("capture id: %w", err)
	}
	return "cap-" + hex.EncodeToString(b), nil
}

// safeID matches the identities that may name a directory or file.
var safeID = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`)

// PlanID returns the plan id that the run compiler gives the plan of agent
// agentID in run runID.
func PlanID(runID, agentID string) string { return runID + "/" + agentID }

func (s *Service) find(runID, id string) *capture {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, c := range s.byRun[runID] {
		if c.id == id {
			return c
		}
	}
	return nil
}

// Start starts a capture on every agent of the running run runID that takes
// part in it. Agents that cannot capture, such as an agent below engine_api
// 0.2.0 or one without the named port, stay out of the capture and their
// reason is recorded. When no agent starts, Start fails and keeps nothing.
func (s *Service) Start(ctx context.Context, runID string, req StartRequest) (Capture, error) {
	run, err := s.cfg.Runs.Get(runID)
	if err != nil {
		return Capture{}, err
	}
	switch {
	case req.Port == "":
		return Capture{}, fmt.Errorf("%w: port is required", ErrInvalidRequest)
	case len(req.BPF) > maxFilterBytes:
		return Capture{}, fmt.Errorf("%w: bpf is longer than %d bytes", ErrInvalidRequest, maxFilterBytes)
	case req.MaxBytes == 0:
		req.MaxBytes = DefaultMaxBytes
	}
	if run.State != runs.Running {
		return Capture{}, fmt.Errorf("start capture on run %s in state %s: %w", runID, run.State, ErrRunNotRunning)
	}
	if !safeID.MatchString(runID) {
		return Capture{}, fmt.Errorf("%w: run id %q cannot name a directory", ErrInvalidRequest, runID)
	}
	var targets []string
	for _, a := range run.Agents {
		if !slices.Contains(run.FailedAgents, a) && agents.ValidID(a) {
			targets = append(targets, a)
		}
	}
	// Capture.files is ordered by agent identity, whatever the order of the run.
	slices.Sort(targets)
	targets = slices.Compact(targets)
	if len(targets) == 0 {
		return Capture{}, fmt.Errorf("start capture on run %s: the run has no working agent: %w", runID, ErrStartFailed)
	}
	id, err := s.cfg.NewID()
	if err != nil {
		return Capture{}, err
	}
	if !safeID.MatchString(id) {
		return Capture{}, fmt.Errorf("capture id %q is not a valid identifier", id)
	}
	c := &capture{id: id, runID: runID, port: req.Port, filter: req.BPF, maxBytes: req.MaxBytes, tlsKeys: req.ExportTLSKeys,
		dir: filepath.Join(s.cfg.Dir, runID, id), started: s.cfg.Clock.Now()}
	c.files = make([]*AgentFile, len(targets))
	errs := make([]error, len(targets))
	var wg sync.WaitGroup
	for i, a := range targets {
		f := &AgentFile{Agent: a, PlanID: PlanID(runID, a)}
		c.files[i] = f
		wg.Add(1)
		go func() {
			defer wg.Done()
			cctx, cancel := context.WithTimeout(ctx, s.cfg.CommandTimeout)
			defer cancel()
			_, errs[i] = s.cfg.Agents.StartCapture(cctx, a, &agentpb.StartCaptureCommand{
				PlanId: f.PlanID, CaptureId: id, Port: req.Port, Filter: req.BPF, MaxBytes: req.MaxBytes,
				ExportTlsKeys: req.ExportTLSKeys,
			})
		}()
	}
	wg.Wait()
	started := 0
	for i, f := range c.files {
		if errs[i] != nil {
			f.StartError = errs[i].Error()
			s.log.Warn("agent did not start the capture", "run_id", runID, "capture_id", id, "agent_id", f.Agent, "error", errs[i])
			continue
		}
		f.Started = true
		started++
	}
	if started == 0 {
		return Capture{}, startFailure(runID, errs)
	}
	if err := os.MkdirAll(c.dir, 0o700); err != nil {
		s.stopQuietly(c)
		return Capture{}, fmt.Errorf("create capture directory %s: %w", c.dir, err)
	}
	s.mu.Lock()
	s.byRun[runID] = append(s.byRun[runID], c)
	s.mu.Unlock()
	s.log.Info("capture started", "run_id", runID, "capture_id", id, "port", req.Port, "agents", started)
	return c.view(), nil
}

// startFailure joins the errors of every agent. It wraps ErrInvalidRequest
// when every agent refused the request as invalid, so the API answers 422,
// agents.ErrCaptureUnsupported when no agent can capture, and ErrStartFailed
// otherwise.
func startFailure(runID string, errs []error) error {
	invalid, unsupported := true, true
	for _, err := range errs {
		var cerr *agents.CommandError
		if !errors.As(err, &cerr) || cerr.Code != agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT {
			invalid = false
		}
		if !errors.Is(err, agents.ErrCaptureUnsupported) {
			unsupported = false
		}
	}
	kind := ErrStartFailed
	switch {
	case invalid:
		kind = ErrInvalidRequest
	case unsupported:
		kind = agents.ErrCaptureUnsupported
	}
	return fmt.Errorf("start capture on run %s: %w: %w", runID, kind, errors.Join(errs...))
}

// stopQuietly ends the capture on every agent that started it, after the
// service decided to drop it.
func (s *Service) stopQuietly(c *capture) {
	ctx, cancel := context.WithTimeout(context.Background(), s.cfg.CommandTimeout)
	defer cancel()
	for _, f := range c.files {
		if f.Started {
			_, _ = s.cfg.Agents.StopCapture(ctx, f.Agent, &agentpb.StopCaptureCommand{PlanId: f.PlanID, CaptureId: c.id})
		}
	}
}

// List returns the captures of run runID in the order that they started. A
// capture that still runs after its run ended is ended and finished first, so
// the list reports its final state.
func (s *Service) List(ctx context.Context, runID string) ([]Capture, error) {
	run, err := s.cfg.Runs.Get(runID)
	if err != nil {
		return nil, err
	}
	s.mu.Lock()
	cs := slices.Clone(s.byRun[runID])
	s.mu.Unlock()
	out := make([]Capture, 0, len(cs))
	for _, c := range cs {
		if runs.Final(run.State) && c.view().State == api.CaptureStateRunning {
			c.work.Lock()
			s.stopAll(ctx, c, true)
			c.work.Unlock()
		}
		out = append(out, c.view())
	}
	return out, nil
}

// stopAll ends the capture on every agent that has not reported an end. The
// agent answers a repeated stop with the same result, so it is safe after a
// limit or the end of the run. An agent that has no answer for a final run, or
// does not know the capture, is marked lost. The caller holds c.work.
func (s *Service) stopAll(ctx context.Context, c *capture, runFinal bool) {
	for _, f := range c.files {
		c.mu.Lock()
		skip := !f.Started || f.Stopped || f.Lost
		c.mu.Unlock()
		if skip {
			continue
		}
		cctx, cancel := context.WithTimeout(ctx, s.cfg.CommandTimeout)
		res, err := s.cfg.Agents.StopCapture(cctx, f.Agent, &agentpb.StopCaptureCommand{PlanId: f.PlanID, CaptureId: c.id})
		cancel()
		c.mu.Lock()
		if err == nil {
			f.Stopped, f.Error = true, ""
			f.Packets, f.Bytes, f.Dropped, f.Digest = res.GetPackets(), res.GetBytes(), res.GetDropped(), res.GetDigest()
		} else {
			f.Error = err.Error()
			var cerr *agents.CommandError
			gone := errors.Is(err, agents.ErrAgentOffline) || errors.Is(err, agents.ErrSessionLost) || errors.Is(err, agents.ErrUnknownAgent)
			if (errors.As(err, &cerr) && cerr.Code == agentpb.ErrorCode_ERROR_CODE_NOT_FOUND) || errors.Is(err, agents.ErrCaptureUnsupported) || (runFinal && gone) {
				f.Lost = true
			}
		}
		c.mu.Unlock()
		if err != nil {
			s.log.Warn("stop capture failed", "run_id", c.runID, "capture_id", c.id, "agent_id", f.Agent, "error", err)
		}
	}
}

// Stop ends the capture on every agent that has not reported an end, without
// fetching any file, and returns the capture. It is idempotent: a capture
// that already ended answers with its state and sends no command. An agent
// answers a repeated stop with its first result. The capture is completed
// when at least one agent delivered an end, and failed when none did. Stop
// fails with ErrStopIncomplete when an agent that may still capture did not
// answer, because the capture would be neither running nor ended then; the
// caller may repeat the stop. A capture that the retention sweep deleted is
// not found.
func (s *Service) Stop(ctx context.Context, runID, captureID string) (Capture, error) {
	run, err := s.cfg.Runs.Get(runID)
	if err != nil {
		return Capture{}, err
	}
	c := s.find(runID, captureID)
	if c == nil {
		return Capture{}, fmt.Errorf("capture %s of run %s: %w", captureID, runID, ErrCaptureNotFound)
	}
	c.work.Lock()
	defer c.work.Unlock()
	c.mu.Lock()
	gone := c.removed
	c.mu.Unlock()
	if gone {
		return Capture{}, fmt.Errorf("capture %s of run %s: expired: %w", captureID, runID, ErrCaptureNotFound)
	}
	s.stopAll(ctx, c, runs.Final(run.State))
	v := c.view()
	if v.State == api.CaptureStateRunning {
		var errs []error
		for _, f := range v.Agents {
			if f.Started && !f.Stopped && !f.Lost && f.Error != "" {
				errs = append(errs, fmt.Errorf("agent %s: %s", f.Agent, f.Error))
			}
		}
		return v, fmt.Errorf("stop capture %s of run %s: %w: %w", captureID, runID, ErrStopIncomplete, errors.Join(errs...))
	}
	s.log.Info("capture stopped", "run_id", runID, "capture_id", captureID, "state", v.State)
	return v, nil
}

// Download is a capture ready to serve: the verified files of its agents.
type Download struct {
	// Size is the sum of the file sizes.
	Size int64
	// Agents lists the agents whose files the download holds, and Missing the
	// agents that took part in the capture and could not deliver theirs.
	Agents  []string
	Missing []string

	paths []string
	c     *capture
	once  sync.Once
}

// Close releases the files, so the retention sweep may delete them. Every
// Download that Prepare returns needs one Close, and a repeated Close does
// nothing.
func (d *Download) Close() {
	d.once.Do(func() {
		d.c.mu.Lock()
		d.c.users--
		d.c.mu.Unlock()
	})
}

// WriteTo writes the files one after the other, which is a valid pcapng
// file, because every file is a complete section.
func (d *Download) WriteTo(w io.Writer) (int64, error) {
	var n int64
	for _, p := range d.paths {
		f, err := os.Open(p) //nolint:gosec // G304: the path is built from validated identities by the service
		if err != nil {
			return n, fmt.Errorf("open capture file: %w", err)
		}
		m, err := io.Copy(w, f)
		_ = f.Close()
		n += m
		if err != nil {
			return n, fmt.Errorf("send capture file %s: %w", filepath.Base(p), err)
		}
	}
	return n, nil
}

// Prepare ends the capture when it still runs, fetches the file of every
// agent that the service does not hold yet, and returns the files. It
// succeeds when at least one agent delivered a verified file, and then the
// caller closes the Download when it has sent the files. A capture that the
// retention sweep deleted is not found.
func (s *Service) Prepare(ctx context.Context, runID, captureID string) (*Download, error) {
	run, err := s.cfg.Runs.Get(runID)
	if err != nil {
		return nil, err
	}
	c := s.find(runID, captureID)
	if c == nil {
		return nil, fmt.Errorf("capture %s of run %s: %w", captureID, runID, ErrCaptureNotFound)
	}
	c.work.Lock()
	defer c.work.Unlock()
	c.mu.Lock()
	gone := c.removed
	c.mu.Unlock()
	if gone {
		return nil, fmt.Errorf("capture %s of run %s: expired: %w", captureID, runID, ErrCaptureNotFound)
	}
	s.stopAll(ctx, c, runs.Final(run.State))
	var firstErr error
	for _, f := range c.files {
		c.mu.Lock()
		need := f.Stopped && !f.Fetched
		c.mu.Unlock()
		if !need {
			continue
		}
		err := s.fetchFile(ctx, c, f)
		c.mu.Lock()
		if err != nil {
			f.Error = err.Error()
		} else {
			f.Fetched, f.Error = true, ""
		}
		c.mu.Unlock()
		if err != nil {
			s.log.Warn("capture fetch failed", "run_id", runID, "capture_id", captureID, "agent_id", f.Agent, "error", err)
			firstErr = errors.Join(firstErr, err)
		}
	}
	d := &Download{c: c}
	c.mu.Lock()
	defer c.mu.Unlock()
	for _, f := range c.files {
		switch {
		case f.Fetched:
			p := filePath(c, f.Agent)
			d.paths = append(d.paths, p)
			d.Agents = append(d.Agents, f.Agent)
			d.Size += int64(f.Bytes) //nolint:gosec // G115: bounded by MaxFileBytes
		case f.Started:
			d.Missing = append(d.Missing, f.Agent)
		}
	}
	if len(d.paths) == 0 {
		if firstErr == nil {
			firstErr = errors.New("no agent has ended its capture yet")
		}
		return nil, fmt.Errorf("capture %s of run %s: %w: %w", captureID, runID, ErrNoFile, firstErr)
	}
	// The sweep takes c.work before it checks users, and the caller holds the
	// download open until Close, so the files stay until then.
	c.users++
	return d, nil
}

func filePath(c *capture, agent string) string { return filepath.Join(c.dir, agent+".pcapng") }

// fetchFile gets the file of one agent into filePath, resuming a broken
// transfer at the offset that it holds, and verifies size and SHA-256 against
// what the agent reported when the capture ended. The caller holds c.work.
func (s *Service) fetchFile(ctx context.Context, c *capture, f *AgentFile) error {
	size, want := f.Bytes, f.Digest
	if size > s.cfg.MaxFileBytes {
		return fmt.Errorf("agent %s reports a capture of %d bytes, the limit is %d: %w", f.Agent, size, s.cfg.MaxFileBytes, ErrNoFile)
	}
	final := filePath(c, f.Agent)
	part := final + ".part"
	var last error
	busy := 0
	for attempt := 0; attempt <= s.cfg.MaxResumes; attempt++ {
		held := fileSize(part)
		if held > size {
			_ = os.Remove(part)
			held = 0
		}
		if held == size {
			// A complete file needs no transfer.
			if got, err := fileDigest(part); err == nil && got == want {
				return os.Rename(part, final)
			}
			_ = os.Remove(part)
			held = 0
		}
		sink := newSink(part, held, size, want)
		cctx, cancel := context.WithTimeout(ctx, s.cfg.CommandTimeout)
		fr, tr, err := s.cfg.Agents.FetchCapture(cctx, f.Agent, &agentpb.FetchCaptureCommand{
			PlanId: f.PlanID, CaptureId: c.id, StartOffset: held,
		}, sink)
		cancel()
		if err != nil {
			sink.close()
			var cerr *agents.CommandError
			if errors.As(err, &cerr) && cerr.Code == agentpb.ErrorCode_ERROR_CODE_RESOURCE_EXHAUSTED && busy < maxBusyRetries {
				// The agent still winds down the broken transfer, which is
				// no attempt of ours, so the retry does not count.
				busy++
				last = err
				if err := sleep(ctx, time.Duration(busy)*50*time.Millisecond); err != nil {
					return err
				}
				attempt--
				continue
			}
			return fmt.Errorf("fetch capture %s from agent %s: %w", c.id, f.Agent, err)
		}
		if fr.GetTotalBytes() != size || fr.GetDigest() != want {
			sink.close()
			return fmt.Errorf("fetch capture %s from agent %s: the agent now reports %d bytes and digest %q, at the end of the capture it reported %d and %q: %w",
				c.id, f.Agent, fr.GetTotalBytes(), fr.GetDigest(), size, want, ErrDigestMismatch)
		}
		timer := time.NewTimer(s.cfg.TransferTimeout)
		select {
		case <-tr.Done():
			timer.Stop()
		case <-timer.C:
			sink.close()
			last = fmt.Errorf("transfer %s took longer than %s", tr.ID(), s.cfg.TransferTimeout)
			continue
		case <-ctx.Done():
			timer.Stop()
			sink.close()
			return fmt.Errorf("fetch capture %s from agent %s: %w", c.id, f.Agent, ctx.Err())
		}
		res := tr.Result()
		sink.close()
		if res.Err == nil && res.Verified {
			if err := os.Rename(part, final); err != nil {
				return fmt.Errorf("store capture file: %w", err)
			}
			s.log.Info("capture file stored", "run_id", c.runID, "capture_id", c.id, "agent_id", f.Agent, "bytes", size, "resumed_from", held, "attempt", attempt+1)
			return nil
		}
		last = res.Err
		s.log.Warn("capture transfer broke", "run_id", c.runID, "capture_id", c.id, "agent_id", f.Agent, "transfer_id", tr.ID(), "received", res.Received, "error", res.Err)
	}
	return fmt.Errorf("fetch capture %s from agent %s: giving up after %d attempts: %w", c.id, f.Agent, s.cfg.MaxResumes+1, last)
}

// sleep waits for d or for the end of ctx.
func sleep(ctx context.Context, d time.Duration) error {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-t.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// problemDetail trims an error to one line for a problem detail.
func problemDetail(err error) string {
	return strings.ReplaceAll(err.Error(), "\n", "; ")
}
