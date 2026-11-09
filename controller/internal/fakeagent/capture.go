// SPDX-License-Identifier: GPL-3.0-only

package fakeagent

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// CaptureOptions shapes how a fake agent captures and uploads. The zero value
// serves a capture of DefaultCaptureBytes bytes in chunks of 262144 bytes.
type CaptureOptions struct {
	// Bytes is the size of every capture file. Zero means DefaultCaptureBytes.
	Bytes int
	// ChunkBytes is the largest chunk of an upload. Zero means 262144.
	ChunkBytes int
	// BreakAfter breaks the upload attempts one by one: attempt i stops after
	// BreakAfter[i] bytes of its own and cancels the call without an end
	// message. Attempts beyond the list run to the end.
	BreakAfter []int
	// CorruptUploads is the number of uploads that flip one byte of the file,
	// while the end message still carries the digest of the true file.
	CorruptUploads int
	// UnimplementedFetch answers fetch_capture with ERROR_CODE_UNIMPLEMENTED,
	// as an agent of engine_api 0.2.2 does until AGT-09.
	UnimplementedFetch bool
	// NoUpload answers fetch_capture but never opens UploadCapture, so a test
	// can drive the call by hand or let the controller time out.
	NoUpload bool
}

// DefaultCaptureBytes is the file size of a fake capture without CaptureOptions.
const DefaultCaptureBytes = 1000

// Upload records one UploadCapture call of a fake agent.
type Upload struct {
	// TransferID and StartOffset come from the fetch command.
	TransferID  string
	StartOffset uint64
	// Sent is the number of bytes sent in this call.
	Sent int
	// Err is the error of the call, or nil when the controller verified it.
	Err error
}

type fakeCapture struct {
	data    []byte
	stopped bool
	digest  string
}

// captureHost holds the capture state of one fake agent.
type captureHost struct {
	mu       sync.Mutex
	opts     CaptureOptions
	captures map[string]*fakeCapture // by plan id and capture id
	busy     bool
	attempts int
	corrupt  int
	uploads  []Upload
	idle     *sync.Cond
}

// hosts maps a fake agent to its capture state, so the Agent type needs no
// field for it.
var hosts sync.Map // *Agent -> *captureHost

func (a *Agent) captureHost() *captureHost {
	if h, ok := hosts.Load(a); ok {
		return h.(*captureHost) //nolint:errcheck,forcetypeassert // only captureHost values are stored
	}
	h := &captureHost{captures: map[string]*fakeCapture{}}
	h.idle = sync.NewCond(&h.mu)
	actual, _ := hosts.LoadOrStore(a, h)
	return actual.(*captureHost) //nolint:errcheck,forcetypeassert // only captureHost values are stored
}

// SetCaptureOptions sets the capture behavior of the agent and resets the
// counters of its uploads.
func (a *Agent) SetCaptureOptions(o CaptureOptions) {
	h := a.captureHost()
	h.mu.Lock()
	defer h.mu.Unlock()
	h.opts, h.attempts, h.corrupt = o, 0, o.CorruptUploads
}

// CaptureBytes returns the true file of a capture that was stopped, or nil.
func (a *Agent) CaptureBytes(planID, captureID string) []byte {
	h := a.captureHost()
	h.mu.Lock()
	defer h.mu.Unlock()
	if c := h.captures[planID+"\x00"+captureID]; c != nil && c.stopped {
		return append([]byte(nil), c.data...)
	}
	return nil
}

// Uploads returns the UploadCapture calls of the agent so far.
func (a *Agent) Uploads() []Upload {
	h := a.captureHost()
	h.mu.Lock()
	defer h.mu.Unlock()
	return append([]Upload(nil), h.uploads...)
}

// WaitUploadsIdle blocks until no upload runs.
func (a *Agent) WaitUploadsIdle() {
	h := a.captureHost()
	h.mu.Lock()
	defer h.mu.Unlock()
	for h.busy {
		h.idle.Wait()
	}
}

var transferIDPattern = regexp.MustCompile(`^[A-Za-z0-9._-]{1,128}$`)

// content returns the deterministic bytes of a capture file.
func content(planID, captureID string, n int) []byte {
	out := make([]byte, 0, n)
	var ctr [8]byte
	for i := uint64(0); len(out) < n; i++ {
		binary.BigEndian.PutUint64(ctr[:], i)
		sum := sha256.Sum256(append([]byte(planID+"\x00"+captureID+"\x00"), ctr[:]...))
		out = append(out, sum[:]...)
	}
	return out[:n]
}

// applyCapture answers the three capture commands.
func (a *Agent) applyCapture(cmd *agentpb.SessionResponse, res *agentpb.CommandResult, state agentpb.AgentState, plan string) *agentpb.CommandResult {
	fail := func(code agentpb.ErrorCode, format string, args ...any) *agentpb.CommandResult {
		res.Error = &agentpb.CommandError{Code: code, Message: fmt.Sprintf(format, args...)}
		return res
	}
	h := a.captureHost()
	switch body := cmd.GetBody().(type) {
	case *agentpb.SessionResponse_StartCapture:
		c := body.StartCapture
		switch {
		case state != agentpb.AgentState_AGENT_STATE_RUNNING:
			return fail(agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION, "start capture in state %s", state)
		case c.GetPlanId() != plan:
			return fail(agentpb.ErrorCode_ERROR_CODE_NOT_FOUND, "plan %s is not loaded", c.GetPlanId())
		case c.GetCaptureId() == "":
			return fail(agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT, "empty capture id")
		case !a.hasPort(c.GetPort()):
			return fail(agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT, "unknown port %q", c.GetPort())
		case strings.Count(c.GetFilter(), "(") != strings.Count(c.GetFilter(), ")"):
			return fail(agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT, "filter %q does not compile", c.GetFilter())
		}
		h.mu.Lock()
		defer h.mu.Unlock()
		key := c.GetPlanId() + "\x00" + c.GetCaptureId()
		if h.captures[key] != nil {
			return fail(agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION, "capture %s exists", c.GetCaptureId())
		}
		h.captures[key] = &fakeCapture{}
		res.Result = &agentpb.CommandResult_StartCapture{StartCapture: &agentpb.StartCaptureResult{CaptureId: c.GetCaptureId()}}
	case *agentpb.SessionResponse_StopCapture:
		c := body.StopCapture
		h.mu.Lock()
		defer h.mu.Unlock()
		fc := h.captures[c.GetPlanId()+"\x00"+c.GetCaptureId()]
		if fc == nil {
			return fail(agentpb.ErrorCode_ERROR_CODE_NOT_FOUND, "capture %s of plan %s is unknown", c.GetCaptureId(), c.GetPlanId())
		}
		if !fc.stopped {
			n := h.opts.Bytes
			if n == 0 {
				n = DefaultCaptureBytes
			}
			fc.data = content(c.GetPlanId(), c.GetCaptureId(), n)
			sum := sha256.Sum256(fc.data)
			fc.digest = "sha256:" + hex.EncodeToString(sum[:])
			fc.stopped = true
		}
		res.Result = &agentpb.CommandResult_StopCapture{StopCapture: &agentpb.StopCaptureResult{
			CaptureId: c.GetCaptureId(), Packets: uint64(len(fc.data) / 100), Bytes: uint64(len(fc.data)), Digest: fc.digest,
		}}
	case *agentpb.SessionResponse_FetchCapture:
		c := body.FetchCapture
		h.mu.Lock()
		defer h.mu.Unlock()
		fc := h.captures[c.GetPlanId()+"\x00"+c.GetCaptureId()]
		switch {
		case h.opts.UnimplementedFetch:
			return fail(agentpb.ErrorCode_ERROR_CODE_UNIMPLEMENTED, "fetch_capture is not implemented")
		case fc == nil:
			return fail(agentpb.ErrorCode_ERROR_CODE_NOT_FOUND, "capture %s of plan %s is unknown", c.GetCaptureId(), c.GetPlanId())
		case !fc.stopped:
			return fail(agentpb.ErrorCode_ERROR_CODE_FAILED_PRECONDITION, "capture %s has not ended", c.GetCaptureId())
		case !transferIDPattern.MatchString(c.GetTransferId()):
			return fail(agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT, "bad transfer id %q", c.GetTransferId())
		case c.GetStartOffset() > uint64(len(fc.data)):
			return fail(agentpb.ErrorCode_ERROR_CODE_INVALID_ARGUMENT, "start offset %d is beyond the file of %d bytes", c.GetStartOffset(), len(fc.data))
		case h.busy:
			return fail(agentpb.ErrorCode_ERROR_CODE_RESOURCE_EXHAUSTED, "a transfer runs")
		}
		chunk := 262144
		if h.opts.ChunkBytes > 0 {
			chunk = h.opts.ChunkBytes
		}
		if m := int(c.GetMaxChunkBytes()); m > 0 && m < chunk {
			chunk = m
		}
		attempt := h.attempts
		h.attempts++
		stopAfter := -1
		if attempt < len(h.opts.BreakAfter) {
			stopAfter = h.opts.BreakAfter[attempt]
		}
		corrupt := h.corrupt > 0
		if corrupt {
			h.corrupt--
		}
		data := append([]byte(nil), fc.data...)
		if !h.opts.NoUpload {
			h.busy = true
			up := &upload{host: h, agent: a, cmd: c, data: data, digest: fc.digest, chunk: chunk, stopAfter: stopAfter, corrupt: corrupt}
			go up.run()
		}
		res.Result = &agentpb.CommandResult_FetchCapture{FetchCapture: &agentpb.FetchCaptureResult{
			CaptureId: c.GetCaptureId(), TransferId: c.GetTransferId(), TotalBytes: uint64(len(data)), Digest: fc.digest, ChunkBytes: uint32(chunk), //nolint:gosec // G115: at most 262144
		}}
	}
	return res
}

func (a *Agent) hasPort(name string) bool {
	if name == "" {
		return true
	}
	for _, p := range a.opts.Capabilities.GetPorts() {
		if p.GetName() == name {
			return true
		}
	}
	return false
}

// upload is one UploadCapture call of a fake agent.
type upload struct {
	host      *captureHost
	agent     *Agent
	cmd       *agentpb.FetchCaptureCommand
	data      []byte
	digest    string
	chunk     int
	stopAfter int
	corrupt   bool
}

var errBroken = errors.New("fake agent broke the upload")

func (u *upload) run() {
	rec := Upload{TransferID: u.cmd.GetTransferId(), StartOffset: u.cmd.GetStartOffset()}
	rec.Sent, rec.Err = u.send()
	u.host.mu.Lock()
	u.host.uploads = append(u.host.uploads, rec)
	u.host.busy = false
	u.host.idle.Broadcast()
	u.host.mu.Unlock()
}

func (u *upload) send() (int, error) {
	a := u.agent
	a.mu.Lock()
	conn, resp := a.conn, a.resp
	a.mu.Unlock()
	if conn == nil || resp == nil {
		return 0, ErrDisconnected
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	stream, err := agentpb.NewControllerServiceClient(conn).UploadCapture(ctx)
	if err != nil {
		return 0, err
	}
	err = stream.Send(&agentpb.UploadCaptureRequest{Body: &agentpb.UploadCaptureRequest_Open{Open: &agentpb.UploadCaptureOpen{
		TransferId: u.cmd.GetTransferId(), SessionToken: resp.GetSessionToken(), CaptureId: u.cmd.GetCaptureId(), StartOffset: u.cmd.GetStartOffset(),
	}}})
	if err != nil {
		return 0, err
	}
	sent := 0
	for off := int(u.cmd.GetStartOffset()); off < len(u.data); { //nolint:gosec // G115: checked against the file size
		n := min(u.chunk, len(u.data)-off)
		if u.stopAfter >= 0 && sent+n > u.stopAfter {
			n = u.stopAfter - sent
		}
		if n <= 0 {
			// Break the call here: let the controller read what was sent,
			// then cancel the call without an end message.
			time.Sleep(50 * time.Millisecond)
			cancel()
			_, _ = stream.CloseAndRecv()
			return sent, errBroken
		}
		piece := append([]byte(nil), u.data[off:off+n]...)
		if u.corrupt && off == int(u.cmd.GetStartOffset()) { //nolint:gosec // G115: checked against the file size
			piece[0] ^= 0xff
		}
		err := stream.Send(&agentpb.UploadCaptureRequest{Body: &agentpb.UploadCaptureRequest_Chunk{Chunk: &agentpb.CaptureChunk{Offset: uint64(off), Data: piece}}}) //nolint:gosec // G115: non negative
		if err != nil {
			return sent, err
		}
		off += n
		sent += n
	}
	if u.stopAfter >= 0 && sent >= u.stopAfter {
		time.Sleep(50 * time.Millisecond)
		cancel()
		_, _ = stream.CloseAndRecv()
		return sent, errBroken
	}
	err = stream.Send(&agentpb.UploadCaptureRequest{Body: &agentpb.UploadCaptureRequest_End{End: &agentpb.CaptureEnd{TotalBytes: uint64(len(u.data)), Digest: u.digest}}})
	if err != nil {
		return sent, err
	}
	out, err := stream.CloseAndRecv()
	if err != nil {
		return sent, err
	}
	if !out.GetVerified() {
		return sent, fmt.Errorf("controller did not verify the file, received offset %d", out.GetReceivedOffset())
	}
	return sent, nil
}
