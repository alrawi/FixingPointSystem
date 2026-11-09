// SPDX-License-Identifier: GPL-3.0-only

package captures

import (
	"context"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// sweepLoop sweeps once at start, which removes what an earlier process left
// behind, and then at every tick, until ctx ends.
func (s *Service) sweepLoop(ctx context.Context) {
	defer close(s.sweepDone)
	s.Sweep(ctx)
	t := time.NewTicker(s.cfg.SweepInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			s.Sweep(ctx)
		}
	}
}

// Sweep deletes every capture whose newest file is older than Retention by
// the time of the clock, and returns how many it deleted. A deleted capture
// is gone: its download and its entry in the list answer as for an unknown
// capture. Sweep keeps
//
//   - a capture that still runs, because an agent still captures into it,
//   - a capture whose download holds the files open or whose transfer is in
//     progress, which it finds by the lock that Prepare holds, and tries
//     again at the next sweep.
//
// It also removes the directories that no capture of this process owns, such
// as those of an earlier process, by the age of their files. Sweep does
// nothing when Retention is zero or less. The service calls it on its own
// ticker, and tests call it directly.
func (s *Service) Sweep(ctx context.Context) int {
	if s.cfg.Retention <= 0 {
		return 0
	}
	cutoff := s.cfg.Clock.Now().Add(-s.cfg.Retention)
	var known []*capture
	s.mu.Lock()
	for _, cs := range s.byRun {
		known = append(known, cs...)
	}
	s.mu.Unlock()
	removed := 0
	for _, c := range known {
		if ctx.Err() != nil {
			return removed
		}
		if s.expire(c, cutoff) {
			removed++
		}
	}
	return removed + s.sweepOrphans(ctx, cutoff)
}

// expire deletes the capture c when it is older than the cutoff and nobody
// uses it. It takes c.work, which every stop and fetch of the capture holds, so
// it never deletes a file under a transfer, and it gives up when it cannot take
// the lock at once. A download that holds the files after Prepare returned
// counts in c.users.
func (s *Service) expire(c *capture, cutoff time.Time) bool {
	if !c.work.TryLock() {
		return false
	}
	defer c.work.Unlock()
	c.mu.Lock()
	skip := c.removed || c.users > 0
	c.mu.Unlock()
	if skip || c.view().State == api.CaptureStateRunning {
		return false
	}
	newest, err := newestModTime(c.dir)
	if err != nil && !errors.Is(err, fs.ErrNotExist) {
		s.log.Warn("capture retention: cannot read the capture directory", "run_id", c.runID, "capture_id", c.id, "error", err)
		return false
	}
	if newest.Before(c.started) {
		newest = c.started
	}
	if newest.After(cutoff) {
		return false
	}
	c.mu.Lock()
	c.removed = true
	c.mu.Unlock()
	s.forget(c)
	if err := os.RemoveAll(c.dir); err != nil {
		// The record is gone, so the next sweep finds the directory as an
		// orphan and tries again.
		s.log.Warn("capture retention: cannot delete the capture files", "run_id", c.runID, "capture_id", c.id, "error", err)
	}
	s.log.Info("capture deleted by retention", "run_id", c.runID, "capture_id", c.id, "retention", s.cfg.Retention.String())
	return true
}

// forget drops c from the captures of its run.
func (s *Service) forget(c *capture) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.byRun[c.runID] = slices.DeleteFunc(s.byRun[c.runID], func(x *capture) bool { return x == c })
	if len(s.byRun[c.runID]) == 0 {
		delete(s.byRun, c.runID)
	}
}

// sweepOrphans deletes the capture directories below Dir that no capture of
// this process owns and whose files are older than the cutoff, and the empty
// run directories that are older than the cutoff. It follows no symbolic link.
func (s *Service) sweepOrphans(ctx context.Context, cutoff time.Time) int {
	removed := 0
	runDirs, err := os.ReadDir(s.cfg.Dir)
	if err != nil {
		s.log.Warn("capture retention: cannot read the capture directory", "dir", s.cfg.Dir, "error", err)
		return 0
	}
	for _, rd := range runDirs {
		if !rd.IsDir() {
			continue
		}
		runPath := filepath.Join(s.cfg.Dir, rd.Name())
		capDirs, err := os.ReadDir(runPath)
		if err != nil {
			continue
		}
		for _, cd := range capDirs {
			if ctx.Err() != nil {
				return removed
			}
			if !cd.IsDir() || s.find(rd.Name(), cd.Name()) != nil {
				continue
			}
			p := filepath.Join(runPath, cd.Name())
			if newest, err := newestModTime(p); err != nil || newest.After(cutoff) {
				continue
			}
			if err := os.RemoveAll(p); err != nil {
				s.log.Warn("capture retention: cannot delete the capture files", "path", p, "error", err)
				continue
			}
			s.log.Info("capture directory deleted by retention", "run_id", rd.Name(), "capture_id", cd.Name(), "retention", s.cfg.Retention.String())
			removed++
		}
		if fi, err := os.Stat(runPath); err == nil && !fi.ModTime().After(cutoff) {
			_ = os.Remove(runPath) // fails, and keeps the directory, unless it is empty
		}
	}
	return removed
}

// newestModTime returns the newest modification time of the directory dir and
// of the entries in it.
func newestModTime(dir string) (time.Time, error) {
	fi, err := os.Lstat(dir)
	if err != nil {
		return time.Time{}, err
	}
	newest := fi.ModTime()
	entries, err := os.ReadDir(dir)
	if err != nil {
		return time.Time{}, err
	}
	for _, e := range entries {
		info, err := e.Info()
		if err != nil {
			continue
		}
		if m := info.ModTime(); m.After(newest) {
			newest = m
		}
	}
	return newest, nil
}
