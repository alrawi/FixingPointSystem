// SPDX-License-Identifier: GPL-3.0-only

package captures_test

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

const stopPath = "/runs/{id}/captures/{capture_id}:stop"

func (r *rig) stop(id string) *httptest.ResponseRecorder {
	r.t.Helper()
	return r.call(http.MethodPost, stopPath, map[string]string{"id": "run-1", "capture_id": id}, "")
}

func decodeCapture(t *testing.T, rec *httptest.ResponseRecorder) api.Capture {
	t.Helper()
	if rec.Code != http.StatusOK {
		t.Fatalf("stop capture: %d %s", rec.Code, rec.Body)
	}
	var c api.Capture
	if err := json.Unmarshal(rec.Body.Bytes(), &c); err != nil {
		t.Fatal(err)
	}
	return c
}

// countCommands returns how many stop_capture and fetch_capture commands the
// agent received.
func countCommands(a *fakeagent.Agent) (stops, fetches int) {
	for _, x := range a.Commands() {
		if x.GetStopCapture() != nil {
			stops++
		}
		if x.GetFetchCapture() != nil {
			fetches++
		}
	}
	return stops, fetches
}

func TestStopCaptureEndsEveryAgentWithoutADownload(t *testing.T) {
	// The run lists agent-b first, and the files still come ordered by identity.
	r := newRig(t, []string{"agent-b", "agent-a"}, rigOptions{})
	c := r.start(`{"port":"eth1","export_tls_keys":true}`)
	if c.Files == nil || len(*c.Files) != 2 {
		t.Fatalf("started capture files %+v, want one entry for each agent", c.Files)
	}
	for _, f := range *c.Files {
		if f.Started == nil || !*f.Started || f.Bytes != nil || f.Digest != nil || f.Error != nil {
			t.Fatalf("file %+v of a running capture, want started and no numbers", f)
		}
	}
	// export_tls_keys of the REST request reaches both agents.
	for id, a := range r.agents {
		var got *bool
		for _, x := range a.Commands() {
			if s := x.GetStartCapture(); s != nil {
				v := s.GetExportTlsKeys()
				got = &v
			}
		}
		if got == nil || !*got {
			t.Fatalf("agent %s: start_capture export_tls_keys = %v, want true", id, got)
		}
	}

	got := decodeCapture(t, r.stop(c.Id))
	if got.Id != c.Id || got.State != api.CaptureStateCompleted || got.Bytes == nil || *got.Bytes != 2000 {
		t.Fatalf("stopped capture %+v, want completed with 2000 bytes", got)
	}
	if got.Files == nil || len(*got.Files) != 2 {
		t.Fatalf("files %+v", got.Files)
	}
	for i, id := range []string{"agent-a", "agent-b"} {
		f := (*got.Files)[i]
		file := r.agents[id].CaptureBytes(r.planOf(id), c.Id)
		sum := sha256.Sum256(file)
		if f.Agent != id || f.Started == nil || !*f.Started || f.Error != nil {
			t.Fatalf("file %d = %+v, want a started file of %s", i, f, id)
		}
		if f.Bytes == nil || *f.Bytes != len(file) || f.Packets == nil || *f.Packets != len(file)/100 || f.Dropped == nil || *f.Dropped != 0 {
			t.Fatalf("file %d numbers %+v, want the report of the agent for %d bytes", i, f, len(file))
		}
		if f.Digest == nil || *f.Digest != "sha256:"+hex.EncodeToString(sum[:]) {
			t.Fatalf("file %d digest %v, want the SHA-256 of the file", i, f.Digest)
		}
	}
	// No download happened: the agents got one stop and no fetch, and uploaded nothing.
	for id, a := range r.agents {
		if stops, fetches := countCommands(a); stops != 1 || fetches != 0 || len(a.Uploads()) != 0 {
			t.Fatalf("agent %s: %d stops, %d fetches, %d uploads, want 1, 0, 0", id, stops, fetches, len(a.Uploads()))
		}
	}
	if files := r.storedFiles(); len(files) != 0 {
		t.Fatalf("the stop stored files %v", files)
	}

	// The stop is idempotent: the same answer, and no command to the agents.
	again := decodeCapture(t, r.stop(c.Id))
	a, _ := json.Marshal(got)
	b, _ := json.Marshal(again)
	if string(a) != string(b) {
		t.Fatalf("second stop answered %s, first %s", b, a)
	}
	for id, ag := range r.agents {
		if stops, _ := countCommands(ag); stops != 1 {
			t.Fatalf("agent %s got %d stop_capture commands after a repeated stop, want 1", id, stops)
		}
	}
	// The list reports the files too, and the download then serves the files.
	if l := r.list(); len(l) != 1 || l[0].State != api.CaptureStateCompleted || l[0].Files == nil || len(*l[0].Files) != 2 {
		t.Fatalf("list after the stop: %+v", l)
	}
	rec := r.download(c.Id)
	want := append(r.agents["agent-a"].CaptureBytes(r.planOf("agent-a"), c.Id), r.agents["agent-b"].CaptureBytes(r.planOf("agent-b"), c.Id)...)
	if rec.Code != http.StatusOK || rec.Body.String() != string(want) {
		t.Fatalf("download after the stop: %d, %d bytes", rec.Code, rec.Body.Len())
	}
}

func TestCaptureFilesNameAnAgentThatDidNotStart(t *testing.T) {
	r := newRig(t, []string{"agent-a", "agent-old"}, rigOptions{engine: map[string]string{"agent-old": "0.1.0"}})
	c := r.start(`{"port":"eth1"}`)
	if c.Files == nil || len(*c.Files) != 2 {
		t.Fatalf("files %+v", c.Files)
	}
	old := (*c.Files)[1]
	if old.Agent != "agent-old" || old.Started == nil || *old.Started || old.Error == nil || !strings.Contains(*old.Error, "engine api") {
		t.Fatalf("file of the old agent %+v, want started false and the reason", old)
	}
	got := decodeCapture(t, r.stop(c.Id))
	if got.State != api.CaptureStateCompleted || got.Bytes == nil || *got.Bytes != 1000 {
		t.Fatalf("stopped capture %+v, want completed with the bytes of agent-a", got)
	}
	f := *got.Files
	if f[0].Agent != "agent-a" || f[0].Bytes == nil || f[1].Agent != "agent-old" || f[1].Bytes != nil || f[1].Error == nil {
		t.Fatalf("files %+v", f)
	}
	if stops, _ := countCommands(r.agents["agent-old"]); stops != 0 {
		t.Fatalf("the agent that did not start got %d stop commands", stops)
	}
}

func TestStopCaptureErrors(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	problemStatus(t, r.stop("cap-nope"), http.StatusNotFound)
	problemStatus(t, r.call(http.MethodPost, stopPath, map[string]string{"id": "run-9", "capture_id": "x"}, ""), http.StatusNotFound)
	// A capture of another run is not reachable through this run.
	c := r.start(`{"port":"eth1"}`)
	r.runs.set(runs.Run{ID: "run-2", State: runs.Running})
	problemStatus(t, r.call(http.MethodPost, stopPath, map[string]string{"id": "run-2", "capture_id": c.Id}, ""), http.StatusNotFound)
}

func TestStopCaptureOfAnAgentThatIsGone(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	c := r.start(`{"port":"eth1"}`)
	r.agents["agent-a"].Crash()
	waitFor(t, "the agent to go offline", func() bool {
		a, err := r.mgr.Get("agent-a")
		return err == nil && a.State == agents.StateOffline
	})
	// While the run runs, the agent may come back, so the capture is neither
	// running nor ended: the stop answers 409 and may be repeated.
	rec := r.stop(c.Id)
	problemStatus(t, rec, http.StatusConflict)
	if !strings.Contains(rec.Body.String(), "agent-a") {
		t.Fatalf("the problem does not name the agent: %s", rec.Body)
	}
	// Once the run ended, the agent cannot answer anymore: failed, no numbers.
	run, _ := r.runs.Get("run-1")
	run.State = runs.Failed
	r.runs.set(run)
	got := decodeCapture(t, r.stop(c.Id))
	if got.State != api.CaptureStateFailed || got.Bytes != nil || got.Files == nil || len(*got.Files) != 1 {
		t.Fatalf("stopped capture %+v, want failed with one file", got)
	}
	if f := (*got.Files)[0]; f.Agent != "agent-a" || f.Started == nil || !*f.Started || f.Bytes != nil || f.Digest != nil {
		t.Fatalf("file %+v, want a started file without numbers", f)
	}
	if again := decodeCapture(t, r.stop(c.Id)); again.State != api.CaptureStateFailed {
		t.Fatalf("second stop %+v", again)
	}
}
