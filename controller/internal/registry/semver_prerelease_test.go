// SPDX-License-Identifier: GPL-3.0-only

package registry

import "testing"

// TestPrereleaseNumericBeyondUint64 checks issue #453: semver 2.0.0 compares
// numeric prerelease identifiers numerically at any length, and orders them
// below alphanumeric ones.
func TestPrereleaseNumericBeyondUint64(t *testing.T) {
	for _, tc := range []struct{ lo, hi string }{
		// Both beyond uint64: lexically "1..." sorts first, numerically it is larger.
		{"1.0.0-99999999999999999999", "1.0.0-100000000000000000000"},
		{"1.0.0-rc.99999999999999999999", "1.0.0-rc.100000000000000000000"},
		// One inside and one beyond the uint64 range.
		{"1.0.0-18446744073709551615", "1.0.0-18446744073709551616"},
		// A numeric identifier, however long, sorts below an alphanumeric one,
		// even one whose bytes sort below the digits.
		{"1.0.0-100000000000000000000", "1.0.0--x"},
		{"1.0.0-rc.100000000000000000000", "1.0.0-rc.-1"},
		// The examples of semver 2.0.0, section 11.
		{"1.0.0-alpha", "1.0.0-alpha.1"},
		{"1.0.0-alpha.1", "1.0.0-alpha.beta"},
		{"1.0.0-alpha.beta", "1.0.0-beta"},
		{"1.0.0-beta", "1.0.0-beta.2"},
		{"1.0.0-beta.2", "1.0.0-beta.11"},
		{"1.0.0-beta.11", "1.0.0-rc.1"},
		{"1.0.0-rc.1", "1.0.0"},
	} {
		if c := compareVersions(tc.lo, tc.hi); c >= 0 {
			t.Errorf("compareVersions(%q, %q) = %d, want < 0", tc.lo, tc.hi, c)
		}
		if c := compareVersions(tc.hi, tc.lo); c <= 0 {
			t.Errorf("compareVersions(%q, %q) = %d, want > 0", tc.hi, tc.lo, c)
		}
	}
	if c := comparePrecedence("1.0.0-100000000000000000000+a", "1.0.0-100000000000000000000+b"); c != 0 {
		t.Errorf("build metadata changed precedence: %d", c)
	}
}
