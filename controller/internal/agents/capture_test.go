// SPDX-License-Identifier: GPL-3.0-only

package agents_test

import (
	"bytes"
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

// memSink keeps the chunks of an upload in memory.
type memSink struct {
	mu     sync.Mutex
	buf    bytes.Buffer
	digest string
	ends   int
}

func (s *memSink) WriteChunk(offset uint64, data []byte) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if offset != uint64(s.buf.Len()) { //nolint:gosec // G115: a length
		return errors.New("gap")
	}
	s.buf.Write(data)
	return nil
}

func (s *memSink) End(total uint64, digest string) (uint64, bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.ends++
	n := uint64(s.buf.Len()) //nolint:gosec // G115: a length
	return n, total == n && digest == s.digest, nil
}

// captureRig is a manager with fake agents that run plan "p/<id>" with one
// finished capture "c1".
type captureRig struct {
	mgr  *agents.Manager
	dial fakeagent.Dialer
}

func newCaptureRig(t *testing.T, timeout time.Duration) *captureRig {
	t.Helper()
	cfg := agents.Config{Identity: agents.ClaimedIdentity, AutoEnroll: true}
	if timeout > 0 {
		cfg.HeartbeatInterval, cfg.HeartbeatTimeout = timeout/4, timeout
	}
	mgr, err := agents.New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	return &captureRig{mgr: mgr, dial: fakeagent.Serve(t, mgr)}
}

func (r *captureRig) agent(t *testing.T, id, engine string, opts fakeagent.CaptureOptions) (*fakeagent.Agent, string) {
	t.Helper()
	caps := fakeagent.DefaultCapabilities()
	caps.EngineApi = engine
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	a, err := fakeagent.Start(ctx, fakeagent.Options{ID: id, Dial: r.dial, Capabilities: caps})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	a.SetCaptureOptions(opts)
	plan := "p/" + id
	if _, err := r.mgr.Prepare(ctx, id, &agentpb.PrepareCommand{Plan: &agentpb.Plan{PlanId: plan, RunId: "p", AgentId: id, EngineApi: engine}}); err != nil {
		t.Fatal(err)
	}
	if _, err := r.mgr.Start(ctx, id, &agentpb.StartCommand{PlanId: plan}); err != nil {
		t.Fatal(err)
	}
	return a, plan
}

// finished starts and stops capture c1 of plan and returns its stop result.
func (r *captureRig) finished(t *testing.T, id, plan string) *agentpb.StopCaptureResult {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if _, err := r.mgr.StartCapture(ctx, id, &agentpb.StartCaptureCommand{PlanId: plan, CaptureId: "c1"}); err != nil {
		t.Fatal(err)
	}
	res, err := r.mgr.StopCapture(ctx, id, &agentpb.StopCaptureCommand{PlanId: plan, CaptureId: "c1"})
	if err != nil {
		t.Fatal(err)
	}
	return res
}

func TestFetchCaptureTransfersTheFile(t *testing.T) {
	r := newCaptureRig(t, 0)
	a, plan := r.agent(t, "agent-a", "0.2.2", fakeagent.CaptureOptions{Bytes: 1000, ChunkBytes: 300})
	stop := r.finished(t, "agent-a", plan)
	sink := &memSink{digest: stop.GetDigest()}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	fr, tr, err := r.mgr.FetchCapture(ctx, "agent-a", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, sink)
	if err != nil {
		t.Fatal(err)
	}
	if fr.GetTotalBytes() != 1000 || fr.GetDigest() != stop.GetDigest() || fr.GetTransferId() != tr.ID() || fr.GetChunkBytes() != 300 {
		t.Fatalf("fetch result %v, transfer %s", fr, tr.ID())
	}
	res := tr.Result()
	if res.Err != nil || !res.Verified || res.Received != 1000 {
		t.Fatalf("transfer result %+v", res)
	}
	if !bytes.Equal(sink.buf.Bytes(), a.CaptureBytes(plan, "c1")) {
		t.Fatal("the sink holds other bytes than the agent captured")
	}
	// The command carries the single use transfer id and the chunk limit.
	var cmd *agentpb.FetchCaptureCommand
	for _, c := range a.Commands() {
		if f := c.GetFetchCapture(); f != nil {
			cmd = f
		}
	}
	if cmd == nil || cmd.GetTransferId() != tr.ID() || cmd.GetMaxChunkBytes() != agents.MaxCaptureChunk {
		t.Fatalf("fetch_capture command %v", cmd)
	}
}

func TestFetchCaptureSendsNothingToAnOldAgent(t *testing.T) {
	r := newCaptureRig(t, 0)
	a, plan := r.agent(t, "agent-old", "0.2.1", fakeagent.CaptureOptions{})
	r.finished(t, "agent-old", plan)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_, _, err := r.mgr.FetchCapture(ctx, "agent-old", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, &memSink{})
	if !errors.Is(err, agents.ErrCaptureUnsupported) {
		t.Fatalf("fetch from engine api 0.2.1: %v", err)
	}
	for _, c := range a.Commands() {
		if c.GetFetchCapture() != nil {
			t.Fatal("the controller sent fetch_capture to an agent of engine api 0.2.1")
		}
	}
	// An agent below 0.2.0 gets no capture command at all.
	r2 := newCaptureRig(t, 0)
	caps := fakeagent.DefaultCapabilities() // engine api 0.1.0
	b, err := fakeagent.Start(ctx, fakeagent.Options{ID: "agent-v1", Dial: r2.dial, Capabilities: caps})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(b.Close)
	_, err = r2.mgr.StartCapture(ctx, "agent-v1", &agentpb.StartCaptureCommand{PlanId: "p/agent-v1", CaptureId: "c1"})
	if !errors.Is(err, agents.ErrCaptureUnsupported) {
		t.Fatalf("start capture on engine api 0.1.0: %v", err)
	}
}

func TestFetchCaptureUnimplementedMeansTheAgentCannotServeTheFile(t *testing.T) {
	r := newCaptureRig(t, 0)
	_, plan := r.agent(t, "agent-a", "0.2.2", fakeagent.CaptureOptions{UnimplementedFetch: true})
	r.finished(t, "agent-a", plan)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_, _, err := r.mgr.FetchCapture(ctx, "agent-a", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, &memSink{})
	if !errors.Is(err, agents.ErrCaptureUnsupported) {
		t.Fatalf("UNIMPLEMENTED answer: %v", err)
	}
	var cerr *agents.CommandError
	if !errors.As(err, &cerr) || cerr.Code != agentpb.ErrorCode_ERROR_CODE_UNIMPLEMENTED {
		t.Fatalf("the error does not carry the agent code: %v", err)
	}
}

func TestUploadCaptureChecksTheTransfer(t *testing.T) {
	r := newCaptureRig(t, 0)
	a, plan := r.agent(t, "agent-a", "0.2.2", fakeagent.CaptureOptions{Bytes: 600, NoUpload: true})
	b, _ := r.agent(t, "agent-b", "0.2.2", fakeagent.CaptureOptions{NoUpload: true})
	stop := r.finished(t, "agent-a", plan)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	sink := &memSink{digest: stop.GetDigest()}
	fr, tr, err := r.mgr.FetchCapture(ctx, "agent-a", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, sink)
	if err != nil {
		t.Fatal(err)
	}
	conn, err := r.dial(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	client := agentpb.NewControllerServiceClient(conn)
	token, other := a.Registration().GetSessionToken(), b.Registration().GetSessionToken()
	data := a.CaptureBytes(plan, "c1")

	open := func(id, tok, capture string, offset uint64) *agentpb.UploadCaptureRequest {
		return &agentpb.UploadCaptureRequest{Body: &agentpb.UploadCaptureRequest_Open{Open: &agentpb.UploadCaptureOpen{
			TransferId: id, SessionToken: tok, CaptureId: capture, StartOffset: offset}}}
	}
	chunk := func(offset uint64, d []byte) *agentpb.UploadCaptureRequest {
		return &agentpb.UploadCaptureRequest{Body: &agentpb.UploadCaptureRequest_Chunk{Chunk: &agentpb.CaptureChunk{Offset: offset, Data: d}}}
	}
	end := func(total uint64, digest string) *agentpb.UploadCaptureRequest {
		return &agentpb.UploadCaptureRequest{Body: &agentpb.UploadCaptureRequest_End{End: &agentpb.CaptureEnd{TotalBytes: total, Digest: digest}}}
	}
	// call sends the requests of one call and returns its answer and status.
	call := func(reqs ...*agentpb.UploadCaptureRequest) (*agentpb.UploadCaptureResponse, codes.Code) {
		t.Helper()
		st, err := client.UploadCapture(ctx)
		if err != nil {
			t.Fatal(err)
		}
		for _, q := range reqs {
			if err := st.Send(q); err != nil {
				break // the server ended the call, CloseAndRecv has the status
			}
		}
		resp, err := st.CloseAndRecv()
		return resp, status.Code(err)
	}

	if _, c := call(open("xfer-unknown", token, "c1", 0)); c != codes.NotFound {
		t.Fatalf("unknown transfer id: %s, want NotFound", c)
	}
	if _, c := call(open(tr.ID(), other, "c1", 0)); c != codes.Unauthenticated {
		t.Fatalf("token of another agent: %s, want Unauthenticated", c)
	}
	if _, c := call(open(tr.ID(), "wrong", "c1", 0)); c != codes.Unauthenticated {
		t.Fatalf("wrong token: %s, want Unauthenticated", c)
	}
	if _, c := call(open(tr.ID(), token, "other", 0)); c != codes.InvalidArgument {
		t.Fatalf("wrong capture id: %s, want InvalidArgument", c)
	}
	if _, c := call(open(tr.ID(), token, "c1", 1)); c != codes.InvalidArgument {
		t.Fatalf("wrong offset: %s, want InvalidArgument", c)
	}
	if fr.GetTotalBytes() != 600 {
		t.Fatalf("total %d", fr.GetTotalBytes())
	}
	// A chunk at the wrong offset ends the transfer, and the id is then used up.
	if _, c := call(open(tr.ID(), token, "c1", 0), chunk(5, data[:10])); c != codes.InvalidArgument {
		t.Fatalf("chunk with a gap: %s, want InvalidArgument", c)
	}
	if res := tr.Result(); res.Err == nil || res.Verified {
		t.Fatalf("a broken call must fail the transfer: %+v", res)
	}
	if _, c := call(open(tr.ID(), token, "c1", 0)); c != codes.NotFound {
		t.Fatalf("reused transfer id: %s, want NotFound", c)
	}

	// A new transfer at the held offset completes with the whole file.
	sink2 := &memSink{digest: stop.GetDigest()}
	_, tr2, err := r.mgr.FetchCapture(ctx, "agent-a", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, sink2)
	if err != nil {
		t.Fatal(err)
	}
	if tr2.ID() == tr.ID() {
		t.Fatal("the manager reused a transfer id")
	}
	resp, c := call(open(tr2.ID(), token, "c1", 0), chunk(0, data[:400]), chunk(400, data[400:]), end(600, stop.GetDigest()))
	if c != codes.OK || !resp.GetVerified() || resp.GetReceivedOffset() != 600 {
		t.Fatalf("upload: %s %v", c, resp)
	}
	if res := tr2.Result(); res.Err != nil || !res.Verified {
		t.Fatalf("transfer result %+v", res)
	}
	if _, c := call(open(tr2.ID(), token, "c1", 0)); c != codes.NotFound {
		t.Fatalf("id after a finished transfer: %s, want NotFound", c)
	}
}

func TestUploadCaptureEndMustMatchWhatWasReceived(t *testing.T) {
	r := newCaptureRig(t, 0)
	a, plan := r.agent(t, "agent-a", "0.2.2", fakeagent.CaptureOptions{Bytes: 600, NoUpload: true})
	stop := r.finished(t, "agent-a", plan)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_, tr, err := r.mgr.FetchCapture(ctx, "agent-a", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, &memSink{digest: stop.GetDigest()})
	if err != nil {
		t.Fatal(err)
	}
	conn, err := r.dial(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	st, err := agentpb.NewControllerServiceClient(conn).UploadCapture(ctx)
	if err != nil {
		t.Fatal(err)
	}
	data := a.CaptureBytes(plan, "c1")
	for _, q := range []*agentpb.UploadCaptureRequest{
		{Body: &agentpb.UploadCaptureRequest_Open{Open: &agentpb.UploadCaptureOpen{TransferId: tr.ID(), SessionToken: a.Registration().GetSessionToken(), CaptureId: "c1"}}},
		{Body: &agentpb.UploadCaptureRequest_Chunk{Chunk: &agentpb.CaptureChunk{Offset: 0, Data: data[:100]}}},
		// The end message claims the whole file after 100 bytes.
		{Body: &agentpb.UploadCaptureRequest_End{End: &agentpb.CaptureEnd{TotalBytes: 600, Digest: stop.GetDigest()}}},
	} {
		if err := st.Send(q); err != nil {
			break
		}
	}
	if _, err := st.CloseAndRecv(); status.Code(err) != codes.InvalidArgument {
		t.Fatalf("short upload: %v, want InvalidArgument", err)
	}
	if res := tr.Result(); res.Err == nil || res.Received != 100 {
		t.Fatalf("transfer result %+v, want an error after 100 bytes", res)
	}
}

func TestFetchCaptureFailsWhenTheAgentNeverUploads(t *testing.T) {
	r := newCaptureRig(t, 400*time.Millisecond)
	_, plan := r.agent(t, "agent-a", "0.2.2", fakeagent.CaptureOptions{NoUpload: true})
	r.finished(t, "agent-a", plan)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_, tr, err := r.mgr.FetchCapture(ctx, "agent-a", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, &memSink{})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-tr.Done():
	case <-time.After(5 * time.Second):
		t.Fatal("the transfer never failed")
	}
	if res := tr.Result(); !errors.Is(res.Err, agents.ErrTransferTimeout) {
		t.Fatalf("transfer result %+v, want ErrTransferTimeout", res)
	}
}

func TestSessionEndFailsAnOpenTransfer(t *testing.T) {
	r := newCaptureRig(t, 0)
	a, plan := r.agent(t, "agent-a", "0.2.2", fakeagent.CaptureOptions{NoUpload: true})
	r.finished(t, "agent-a", plan)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_, tr, err := r.mgr.FetchCapture(ctx, "agent-a", &agentpb.FetchCaptureCommand{PlanId: plan, CaptureId: "c1"}, &memSink{})
	if err != nil {
		t.Fatal(err)
	}
	if !r.mgr.EndSession("agent-a") {
		t.Fatal("no session to end")
	}
	select {
	case <-tr.Done():
	case <-time.After(5 * time.Second):
		t.Fatal("the transfer survived its session")
	}
	if res := tr.Result(); !errors.Is(res.Err, agents.ErrSessionLost) {
		t.Fatalf("transfer result %+v, want ErrSessionLost", res)
	}
	_ = a
}
