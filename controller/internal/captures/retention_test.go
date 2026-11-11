// SPDX-License-Identifier: GPL-3.0-only

package captures_test

import (
	"context"
	"errors"
	"io/fs"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/captures"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

const week = 7 * 24 * time.Hour

// fakeClock is a captures.Clock that a test moves by hand. It starts at the
// wall clock, because the age of a file comes from its modification time.
type fakeClock struct {
	mu  sync.Mutex
	now time.Time
}

func newFakeClock() *fakeClock { return &fakeClock{now: time.Now()} }

func (c *fakeClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *fakeClock) Advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.now = c.now.Add(d)
}

// retentionRig returns a rig whose service keeps captures for a week by a fake
// clock. The ticker interval is long, so only the test calls Sweep.
func retentionRig(t *testing.T, ids ...string) (*rig, *fakeClock) {
	t.Helper()
	clock := newFakeClock()
	r := newRig(t, ids, rigOptions{mutate: func(c *captures.Config) {
		c.Retention = week
		c.Clock = clock
		c.SweepInterval = time.Hour
	}})
	return r, clock
}

func TestSweepKeepsAFileBeforeTheRetentionEndsAndDeletesItAfter(t *testing.T) {
	r, clock := retentionRig(t, "agent-a")
	c := r.start(`{"port":"eth1"}`)
	if rec := r.download(c.Id); rec.Code != http.StatusOK || rec.Body.Len() != 1000 {
		t.Fatalf("download: %d", rec.Code)
	}
	if files := r.storedFiles(); len(files) != 1 {
		t.Fatalf("stored files %v, want one", files)
	}
	ctx := context.Background()

	// Just before the retention ends the file stays and downloads.
	clock.Advance(week - time.Hour)
	if n := r.svc.Sweep(ctx); n != 0 {
		t.Fatalf("sweep before the end of the retention deleted %d captures", n)
	}
	if files := r.storedFiles(); len(files) != 1 {
		t.Fatalf("stored files %v after an early sweep, want one", files)
	}
	if rec := r.download(c.Id); rec.Code != http.StatusOK || rec.Body.Len() != 1000 {
		t.Fatalf("download after an early sweep: %d", rec.Code)
	}

	// Two hours later the retention has ended.
	clock.Advance(2 * time.Hour)
	if n := r.svc.Sweep(ctx); n != 1 {
		t.Fatalf("sweep after the end of the retention deleted %d captures, want 1", n)
	}
	if files := r.storedFiles(); len(files) != 0 {
		t.Fatalf("stored files %v after the sweep, want none", files)
	}
	problemStatus(t, r.download(c.Id), http.StatusNotFound)
	if got := r.list(); len(got) != 0 {
		t.Fatalf("list after the sweep: %+v", got)
	}
	if n := r.svc.Sweep(ctx); n != 0 {
		t.Fatalf("a second sweep deleted %d captures", n)
	}
}

func TestSweepDeletesOnlyTheExpiredCapture(t *testing.T) {
	r, clock := retentionRig(t, "agent-a")
	old := r.start(`{"port":"eth1"}`)
	if rec := r.download(old.Id); rec.Code != http.StatusOK {
		t.Fatalf("download: %d", rec.Code)
	}
	clock.Advance(week)
	fresh := r.start(`{"port":"eth1"}`)
	if rec := r.download(fresh.Id); rec.Code != http.StatusOK {
		t.Fatalf("download: %d", rec.Code)
	}
	clock.Advance(time.Hour)
	if n := r.svc.Sweep(context.Background()); n != 1 {
		t.Fatalf("sweep deleted %d captures, want 1", n)
	}
	problemStatus(t, r.download(old.Id), http.StatusNotFound)
	if rec := r.download(fresh.Id); rec.Code != http.StatusOK {
		t.Fatalf("download of the younger capture: %d", rec.Code)
	}
}

func TestSweepKeepsARunningCapture(t *testing.T) {
	r, clock := retentionRig(t, "agent-a")
	c := r.start(`{"port":"eth1"}`)
	clock.Advance(4 * week)
	if n := r.svc.Sweep(context.Background()); n != 0 {
		t.Fatalf("sweep deleted %d captures while an agent still captures", n)
	}
	if got := r.list(); len(got) != 1 || got[0].Id != c.Id {
		t.Fatalf("list: %+v", got)
	}
}

func TestSweepKeepsTheFilesOfAnOpenDownload(t *testing.T) {
	r, clock := retentionRig(t, "agent-a")
	c := r.start(`{"port":"eth1"}`)
	d, err := r.svc.Prepare(context.Background(), "run-1", c.Id, true)
	if err != nil {
		t.Fatal(err)
	}
	clock.Advance(2 * week)
	if n := r.svc.Sweep(context.Background()); n != 0 {
		t.Fatalf("sweep deleted %d captures under an open download", n)
	}
	if files := r.storedFiles(); len(files) != 1 {
		t.Fatalf("stored files %v under an open download, want one", files)
	}
	var sink countingWriter
	if n, err := d.WriteTo(&sink); err != nil || n != 1000 {
		t.Fatalf("WriteTo = %d, %v", n, err)
	}
	d.Close()
	d.Close() // a second Close must not release the capture twice
	if n := r.svc.Sweep(context.Background()); n != 1 {
		t.Fatalf("sweep after the download closed deleted %d captures, want 1", n)
	}
	if files := r.storedFiles(); len(files) != 0 {
		t.Fatalf("stored files %v, want none", files)
	}
}

type countingWriter struct{ n int }

func (w *countingWriter) Write(p []byte) (int, error) { w.n += len(p); return len(p), nil }

func TestSweepSkipsACaptureWhoseTransferIsRunning(t *testing.T) {
	r, clock := retentionRig(t, "agent-a")
	// The agent answers fetch_capture and never uploads, so the download waits
	// inside the transfer and holds the lock of the capture.
	r.agents["agent-a"].SetCaptureOptions(fakeagent.CaptureOptions{Bytes: 1000, NoUpload: true})
	c := r.start(`{"port":"eth1"}`)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := r.svc.Prepare(ctx, "run-1", c.Id, true)
		done <- err
	}()
	waitFor(t, "the fetch command", func() bool {
		for _, x := range r.agents["agent-a"].Commands() {
			if x.GetFetchCapture() != nil {
				return true
			}
		}
		return false
	})
	clock.Advance(2 * week)
	if n := r.svc.Sweep(context.Background()); n != 0 {
		t.Fatalf("sweep deleted %d captures under a transfer", n)
	}
	cancel()
	if err := <-done; err == nil {
		t.Fatal("the download of a transfer that never started succeeded")
	}
	// The transfer is over, so the next sweep may delete the capture.
	if n := r.svc.Sweep(context.Background()); n != 1 {
		t.Fatalf("sweep after the transfer deleted %d captures, want 1", n)
	}
}

func TestSweepDeletesOrphanedDirectoriesByAge(t *testing.T) {
	r, clock := retentionRig(t, "agent-a")
	root := filepath.Join(r.dir, "captures")
	mk := func(run, id string, age time.Duration) string {
		dir := filepath.Join(root, run, id)
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		f := filepath.Join(dir, "agent-a.pcapng.part")
		if err := os.WriteFile(f, []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
		when := clock.Now().Add(-age)
		for _, p := range []string{f, dir} {
			if err := os.Chtimes(p, when, when); err != nil {
				t.Fatal(err)
			}
		}
		return dir
	}
	old := mk("run-old", "cap-old", week+time.Hour)
	young := mk("run-old", "cap-young", week-time.Hour)
	if n := r.svc.Sweep(context.Background()); n != 1 {
		t.Fatalf("sweep deleted %d directories, want 1", n)
	}
	if _, err := os.Stat(old); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("the old directory remains: %v", err)
	}
	if _, err := os.Stat(young); err != nil {
		t.Fatalf("the young directory is gone: %v", err)
	}
}

func TestWithoutRetentionNothingIsDeleted(t *testing.T) {
	clock := newFakeClock()
	r := newRig(t, []string{"agent-a"}, rigOptions{mutate: func(c *captures.Config) { c.Clock = clock }})
	c := r.start(`{"port":"eth1"}`)
	if rec := r.download(c.Id); rec.Code != http.StatusOK {
		t.Fatalf("download: %d", rec.Code)
	}
	clock.Advance(100 * week)
	if n := r.svc.Sweep(context.Background()); n != 0 {
		t.Fatalf("sweep deleted %d captures without a retention", n)
	}
	if rec := r.download(c.Id); rec.Code != http.StatusOK {
		t.Fatalf("download: %d", rec.Code)
	}
}

func TestTheServiceSweepsOnItsTickerUntilClosed(t *testing.T) {
	clock := newFakeClock()
	r := newRig(t, []string{"agent-a"}, rigOptions{mutate: func(c *captures.Config) {
		c.Retention = time.Hour
		c.Clock = clock
		c.SweepInterval = 5 * time.Millisecond
	}})
	c := r.start(`{"port":"eth1"}`)
	if rec := r.download(c.Id); rec.Code != http.StatusOK {
		t.Fatalf("download: %d", rec.Code)
	}
	if files := r.storedFiles(); len(files) != 1 {
		t.Fatalf("stored files %v, want one", files)
	}
	clock.Advance(2 * time.Hour)
	waitFor(t, "the ticker to delete the capture", func() bool { return len(r.storedFiles()) == 0 })
	problemStatus(t, r.download(c.Id), http.StatusNotFound)

	// After Close no sweep runs, even for a directory that has expired.
	r.svc.Close()
	r.svc.Close() // a second Close is harmless
	orphan := filepath.Join(r.dir, "captures", "run-x", "cap-x")
	if err := os.MkdirAll(orphan, 0o700); err != nil {
		t.Fatal(err)
	}
	old := clock.Now().Add(-2 * time.Hour)
	if err := os.Chtimes(orphan, old, old); err != nil {
		t.Fatal(err)
	}
	time.Sleep(100 * time.Millisecond)
	if _, err := os.Stat(orphan); err != nil {
		t.Fatalf("a sweep ran after Close: %v", err)
	}
}
