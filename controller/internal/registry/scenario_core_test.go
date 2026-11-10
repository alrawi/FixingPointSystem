// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"strings"
	"testing"
)

// TestCoreCallResultTypes checks that int_bytes and now carry their result
// types into an outer dist call, which is the only place where the registry
// uses the type of a core call. int_bytes gives bytes, and now gives the type
// of its literal format, or any for a format that is not a known literal.
func TestCoreCallResultTypes(t *testing.T) {
	tests := []struct {
		name string
		expr string
		want string // substring of the only finding, empty for none
	}{
		{"int_bytes is bytes", "dist.uniform(int_bytes(258, 2), 5)", "parameter 'low' of 'dist.uniform' takes float, got bytes"},
		{"int_bytes with keywords is bytes", "dist.uniform(5, int_bytes(value=1, width=4, endianness='little'))", "parameter 'high' of 'dist.uniform' takes float, got bytes"},
		{"now epoch_s is int", "dist.uniform(now('epoch_s'), now('epoch_s'))", ""},
		{"now epoch_ms is int", "dist.uniform(now('epoch_ms'), 9999999999999)", ""},
		{"now http_date is text", "dist.uniform(now('http_date'), 5)", "parameter 'low' of 'dist.uniform' takes float, got text"},
		{"now with a keyword format", "dist.uniform(5, now(format='http_date'))", "parameter 'high' of 'dist.uniform' takes float, got text"},
		{"now with an unknown format is any", "dist.uniform(now('rfc3339'), 5)", ""},
		{"now with a variable format is any", "dist.uniform(now(vars.fmt), 5)", ""},
		{"fill stays text", "dist.uniform(fill(3), 5)", "got text"},
		{"rand_int stays int", "dist.uniform(rand_int(1, 5), 5)", ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := checkExpression(tt.expr, scenarioScope{})
			switch {
			case tt.want == "" && len(got) != 0:
				t.Fatalf("findings %q, want none", got)
			case tt.want != "" && (len(got) != 1 || !strings.Contains(got[0], tt.want)):
				t.Fatalf("findings %q, want one with %q", got, tt.want)
			}
		})
	}
}
