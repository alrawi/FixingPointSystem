// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"errors"
	"testing"
)

func TestCheckLicense(t *testing.T) {
	tests := []struct {
		expr string
		want error
	}{
		{"GPL-3.0-only", nil},
		{"gpl-3.0-or-later", nil},
		{"MIT", nil},
		{"Apache-2.0 WITH LLVM-exception", nil},
		{"MIT OR GPL-2.0-only", nil},
		{"(MIT AND BSD-3-Clause) OR Apache-2.0", nil},
		{"LGPL-2.1-or-later AND CC0-1.0", nil},
		{"GPL-2.0-only", ErrLicenseIncompatible},
		{"MIT AND EPL-2.0", ErrLicenseIncompatible},
		{"GPL-2.0-only OR CDDL-1.0", ErrLicenseIncompatible},
		{"Proprietary", ErrLicenseUnknown},
		{"LicenseRef-acme", ErrLicenseUnknown},
		{"MIT OR LicenseRef-acme", ErrLicenseUnknown},
		{"GPL-3.0", ErrLicenseUnknown},
		{"GPL-2.0+", ErrLicenseUnknown},
		{"Apache-2.0 WITH Made-up-exception", ErrLicenseUnknown},
		{"MIT AND", ErrLicenseUnknown},
		{"(MIT", ErrLicenseUnknown},
		{"MIT BSD-3-Clause", ErrLicenseUnknown},
		{"", ErrLicenseUnknown},
		{"mit or apache-2.0", ErrLicenseUnknown}, // operators are upper case
	}
	for _, tt := range tests {
		t.Run(tt.expr, func(t *testing.T) {
			err := CheckLicense(tt.expr)
			if tt.want == nil {
				if err != nil {
					t.Fatalf("CheckLicense(%q) = %v, want nil", tt.expr, err)
				}
				return
			}
			if !errors.Is(err, tt.want) {
				t.Fatalf("CheckLicense(%q) = %v, want %v", tt.expr, err, tt.want)
			}
		})
	}
}

// FuzzCheckLicense checks that arbitrary expressions never panic and always
// give one of the two sentinel errors.
func FuzzCheckLicense(f *testing.F) {
	for _, s := range []string{"MIT", "(MIT OR (GPL-3.0-only AND", "A WITH", ")("} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, expr string) {
		err := CheckLicense(expr)
		if err != nil && !errors.Is(err, ErrLicenseUnknown) && !errors.Is(err, ErrLicenseIncompatible) {
			t.Fatalf("CheckLicense(%q) = %v without a sentinel", expr, err)
		}
	})
}

func TestCompareVersions(t *testing.T) {
	ordered := []string{
		"0.1.0", "0.2.0-alpha", "0.2.0-alpha.1", "0.2.0-alpha.beta", "0.2.0-beta.2",
		"0.2.0-beta.11", "0.2.0-rc.1", "0.2.0", "0.10.0", "1.0.0", "10.0.0",
	}
	for i := range ordered {
		for j := range ordered {
			got := compareVersions(ordered[i], ordered[j])
			want := 0
			if i < j {
				want = -1
			} else if i > j {
				want = 1
			}
			if sign(got) != want {
				t.Errorf("compareVersions(%s, %s) = %d, want sign %d", ordered[i], ordered[j], got, want)
			}
		}
	}
}

func sign(n int) int {
	switch {
	case n < 0:
		return -1
	case n > 0:
		return 1
	}
	return 0
}
