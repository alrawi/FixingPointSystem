// SPDX-License-Identifier: GPL-3.0-only

package pki

import "slices"

// ValidIdentities returns the agent identities that hold at least one
// certificate which the authority still accepts, sorted and without
// duplicates. A certificate counts when the CA issued it, Revoke did not end
// its identity, and the clock of the authority is before its NotAfter time,
// which is the rule that Authorize applies to the validity period. The
// controller seeds the agent inventory with this list at start, because the
// inventory lives in memory and the PKI state on disk. An identity with only
// an unused enrollment token is not listed, because it holds no certificate.
func (a *Authority) ValidIdentities() []string {
	a.mu.Lock()
	defer a.mu.Unlock()
	now := a.now()
	seen := map[string]struct{}{}
	for _, c := range a.certs {
		if now.Before(c.NotAfter) {
			seen[c.AgentID] = struct{}{}
		}
	}
	out := make([]string, 0, len(seen))
	for id := range seen {
		out = append(out, id)
	}
	slices.Sort(out)
	return out
}
