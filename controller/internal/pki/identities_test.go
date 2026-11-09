// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"path/filepath"
	"slices"
	"testing"
	"time"
)

func TestValidIdentities(t *testing.T) {
	clk := newClock()
	dir := filepath.Join(t.TempDir(), "pki")
	a, err := Open(Config{Dir: dir, Now: clk.Now, ClientLifetime: time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	if got := a.ValidIdentities(); len(got) != 0 {
		t.Fatalf("a new authority lists %v", got)
	}
	enrollAgent(t, a, "agent-b")
	enrollAgent(t, a, "agent-a")
	enrollAgent(t, a, "agent-a") // a second certificate of one identity lists it once
	enrollAgent(t, a, "agent-revoked")
	if _, err := a.Revoke("agent-revoked"); err != nil {
		t.Fatal(err)
	}
	if _, _, err := a.NewToken("agent-token-only", time.Minute); err != nil {
		t.Fatal(err)
	}
	want := []string{"agent-a", "agent-b"}
	if got := a.ValidIdentities(); !slices.Equal(got, want) {
		t.Fatalf("ValidIdentities = %v, want %v", got, want)
	}

	// The list survives a restart, and a revocation stays revoked.
	b, err := Open(Config{Dir: dir, Now: clk.Now, ClientLifetime: time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	if got := b.ValidIdentities(); !slices.Equal(got, want) {
		t.Fatalf("ValidIdentities after restart = %v, want %v", got, want)
	}

	// An expired certificate does not count, from its NotAfter instant on.
	clk.Advance(time.Hour)
	if got := b.ValidIdentities(); len(got) != 0 {
		t.Fatalf("ValidIdentities after expiry = %v, want none", got)
	}
}
