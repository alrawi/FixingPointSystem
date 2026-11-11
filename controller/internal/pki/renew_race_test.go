// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"errors"
	"testing"
)

// TestRenewCannotOutliveARevocation runs the interleaving of issue #450:
// Renew authorizes the presented certificate, Revoke ends the identity, and
// only then does Renew reach issuance. The renewal must fail, no certificate of
// the identity may remain, and the state on disk must agree after a restart.
func TestRenewCannotOutliveARevocation(t *testing.T) {
	clk := newClock()
	dir := t.TempDir()
	a, err := Open(Config{Dir: dir, Now: clk.Now})
	if err != nil {
		t.Fatal(err)
	}
	_, enrolled := enrollAgent(t, a, "agent-race")
	old := parseCert(t, enrolled.CertPEM)
	_, csr, err := NewCSR("agent-race")
	if err != nil {
		t.Fatal(err)
	}

	revoked := false
	renewHook = func() {
		n, err := a.Revoke("agent-race")
		if err != nil || n != 1 {
			t.Errorf("Revoke in the gap: %d, %v", n, err)
		}
		revoked = true
	}
	t.Cleanup(func() { renewHook = nil })

	iss, err := a.Renew(old, csr)
	if !revoked {
		t.Fatal("the hook did not run between authorization and issuance")
	}
	if !errors.Is(err, ErrRevoked) {
		t.Fatalf("Renew after a revocation in its gap issued %v (error %v); want ErrRevoked", iss != nil, err)
	}
	if got := a.certCount("agent-race"); got != 0 {
		t.Fatalf("%d certificates of the revoked identity remain", got)
	}

	// The persisted state agrees after a restart.
	renewHook = nil
	b, err := Open(Config{Dir: dir, Now: clk.Now})
	if err != nil {
		t.Fatal(err)
	}
	if got := b.certCount("agent-race"); got != 0 {
		t.Fatalf("after a restart %d certificates of the revoked identity remain", got)
	}
	if err := b.Authorize(old); !errors.Is(err, ErrRevoked) {
		t.Fatalf("the old certificate after a restart: %v", err)
	}
}

// TestRenewWithoutARevocationStillWorks checks that the recheck under the lock
// leaves an ordinary renewal alone, the hook running in the same place.
func TestRenewWithoutARevocationStillWorks(t *testing.T) {
	clk := newClock()
	a := newAuthority(t, clk)
	_, enrolled := enrollAgent(t, a, "agent-calm")
	old := parseCert(t, enrolled.CertPEM)
	_, csr, err := NewCSR("agent-calm")
	if err != nil {
		t.Fatal(err)
	}
	ran := false
	renewHook = func() { ran = true }
	t.Cleanup(func() { renewHook = nil })
	iss, err := a.Renew(old, csr)
	if err != nil || !ran {
		t.Fatalf("Renew: %v (hook ran: %v)", err, ran)
	}
	if err := a.Authorize(parseCert(t, iss.CertPEM)); err != nil {
		t.Fatalf("the renewed certificate: %v", err)
	}
}

// certCount reports how many certificates of agentID the authority accepts.
func (a *Authority) certCount(agentID string) int {
	a.mu.Lock()
	defer a.mu.Unlock()
	n := 0
	for _, r := range a.certs {
		if r.AgentID == agentID {
			n++
		}
	}
	return n
}
