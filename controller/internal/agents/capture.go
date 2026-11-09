// SPDX-License-Identifier: GPL-3.0-only

package agents

import (
	"context"
	"crypto/subtle"
	"errors"
	"fmt"
	"io"
	"strconv"
	"strings"
	"sync"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// MaxCaptureChunk is the largest chunk of capture data that an agent sends in
// one UploadCapture request, from CON-13.
const MaxCaptureChunk = 262144

// Errors of the capture commands.
var (
	// ErrCaptureUnsupported reports that the agent cannot serve the request:
	// its engine_api is older than the command needs, or it answered
	// ERROR_CODE_UNIMPLEMENTED. It is not a protocol fault.
	ErrCaptureUnsupported = errors.New("agent does not support this capture command")
	// ErrTransferTimeout reports that the agent did not open UploadCapture
	// within the heartbeat timeout of the fetch command.
	ErrTransferTimeout = errors.New("agent did not open the capture upload in time")
)

// The first engine_api versions that carry each capture command, from CON-05
// and CON-13.
var (
	captureEngine = [3]int{0, 2, 0}
	fetchEngine   = [3]int{0, 2, 2}
)

// engineAtLeast reports whether the semantic version v is at least floor. A
// version that does not parse is older than every floor.
func engineAtLeast(v string, floor [3]int) bool {
	parts := strings.SplitN(v, ".", 3)
	if len(parts) < 3 {
		return false
	}
	for i, p := range parts {
		// Drop a pre-release or build suffix of the last part.
		if j := strings.IndexAny(p, "-+"); j >= 0 {
			p = p[:j]
		}
		n, err := strconv.Atoi(p)
		if err != nil {
			return false
		}
		switch {
		case n > floor[i]:
			return true
		case n < floor[i]:
			return false
		}
	}
	return true
}

// requireEngine fails with ErrCaptureUnsupported when agent id is known and
// reports an engine_api below floor. An unknown agent passes, so that command
// reports the usual error.
func (m *Manager) requireEngine(id, what string, floor [3]int) error {
	m.mu.Lock()
	r := m.agents[id]
	var engine string
	known := r != nil && r.caps != nil
	if known {
		engine = r.caps.GetEngineApi()
	}
	m.mu.Unlock()
	if known && !engineAtLeast(engine, floor) {
		return fmt.Errorf("%s to agent %s: engine api %q is older than %d.%d.%d: %w", what, id, engine, floor[0], floor[1], floor[2], ErrCaptureUnsupported)
	}
	return nil
}

// unsupported maps an UNIMPLEMENTED command error to ErrCaptureUnsupported.
func unsupported(err error) error {
	var cerr *CommandError
	if errors.As(err, &cerr) && cerr.Code == agentpb.ErrorCode_ERROR_CODE_UNIMPLEMENTED {
		return fmt.Errorf("%w: %w", ErrCaptureUnsupported, err)
	}
	return err
}

// StartCapture sends a start_capture command to agent id and waits for its
// result. It needs an agent of engine_api 0.2.0 or later.
func (m *Manager) StartCapture(ctx context.Context, id string, cmd *agentpb.StartCaptureCommand) (*agentpb.StartCaptureResult, error) {
	if err := m.requireEngine(id, "start capture", captureEngine); err != nil {
		return nil, err
	}
	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_StartCapture{StartCapture: cmd}}, false)
	if err != nil {
		return nil, unsupported(err)
	}
	return res.GetStartCapture(), nil
}

// StopCapture sends a stop_capture command to agent id and waits for its
// result. Like stop, it does not wait for an earlier command. It needs an
// agent of engine_api 0.2.0 or later.
func (m *Manager) StopCapture(ctx context.Context, id string, cmd *agentpb.StopCaptureCommand) (*agentpb.StopCaptureResult, error) {
	if err := m.requireEngine(id, "stop capture", captureEngine); err != nil {
		return nil, err
	}
	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_StopCapture{StopCapture: cmd}}, true)
	if err != nil {
		return nil, unsupported(err)
	}
	return res.GetStopCapture(), nil
}

// CaptureSink receives the bytes of one capture upload. The upload handler
// calls it from one goroutine at a time.
type CaptureSink interface {
	// WriteChunk stores data at offset. Offsets arrive in ascending order,
	// and the first equals the start offset of the transfer.
	WriteChunk(offset uint64, data []byte) error
	// End is called once after the last chunk with the total size and the
	// digest that the agent reported. It returns the offset after the last
	// byte that the sink holds and whether it holds the whole file with that
	// digest.
	End(totalBytes uint64, digest string) (received uint64, verified bool, err error)
}

// TransferResult is the outcome of one capture upload.
type TransferResult struct {
	// Received is the offset after the last byte that the sink took.
	Received uint64
	// Verified is true when the sink verified the whole file.
	Verified bool
	// Err is nil for a transfer that ended with verified bytes or an
	// unverified file that End reported without error.
	Err error
}

// Transfer is one controller issued capture transfer, from the fetch_capture
// command to the end of its UploadCapture call.
type Transfer struct {
	id        string
	agentID   string
	captureID string
	start     uint64
	sessionID uint64
	token     string
	sink      CaptureSink
	owner     *transfers

	mu     sync.Mutex
	total  uint64 // size of the file, known after FetchCaptureResult
	chunk  uint64 // largest chunk the agent may send
	active bool
	opened bool
	timer  *time.Timer

	// ready is closed once the agent answered the fetch command, so an upload
	// that arrives first waits for the file size and the chunk limit.
	ready     chan struct{}
	readyOnce sync.Once

	done     chan struct{}
	finishMu sync.Once
	result   TransferResult
}

func (t *Transfer) markReady() { t.readyOnce.Do(func() { close(t.ready) }) }

// ID returns the transfer id that the controller issued.
func (t *Transfer) ID() string { return t.id }

// Done is closed when the transfer ended, successfully or not.
func (t *Transfer) Done() <-chan struct{} { return t.done }

// Result returns the outcome once Done is closed.
func (t *Transfer) Result() TransferResult {
	<-t.done
	return t.result
}

// finish records the outcome once, drops the transfer from the table, and
// wakes the waiters.
func (t *Transfer) finish(res TransferResult) {
	t.finishMu.Do(func() {
		t.mu.Lock()
		if t.timer != nil {
			t.timer.Stop()
		}
		t.mu.Unlock()
		t.result = res
		t.owner.remove(t.id)
		close(t.done)
		t.markReady()
	})
}

// transfers is the table of live transfers of a Manager. The zero value is
// ready to use.
type transfers struct {
	mu   sync.Mutex
	byID map[string]*Transfer
}

func (tt *transfers) add(t *Transfer) {
	tt.mu.Lock()
	defer tt.mu.Unlock()
	if tt.byID == nil {
		tt.byID = map[string]*Transfer{}
	}
	tt.byID[t.id] = t
}

func (tt *transfers) get(id string) *Transfer {
	tt.mu.Lock()
	defer tt.mu.Unlock()
	return tt.byID[id]
}

func (tt *transfers) remove(id string) {
	tt.mu.Lock()
	defer tt.mu.Unlock()
	delete(tt.byID, id)
}

// FetchCapture asks agent id to upload the finished capture cmd.CaptureId
// from cmd.StartOffset on. The manager chooses the single use transfer id and
// the chunk limit, so it overwrites cmd.TransferId and sets
// cmd.MaxChunkBytes. The agent answers at once, and the bytes then arrive
// through UploadCapture into sink. The returned Transfer reports the end of
// the upload. An agent that reports engine_api below 0.2.2 or answers
// UNIMPLEMENTED fails with ErrCaptureUnsupported.
func (m *Manager) FetchCapture(ctx context.Context, id string, cmd *agentpb.FetchCaptureCommand, sink CaptureSink) (*agentpb.FetchCaptureResult, *Transfer, error) {
	if err := m.requireEngine(id, "fetch capture", fetchEngine); err != nil {
		return nil, nil, err
	}
	tid, err := m.newTransferID()
	if err != nil {
		return nil, nil, err
	}
	m.mu.Lock()
	r := m.agents[id]
	var s *session
	var token string
	if r != nil {
		s, token = r.sess, r.token
	}
	m.mu.Unlock()
	if s == nil {
		if r == nil {
			return nil, nil, fmt.Errorf("fetch capture from agent %s: %w", id, ErrUnknownAgent)
		}
		return nil, nil, fmt.Errorf("fetch capture from agent %s: %w", id, ErrAgentOffline)
	}
	c := &agentpb.FetchCaptureCommand{
		PlanId:        cmd.GetPlanId(),
		CaptureId:     cmd.GetCaptureId(),
		StartOffset:   cmd.GetStartOffset(),
		TransferId:    tid,
		MaxChunkBytes: MaxCaptureChunk,
	}
	t := &Transfer{
		id: tid, agentID: id, captureID: c.GetCaptureId(), start: c.GetStartOffset(),
		sessionID: s.id, token: token, sink: sink, owner: &m.xfers,
		chunk: MaxCaptureChunk, ready: make(chan struct{}), done: make(chan struct{}),
	}
	m.xfers.add(t)
	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_FetchCapture{FetchCapture: c}}, false)
	if err != nil {
		t.finish(TransferResult{Err: err})
		return nil, nil, unsupported(err)
	}
	fr := res.GetFetchCapture()
	t.mu.Lock()
	t.total = fr.GetTotalBytes()
	if cb := uint64(fr.GetChunkBytes()); cb > 0 && cb < t.chunk {
		t.chunk = cb
	}
	if fr.GetTransferId() != tid || fr.GetCaptureId() != c.GetCaptureId() {
		t.mu.Unlock()
		err := fmt.Errorf("fetch capture from agent %s: answer names transfer %q capture %q, not %q and %q", id, fr.GetTransferId(), fr.GetCaptureId(), tid, c.GetCaptureId())
		t.finish(TransferResult{Err: err})
		return nil, nil, err
	}
	t.markReady()
	// The agent opens the call right after it answered. Without the call
	// within the heartbeat timeout, the transfer failed.
	t.timer = time.AfterFunc(m.cfg.HeartbeatTimeout, func() {
		t.mu.Lock()
		opened := t.opened
		t.mu.Unlock()
		if !opened {
			t.finish(TransferResult{Err: fmt.Errorf("agent %s: %w", id, ErrTransferTimeout)})
		}
	})
	t.mu.Unlock()
	// The session ending fails a transfer that never reached its upload.
	go func() {
		select {
		case <-s.done:
			t.finish(TransferResult{Err: fmt.Errorf("agent %s: %w", id, ErrSessionLost)})
		case <-t.done:
		}
	}()
	return fr, t, nil
}

func (m *Manager) newTransferID() (string, error) {
	b := make([]byte, 16)
	if _, err := io.ReadFull(m.cfg.Rand, b); err != nil {
		return "", fmt.Errorf("transfer id: %w", err)
	}
	return fmt.Sprintf("xfer-%x", b), nil
}

// uploadRecv is one Recv outcome of the upload receive goroutine.
type uploadRecv struct {
	msg *agentpb.UploadCaptureRequest
	err error
}

// UploadCapture implements ControllerService.UploadCapture. The first request
// must be open and name a transfer that this controller issued to the calling
// agent and that no call used yet. The call ends with NOT_FOUND for an
// unknown, used, or foreign transfer id, ALREADY_EXISTS while another call
// serves the transfer, UNAUTHENTICATED for a wrong session token, and ABORTED
// when the session of the command ended.
func (m *Manager) UploadCapture(stream grpc.ClientStreamingServer[agentpb.UploadCaptureRequest, agentpb.UploadCaptureResponse]) error {
	ctx := stream.Context()
	first, err := stream.Recv()
	if err != nil {
		return err
	}
	open := first.GetOpen()
	if open == nil {
		return status.Error(codes.InvalidArgument, "the first upload message must be open")
	}
	t, s, err := m.claimTransfer(ctx, open)
	if err != nil {
		return err
	}
	// A transfer id serves one call, so a broken call ends the transfer, and
	// the caller resumes with a new fetch at the offset it holds.
	next := t.start
	fail := func(err error) error {
		t.finish(TransferResult{Received: next, Err: err})
		return err
	}
	msgs := make(chan uploadRecv)
	stop := make(chan struct{})
	defer close(stop)
	go func() {
		for {
			msg, err := stream.Recv()
			select {
			case msgs <- uploadRecv{msg, err}:
			case <-stop:
				return
			}
			if err != nil {
				return
			}
		}
	}()
	select {
	case <-t.ready:
	case <-s.done:
		return fail(status.Error(codes.Aborted, "the session of the fetch command ended"))
	case <-ctx.Done():
		return fail(status.FromContextError(ctx.Err()).Err())
	}
	select {
	case <-t.done:
		return status.Error(codes.Aborted, "the transfer ended before the upload began")
	default:
	}
	t.mu.Lock()
	total, limit := t.total, t.chunk
	t.mu.Unlock()
	for {
		select {
		case <-s.done:
			return fail(status.Error(codes.Aborted, "the session of the fetch command ended"))
		case <-ctx.Done():
			return fail(status.FromContextError(ctx.Err()).Err())
		case r := <-msgs:
			if r.err != nil {
				if errors.Is(r.err, io.EOF) {
					r.err = status.Error(codes.InvalidArgument, "the upload ended without an end message")
				}
				return fail(r.err)
			}
			switch body := r.msg.GetBody().(type) {
			case *agentpb.UploadCaptureRequest_Chunk:
				c := body.Chunk
				n := uint64(len(c.GetData()))
				switch {
				case c.GetOffset() != next:
					return fail(status.Errorf(codes.InvalidArgument, "chunk at offset %d, expected %d", c.GetOffset(), next))
				case n == 0 || n > limit:
					return fail(status.Errorf(codes.InvalidArgument, "chunk of %d bytes, allowed 1 to %d", n, limit))
				case total > 0 && next+n > total:
					return fail(status.Errorf(codes.InvalidArgument, "chunk ends at %d, after the file size %d", next+n, total))
				}
				if err := t.sink.WriteChunk(next, c.GetData()); err != nil {
					return fail(status.Errorf(codes.Internal, "store capture chunk: %v", err))
				}
				next += n
			case *agentpb.UploadCaptureRequest_End:
				end := body.End
				if end.GetTotalBytes() != next || (total > 0 && end.GetTotalBytes() != total) {
					return fail(status.Errorf(codes.InvalidArgument, "end reports %d bytes, received up to %d of %d", end.GetTotalBytes(), next, total))
				}
				received, verified, err := t.sink.End(end.GetTotalBytes(), end.GetDigest())
				res := TransferResult{Received: received, Verified: verified, Err: err}
				if err == nil && !verified {
					res.Err = errors.New("capture digest mismatch")
				}
				if serr := stream.SendAndClose(&agentpb.UploadCaptureResponse{ReceivedOffset: received, Verified: verified}); serr != nil && res.Err == nil {
					res.Err = fmt.Errorf("answer upload: %w", serr)
				}
				t.finish(res)
				return nil
			default:
				return fail(status.Error(codes.InvalidArgument, "upload message has no body or repeats open"))
			}
		}
	}
}

// claimTransfer checks the open message of an upload and marks its transfer
// active. It returns the transfer and the session that carried its command.
func (m *Manager) claimTransfer(ctx context.Context, open *agentpb.UploadCaptureOpen) (*Transfer, *session, error) {
	t := m.xfers.get(open.GetTransferId())
	if t == nil {
		return nil, nil, status.Errorf(codes.NotFound, "transfer %q is unknown or used", open.GetTransferId())
	}
	identity, err := m.cfg.Identity(ctx, t.agentID)
	if err != nil {
		return nil, nil, status.Errorf(codes.Unauthenticated, "agent identity: %v", err)
	}
	if identity != t.agentID {
		// The same answer as for an unknown id, so an agent learns nothing
		// about the transfers of another.
		return nil, nil, status.Errorf(codes.NotFound, "transfer %q is unknown or used", open.GetTransferId())
	}
	m.mu.Lock()
	r := m.agents[t.agentID]
	var cur *session
	var token string
	if r != nil {
		cur, token = r.sess, r.token
	}
	m.mu.Unlock()
	if subtle.ConstantTimeCompare([]byte(t.token), []byte(open.GetSessionToken())) != 1 ||
		(token != "" && subtle.ConstantTimeCompare([]byte(token), []byte(open.GetSessionToken())) != 1) {
		return nil, nil, status.Errorf(codes.Unauthenticated, "agent %s presented a wrong session token for the transfer", t.agentID)
	}
	if cur == nil || cur.id != t.sessionID {
		return nil, nil, status.Error(codes.Aborted, "the session of the fetch command ended")
	}
	if open.GetCaptureId() != t.captureID || open.GetStartOffset() != t.start {
		return nil, nil, status.Errorf(codes.InvalidArgument, "open names capture %q at offset %d, the command asked for %q at %d",
			open.GetCaptureId(), open.GetStartOffset(), t.captureID, t.start)
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.active {
		return nil, nil, status.Errorf(codes.AlreadyExists, "transfer %q is served by another call", t.id)
	}
	t.active, t.opened = true, true
	return t, cur, nil
}
