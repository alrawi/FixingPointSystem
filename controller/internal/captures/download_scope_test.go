// SPDX-License-Identifier: GPL-3.0-only

package captures_test

import (
	"net/http"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// stops returns the stop_capture commands that each agent of the rig got.
func (r *rig) stops() map[string]int {
	out := map[string]int{}
	for id, a := range r.agents {
		out[id], _ = countCommands(a)
	}
	return out
}

func (r *rig) asRead(f func()) {
	r.token = "token-read"
	defer func() { r.token = "" }()
	f()
}

// TestReadScopedDownloadDoesNotStopARunningCapture checks issue #452: a
// download needs only read scope, so it must not end a capture that another
// operator still runs. It answers 409, sends no stop_capture, and the capture
// keeps running until a caller with run scope stops it.
func TestReadScopedDownloadDoesNotStopARunningCapture(t *testing.T) {
	r := newRig(t, []string{"agent-a", "agent-b"}, rigOptions{})
	c := r.start(`{"port":"eth1"}`)

	r.asRead(func() {
		rec := r.download(c.Id)
		problemStatus(t, rec, http.StatusConflict)
		// The list is read-scoped too, and a running run's capture stays running.
		if l := r.list(); len(l) != 1 || l[0].State != api.CaptureStateRunning {
			t.Fatalf("list after a refused download: %+v", l)
		}
	})
	for id, n := range r.stops() {
		if n != 0 {
			t.Fatalf("agent %s got %d stop_capture commands from read-scoped requests, want 0", id, n)
		}
	}

	// A caller with run scope stops it, and then a read-scoped download serves
	// the files without another command.
	if got := decodeCapture(t, r.stop(c.Id)); got.State != api.CaptureStateCompleted {
		t.Fatalf("stopped capture %+v", got)
	}
	r.asRead(func() {
		if rec := r.download(c.Id); rec.Code != http.StatusOK || rec.Body.Len() != 2000 {
			t.Fatalf("read-scoped download of a stopped capture: %d, %d bytes", rec.Code, rec.Body.Len())
		}
	})
	for id, n := range r.stops() {
		if n != 1 {
			t.Fatalf("agent %s got %d stop_capture commands, want the one of the run-scoped stop", id, n)
		}
	}
}

// TestRunScopedDownloadStillEndsARunningCapture keeps the behaviour that
// callers with run scope rely on: their download ends the capture first.
func TestRunScopedDownloadStillEndsARunningCapture(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	c := r.start(`{"port":"eth1"}`)
	if rec := r.download(c.Id); rec.Code != http.StatusOK || rec.Body.Len() != 1000 {
		t.Fatalf("run-scoped download: %d, %d bytes", rec.Code, rec.Body.Len())
	}
	if n := r.stops()["agent-a"]; n != 1 {
		t.Fatalf("agent-a got %d stop_capture commands, want 1", n)
	}
}

// TestReadScopedDownloadFinishesACaptureOfAnEndedRun matches the list: once
// the run ended, its capture cannot go on, and finishing it is no
// interruption, so a read-scoped download may collect it.
func TestReadScopedDownloadFinishesACaptureOfAnEndedRun(t *testing.T) {
	r := newRig(t, []string{"agent-a"}, rigOptions{})
	c := r.start(`{"port":"eth1"}`)
	run, err := r.runs.Get("run-1")
	if err != nil {
		t.Fatal(err)
	}
	run.State = runs.Completed
	r.runs.set(run)
	r.asRead(func() {
		if rec := r.download(c.Id); rec.Code != http.StatusOK || rec.Body.Len() != 1000 {
			t.Fatalf("read-scoped download after the run ended: %d, %d bytes", rec.Code, rec.Body.Len())
		}
	})
	if n := r.stops()["agent-a"]; n != 1 {
		t.Fatalf("agent-a got %d stop_capture commands, want 1", n)
	}
}
