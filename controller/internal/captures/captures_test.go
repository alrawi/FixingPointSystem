// SPDX-License-Identifier: GPL-3.0-only

package captures_test

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/getkin/kin-openapi/openapi3"
	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
	"github.com/alrawi/FixingPointSystem/controller/internal/captures"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

const contractPath = "../../../contracts/openapi/fps-api-v1.yaml"

// runList is a RunSource over a fixed set of runs.
type runList struct {
	mu   sync.Mutex
	runs map[string]runs.Run
}

func (l *runList) Get(id string) (runs.Run, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	r, ok := l.runs[id]
	if !ok {
		return runs.Run{}, fmt.Errorf("get run %s: %w", id, runs.ErrRunNotFound)
	}
	return r, nil
}

func (l *runList) set(r runs.Run) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.runs[r.ID] = r
}

// rig is a real agent manager with fake agents behind the capture service,
// and the REST layer of the service behind the real router and token check.
type rig struct {
	t      *testing.T
	mgr    *agents.Manager
	dial   fakeagent.Dialer
	runs   *runList
	svc    *captures.Service
	dir    string
	doc    *openapi3.T
	server http.Handler
	agents map[string]*fakeagent.Agent
}

type rigOptions struct {
	engine map[string]string // engine_api by agent, default 0.2.2
	mutate func(*captures.Config)
}

func newRig(t *testing.T, ids []string, o rigOptions) *rig {
	t.Helper()
	mgr, err := agents.New(agents.Config{Identity: agents.ClaimedIdentity, AutoEnroll: true})
	if err != nil {
		t.Fatal(err)
	}
	r := &rig{t: t, mgr: mgr, dial: fakeagent.Serve(t, mgr), runs: &runList{runs: map[string]runs.Run{}}, dir: t.TempDir(), agents: map[string]*fakeagent.Agent{}}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	run := runs.Run{ID: "run-1", State: runs.Running}
	for _, id := range ids {
		caps := fakeagent.DefaultCapabilities()
		caps.EngineApi = "0.2.2"
		if v := o.engine[id]; v != "" {
			caps.EngineApi = v
		}
		a, err := fakeagent.Start(ctx, fakeagent.Options{ID: id, Dial: r.dial, Capabilities: caps})
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(a.Close)
		a.SetCaptureOptions(fakeagent.CaptureOptions{Bytes: 1000, ChunkBytes: 100})
		r.agents[id] = a
		plan := captures.PlanID(run.ID, id)
		if _, err := mgr.Prepare(ctx, id, &agentpb.PrepareCommand{Plan: &agentpb.Plan{PlanId: plan, RunId: run.ID, AgentId: id, EngineApi: caps.EngineApi}}); err != nil {
			t.Fatal(err)
		}
		if _, err := mgr.Start(ctx, id, &agentpb.StartCommand{PlanId: plan}); err != nil {
			t.Fatal(err)
		}
		run.Agents = append(run.Agents, id)
	}
	r.runs.set(run)
	cfg := captures.Config{Agents: mgr, Runs: r.runs, Dir: filepath.Join(r.dir, "captures"), CommandTimeout: 5 * time.Second, TransferTimeout: 5 * time.Second, MaxResumes: 3}
	if o.mutate != nil {
		o.mutate(&cfg)
	}
	if r.svc, err = captures.New(cfg); err != nil {
		t.Fatal(err)
	}
	if r.doc, err = contract.Load(contractPath); err != nil {
		t.Fatal(err)
	}
	auth, err := api.NewStaticTokens(map[string]api.Scope{"token-run": api.ScopeRun})
	if err != nil {
		t.Fatal(err)
	}
	r.server = api.New(captures.NewAPI(r.svc), auth, slog.New(slog.DiscardHandler)).Handler()
	return r
}

// call sends one request to the operation at path and method, and checks the
// exchange against the contract. A JSON answer is validated by the contract
// validator. The capture download carries a binary body, so its status and
// media type are checked against the responses that the contract lists.
func (r *rig) call(method, path string, params map[string]string, body string) *httptest.ResponseRecorder {
	r.t.Helper()
	item := r.doc.Paths.Value(path)
	if item == nil || item.GetOperation(method) == nil {
		r.t.Fatalf("the contract has no %s %s", method, path)
	}
	url := path
	for k, v := range params {
		url = strings.ReplaceAll(url, "{"+k+"}", v)
	}
	newReq := func() *http.Request {
		req := httptest.NewRequest(method, api.BasePath+url, strings.NewReader(body))
		if body != "" {
			req.Header.Set("Content-Type", "application/json")
		}
		req.Header.Set("Authorization", "Bearer token-run")
		return req
	}
	rec := httptest.NewRecorder()
	r.server.ServeHTTP(rec, newReq())
	op := item.GetOperation(method)
	in := &openapi3filter.RequestValidationInput{
		Request:    newReq(),
		PathParams: params,
		Route:      &routers.Route{Spec: r.doc, Path: path, PathItem: item, Method: method, Operation: op},
		Options:    &openapi3filter.Options{AuthenticationFunc: openapi3filter.NoopAuthenticationFunc},
	}
	if body != "" && !strings.Contains(body, "BREAK") {
		if err := openapi3filter.ValidateRequest(context.Background(), in); err != nil {
			r.t.Fatalf("%s %s: request breaks the contract: %v", method, url, err)
		}
	}
	ct := rec.Header().Get("Content-Type")
	if method == http.MethodGet && path == "/runs/{id}/captures/{capture_id}" && rec.Code == http.StatusOK {
		ok := op.Responses.Status(http.StatusOK)
		if ok == nil || ok.Value.Content.Get(ct) == nil {
			r.t.Fatalf("%s %s: the contract lists no 200 answer of media type %q", method, url, ct)
		}
		return rec
	}
	out := &openapi3filter.ResponseValidationInput{
		RequestValidationInput: in,
		Status:                 rec.Code,
		Header:                 rec.Header(),
		Body:                   io.NopCloser(bytes.NewReader(rec.Body.Bytes())),
		Options:                &openapi3filter.Options{IncludeResponseStatus: true},
	}
	if err := openapi3filter.ValidateResponse(context.Background(), out); err != nil {
		r.t.Fatalf("%s %s: response breaks the contract: %v\nbody: %s", method, url, err, rec.Body)
	}
	return rec
}

func (r *rig) start(body string) api.Capture {
	r.t.Helper()
	rec := r.call(http.MethodPost, "/runs/{id}/captures", map[string]string{"id": "run-1"}, body)
	if rec.Code != http.StatusAccepted {
		r.t.Fatalf("start capture: %d %s", rec.Code, rec.Body)
	}
	var c api.Capture
	if err := json.Unmarshal(rec.Body.Bytes(), &c); err != nil {
		r.t.Fatal(err)
	}
	return c
}

func (r *rig) list() []api.Capture {
	r.t.Helper()
	rec := r.call(http.MethodGet, "/runs/{id}/captures", map[string]string{"id": "run-1"}, "")
	if rec.Code != http.StatusOK {
		r.t.Fatalf("list captures: %d %s", rec.Code, rec.Body)
	}
	var cs []api.Capture
	if err := json.Unmarshal(rec.Body.Bytes(), &cs); err != nil {
		r.t.Fatal(err)
	}
	return cs
}

func (r *rig) download(id string) *httptest.ResponseRecorder {
	r.t.Helper()
	return r.call(http.MethodGet, "/runs/{id}/captures/{capture_id}", map[string]string{"id": "run-1", "capture_id": id}, "")
}

// problemStatus checks that rec is a problem answer with the status.
func problemStatus(t *testing.T, rec *httptest.ResponseRecorder, want int) {
	t.Helper()
	if rec.Code != want || rec.Header().Get("Content-Type") != api.ProblemContentType {
		t.Fatalf("status %d type %q, want a %d problem: %s", rec.Code, rec.Header().Get("Content-Type"), want, rec.Body)
	}
}

func (r *rig) planOf(agent string) string { return captures.PlanID("run-1", agent) }

// storedFiles lists the files below the capture directory.
func (r *rig) storedFiles() []string {
	var out []string
	_ = filepath.Walk(filepath.Join(r.dir, "captures"), func(p string, fi os.FileInfo, err error) error {
		if err == nil && !fi.IsDir() {
			out = append(out, filepath.Base(p))
		}
		return nil
	})
	return out
}

func TestCaptureLifecycleAgainstTheContract(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	c := r.start(`{"port":"eth1","bpf":"tcp port 80","max_bytes":4096,"export_tls_keys":false}`)
	if c.State != api.CaptureStateRunning || c.Port != "eth1" || c.Id == "" || c.Bytes != nil {
		t.Fatalf("started capture %+v", c)
	}
	// The agent got the command with the controller's capture id and filter.
	var cmd *agentpb.StartCaptureCommand
	for _, x := range r.agents["agent-a"].Commands() {
		if s := x.GetStartCapture(); s != nil {
			cmd = s
		}
	}
	if cmd == nil || cmd.GetCaptureId() != c.Id || cmd.GetPlanId() != "run-1/agent-a" || cmd.GetPort() != "eth1" ||
		cmd.GetFilter() != "tcp port 80" || cmd.GetMaxBytes() != 4096 {
		t.Fatalf("start_capture command %v for capture %+v", cmd, c)
	}
	if got := r.list(); len(got) != 1 || got[0].Id != c.Id || got[0].State != api.CaptureStateRunning {
		t.Fatalf("list while running: %+v", got)
	}

	rec := r.download(c.Id)
	if rec.Code != http.StatusOK {
		t.Fatalf("download: %d %s", rec.Code, rec.Body)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "application/vnd.tcpdump.pcap" {
		t.Fatalf("Content-Type %q", ct)
	}
	want := r.agents["agent-a"].CaptureBytes(r.planOf("agent-a"), c.Id)
	if len(want) != 1000 || !bytes.Equal(rec.Body.Bytes(), want) {
		t.Fatalf("download holds %d bytes, the agent captured %d, equal %v", rec.Body.Len(), len(want), bytes.Equal(rec.Body.Bytes(), want))
	}
	if rec.Header().Get("Content-Length") != "1000" || rec.Header().Get("X-FPS-Capture-Agents") != "agent-a" || rec.Header().Get("X-FPS-Capture-Missing") != "" {
		t.Fatalf("headers %v", rec.Header())
	}
	if got := r.list(); len(got) != 1 || got[0].State != api.CaptureStateCompleted || got[0].Bytes == nil || *got[0].Bytes != 1000 {
		t.Fatalf("list after the download: %+v", got)
	}
	// A second download reads the stored file and asks the agent for nothing.
	r.agents["agent-a"].WaitUploadsIdle()
	before := len(r.agents["agent-a"].Uploads())
	if again := r.download(c.Id); !bytes.Equal(again.Body.Bytes(), want) {
		t.Fatal("the second download differs")
	}
	if n := len(r.agents["agent-a"].Uploads()); n != before {
		t.Fatalf("the second download fetched again: %d uploads, want %d", n, before)
	}
	if files := r.storedFiles(); len(files) != 1 || !strings.HasSuffix(files[0], ".pcapng") {
		t.Fatalf("stored files %v, want one finished file and no part file", files)
	}
}

func TestDownloadResumesAfterAnInterruptedUpload(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	a := r.agents["agent-a"]
	// The first upload breaks after 300 bytes and the second after 200 more.
	a.SetCaptureOptions(fakeagent.CaptureOptions{Bytes: 1000, ChunkBytes: 100, BreakAfter: []int{300, 200}})
	c := r.start(`{"port":"eth1"}`)
	rec := r.download(c.Id)
	if rec.Code != http.StatusOK {
		t.Fatalf("download: %d %s", rec.Code, rec.Body)
	}
	want := a.CaptureBytes(r.planOf("agent-a"), c.Id)
	if !bytes.Equal(rec.Body.Bytes(), want) {
		t.Fatal("the resumed download differs from the captured file")
	}
	a.WaitUploadsIdle()
	ups := a.Uploads()
	if len(ups) != 3 {
		t.Fatalf("uploads %+v, want 3 attempts", ups)
	}
	ids := map[string]bool{}
	var held uint64
	for i, u := range ups {
		if ids[u.TransferID] {
			t.Fatalf("transfer id %s used twice", u.TransferID)
		}
		ids[u.TransferID] = true
		if u.StartOffset != held {
			t.Fatalf("attempt %d starts at offset %d, the controller held %d", i, u.StartOffset, held)
		}
		held = u.StartOffset + uint64(u.Sent)
	}
	if ups[1].StartOffset == 0 || ups[2].StartOffset <= ups[1].StartOffset || ups[2].Err != nil {
		t.Fatalf("resume offsets %+v", ups)
	}
	if held != 1000 {
		t.Fatalf("the attempts cover %d bytes, want 1000: %+v", held, ups)
	}
	if files := r.storedFiles(); len(files) != 1 || !strings.HasSuffix(files[0], ".pcapng") {
		t.Fatalf("stored files %v", files)
	}
}

func TestDigestMismatchIsRejected(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	a := r.agents["agent-a"]
	a.SetCaptureOptions(fakeagent.CaptureOptions{Bytes: 1000, ChunkBytes: 100, CorruptUploads: 100})
	c := r.start(`{"port":"eth1"}`)
	rec := r.download(c.Id)
	problemStatus(t, rec, http.StatusBadGateway)
	if !strings.Contains(rec.Body.String(), "digest mismatch") {
		t.Fatalf("the problem does not name the mismatch: %s", rec.Body)
	}
	if files := r.storedFiles(); len(files) != 0 {
		t.Fatalf("a corrupt file stayed on disk: %v", files)
	}
	a.WaitUploadsIdle()
	ups := a.Uploads()
	if len(ups) != 4 { // the first attempt and three resumes
		t.Fatalf("uploads %d, want 4", len(ups))
	}
	for _, u := range ups {
		if u.StartOffset != 0 || u.Err == nil {
			t.Fatalf("every attempt must restart at zero and fail: %+v", ups)
		}
	}
	// The capture itself ended, so it counts as completed, and the agent
	// still has the good file.
	if got := r.list(); got[0].State != api.CaptureStateCompleted {
		t.Fatalf("list: %+v", got)
	}

	// One corrupt upload is repaired by the next attempt.
	a.SetCaptureOptions(fakeagent.CaptureOptions{Bytes: 1000, ChunkBytes: 100, CorruptUploads: 1})
	rec = r.download(c.Id)
	if rec.Code != http.StatusOK || !bytes.Equal(rec.Body.Bytes(), a.CaptureBytes(r.planOf("agent-a"), c.Id)) {
		t.Fatalf("download after one corrupt upload: %d", rec.Code)
	}
}

func TestAgentThatCannotServeTheFile(t *testing.T) {
	r := newRig(t, []string{"agent-a", "agent-b"}, rigOptions{})
	r.agents["agent-a"].SetCaptureOptions(fakeagent.CaptureOptions{Bytes: 1000, UnimplementedFetch: true})
	c := r.start(`{"port":"eth1"}`)
	// The file of agent-b alone is served, and the headers name the gap.
	rec := r.download(c.Id)
	if rec.Code != http.StatusOK || !bytes.Equal(rec.Body.Bytes(), r.agents["agent-b"].CaptureBytes(r.planOf("agent-b"), c.Id)) {
		t.Fatalf("download: %d %d", rec.Code, rec.Body.Len())
	}
	if rec.Header().Get("X-FPS-Capture-Agents") != "agent-b" || rec.Header().Get("X-FPS-Capture-Missing") != "agent-a" {
		t.Fatalf("headers %v", rec.Header())
	}

	// With nobody able to serve, the answer is 501.
	r.agents["agent-b"].SetCaptureOptions(fakeagent.CaptureOptions{Bytes: 1000, UnimplementedFetch: true})
	c2 := r.start(`{"port":"eth1"}`)
	problemStatus(t, r.download(c2.Id), http.StatusNotImplemented)
}

func TestOldAgentGetsNoFetchCommand(t *testing.T) {
	r := newRig(t, []string{"agent-old"}, rigOptions{engine: map[string]string{"agent-old": "0.2.1"}})
	c := r.start(`{"port":"eth1"}`) // 0.2.1 still takes start_capture
	problemStatus(t, r.download(c.Id), http.StatusNotImplemented)
	for _, x := range r.agents["agent-old"].Commands() {
		if x.GetFetchCapture() != nil {
			t.Fatal("the controller sent fetch_capture to an agent of engine api 0.2.1")
		}
	}
}

func TestMultipleAgentsServeOneStream(t *testing.T) {
	r := newRig(t, []string{"agent-a", "agent-b"}, rigOptions{})
	c := r.start(`{"port":"eth1"}`)
	rec := r.download(c.Id)
	want := append(r.agents["agent-a"].CaptureBytes(r.planOf("agent-a"), c.Id), r.agents["agent-b"].CaptureBytes(r.planOf("agent-b"), c.Id)...)
	if rec.Code != http.StatusOK || !bytes.Equal(rec.Body.Bytes(), want) || rec.Header().Get("Content-Length") != "2000" {
		t.Fatalf("download: %d, %d bytes", rec.Code, rec.Body.Len())
	}
	if rec.Header().Get("X-FPS-Capture-Agents") != "agent-a,agent-b" {
		t.Fatalf("headers %v", rec.Header())
	}
	if got := r.list(); *got[0].Bytes != 2000 {
		t.Fatalf("list: %+v", got)
	}
}

func TestStartCaptureErrors(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	id := map[string]string{"id": "run-1"}
	cases := []struct {
		name   string
		params map[string]string
		body   string
		want   int
	}{
		{"unknown port", id, `{"port":"nope"}`, http.StatusUnprocessableEntity},
		{"bad filter", id, `{"port":"eth1","bpf":"tcp and ((port 80"}`, http.StatusUnprocessableEntity},
		{"unknown run", map[string]string{"id": "run-9"}, `{"port":"eth1"}`, http.StatusNotFound},
		{"unknown field BREAK", id, `{"port":"eth1","extra":1}`, http.StatusBadRequest},
		{"zero max_bytes BREAK", id, `{"port":"eth1","max_bytes":0}`, http.StatusUnprocessableEntity},
		{"not json BREAK", id, `[`, http.StatusBadRequest},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			problemStatus(t, r.call(http.MethodPost, "/runs/{id}/captures", c.params, c.body), c.want)
		})
	}
	if got := r.list(); len(got) != 0 {
		t.Fatalf("failed starts left captures behind: %+v", got)
	}

	run, _ := r.runs.Get("run-1")
	run.State = runs.Stopping
	r.runs.set(run)
	problemStatus(t, r.call(http.MethodPost, "/runs/{id}/captures", id, `{"port":"eth1"}`), http.StatusConflict)
}

func TestUnknownCaptureAndRun(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	problemStatus(t, r.download("cap-nope"), http.StatusNotFound)
	problemStatus(t, r.call(http.MethodGet, "/runs/{id}/captures", map[string]string{"id": "run-9"}, ""), http.StatusNotFound)
	problemStatus(t, r.call(http.MethodGet, "/runs/{id}/captures/{capture_id}", map[string]string{"id": "run-9", "capture_id": "x"}, ""), http.StatusNotFound)
	// A capture of another run is not reachable through this run.
	c := r.start(`{"port":"eth1"}`)
	r.runs.set(runs.Run{ID: "run-2", State: runs.Running})
	problemStatus(t, r.call(http.MethodGet, "/runs/{id}/captures/{capture_id}", map[string]string{"id": "run-2", "capture_id": c.Id}, ""), http.StatusNotFound)
}

func TestFinishedRunEndsRunningCaptures(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	c := r.start(`{"port":"eth1"}`)
	run, _ := r.runs.Get("run-1")
	run.State = runs.Completed
	r.runs.set(run)
	// The list of a finished run ends the capture and reports its size.
	if got := r.list(); len(got) != 1 || got[0].Id != c.Id || got[0].State != api.CaptureStateCompleted || *got[0].Bytes != 1000 {
		t.Fatalf("list after the run ended: %+v", got)
	}
	if rec := r.download(c.Id); rec.Code != http.StatusOK || rec.Body.Len() != 1000 {
		t.Fatalf("download after the run ended: %d", rec.Code)
	}
}

func TestCaptureOfAnAgentThatIsGoneIsLost(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	c := r.start(`{"port":"eth1"}`)
	run, _ := r.runs.Get("run-1")
	run.State = runs.Failed
	r.runs.set(run)
	r.agents["agent-a"].Crash()
	waitFor(t, "the agent to go offline", func() bool {
		a, err := r.mgr.Get("agent-a")
		return err == nil && a.State == agents.StateOffline
	})
	if got := r.list(); len(got) != 1 || got[0].State != api.CaptureStateFailed || got[0].Bytes != nil {
		t.Fatalf("list: %+v", got)
	}
	problemStatus(t, r.download(c.Id), http.StatusBadGateway)
}

func TestNewRequiresItsParts(t *testing.T) {
	if _, err := captures.New(captures.Config{}); err == nil {
		t.Fatal("New accepted an empty config")
	}
}

func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for !cond() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(5 * time.Millisecond)
	}
}
