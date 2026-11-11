// SPDX-License-Identifier: GPL-3.0-only

package captures

import "testing"

// TestFileToAPICarriesTheDropCauses checks CON-24: the causes an agent of
// protocol 0.2.4 reports pass through, and an older agent's file has none,
// not zeros.
func TestFileToAPICarriesTheDropCauses(t *testing.T) {
	u := func(n uint64) *uint64 { return &n }
	newer := fileToAPI(AgentFile{Agent: "a", Started: true, Stopped: true, Dropped: 6,
		DroppedOverflow: u(1), DroppedOversize: u(2), CutAtStop: u(3)})
	if newer.Dropped == nil || *newer.Dropped != 6 {
		t.Fatalf("dropped = %v", newer.Dropped)
	}
	for name, got := range map[string]*int{"overflow": newer.DroppedOverflow, "oversize": newer.DroppedOversize, "cut_at_stop": newer.CutAtStop} {
		want := map[string]int{"overflow": 1, "oversize": 2, "cut_at_stop": 3}[name]
		if got == nil || *got != want {
			t.Errorf("%s = %v, want %d", name, got, want)
		}
	}
	older := fileToAPI(AgentFile{Agent: "b", Started: true, Stopped: true, Dropped: 4})
	if older.DroppedOverflow != nil || older.DroppedOversize != nil || older.CutAtStop != nil {
		t.Errorf("an agent without the causes gave %v %v %v", older.DroppedOverflow, older.DroppedOversize, older.CutAtStop)
	}
	if older.Dropped == nil || *older.Dropped != 4 {
		t.Errorf("older dropped = %v", older.Dropped)
	}
}
