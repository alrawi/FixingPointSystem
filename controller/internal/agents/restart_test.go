// SPDX-License-Identifier: GPL-3.0-only

package agents_test

import (
	"errors"
	"path/filepath"
	"slices"
	"testing"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/pki"
)

func TestSeed(t *testing.T) {
	h := newHarness(t, nil)
	if _, err := h.mgr.Enroll("agent-kept", map[string]string{"site": "lab"}); err != nil {
		t.Fatal(err)
	}
	added := h.mgr.Seed([]string{"agent-kept", "agent-new", "a/b", "agent-new"})
	if !slices.Equal(added, []string{"agent-new"}) {
		t.Fatalf("Seed added %v, want [agent-new]", added)
	}
	if ag := h.state(t, "agent-kept"); ag.Labels["site"] != "lab" {
		t.Fatalf("Seed replaced the labels of a known agent: %v", ag.Labels)
	}
	if ag := h.state(t, "agent-new"); !ag.Enrolled || ag.State != agents.StateOffline {
		t.Fatalf("seeded agent = %+v, want enrolled and offline", ag)
	}
	// A seeded identity registers although agent_auto_enroll is off, and a
	// stranger still does not.
	h.start(t, fakeagent.Options{ID: "agent-new"})
	wantState(t, h, "agent-new", agents.StateOnline)
	_, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: "stranger", Dial: h.dial})
	if code := status.Code(errors.Unwrap(err)); code != codes.PermissionDenied {
		t.Fatalf("a stranger got %v, want PermissionDenied", err)
	}
}

// TestRestartRebuildsInventoryFromPKIState is the acceptance test of CTL-29.
// It enrolls agents, builds a new authority and manager from the same state
// directory, and seeds the manager the way the controller does at start.
func TestRestartRebuildsInventoryFromPKIState(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "pki")
	pkiClock := newFakeClock()
	open := func() *pki.Authority {
		t.Helper()
		a, err := pki.Open(pki.Config{Dir: dir, Now: pkiClock.Now, ClientLifetime: time.Hour})
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	enroll := func(a *pki.Authority, id string) {
		t.Helper()
		tok, _, err := a.NewToken(id, time.Minute)
		if err != nil {
			t.Fatal(err)
		}
		_, csr, err := pki.NewCSR(id)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := a.Enroll(tok, csr); err != nil {
			t.Fatal(err)
		}
	}
	registers := func(h *harness, id string) error {
		_, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: id, Dial: h.dial})
		return err
	}

	before := open()
	for _, id := range []string{"agent-live", "agent-revoked"} {
		enroll(before, id)
	}
	if _, err := before.Revoke("agent-revoked"); err != nil {
		t.Fatal(err)
	}

	// Restart: a new authority and a new manager with agent_auto_enroll off.
	after := open()
	h := newHarness(t, func(c *agents.Config) { c.AutoEnroll = false })
	h.mgr.Seed(after.ValidIdentities())

	h.start(t, fakeagent.Options{ID: "agent-live"})
	wantState(t, h, "agent-live", agents.StateOnline)

	for _, id := range []string{"agent-revoked", "agent-never-enrolled"} {
		if err := registers(h, id); status.Code(errors.Unwrap(err)) != codes.PermissionDenied {
			t.Fatalf("%s registered after the restart: %v", id, err)
		}
	}

	// A certificate that expired before the restart does not seed the inventory.
	pkiClock.Advance(2 * time.Hour)
	late := open()
	h2 := newHarness(t, nil)
	if added := h2.mgr.Seed(late.ValidIdentities()); len(added) != 0 {
		t.Fatalf("an expired certificate seeded %v", added)
	}
	if err := registers(h2, "agent-live"); status.Code(errors.Unwrap(err)) != codes.PermissionDenied {
		t.Fatalf("agent-live registered with an expired certificate: %v", err)
	}
}
