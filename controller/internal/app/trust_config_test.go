// SPDX-License-Identifier: GPL-3.0-only

package app

import "testing"

// TestParseConfigReadsTheSigningSettings covers the settings of CTL-28: the
// defaults leave signatures unenforced, and both keys read from YAML.
func TestParseConfigReadsTheSigningSettings(t *testing.T) {
	def, err := ParseConfig(nil)
	if err != nil {
		t.Fatal(err)
	}
	if def.TrustStoreFile != "" || def.AllowUnsignedModules {
		t.Fatalf("defaults = %q, %v, want no trust store and no unsigned allowance", def.TrustStoreFile, def.AllowUnsignedModules)
	}
	cfg, err := ParseConfig([]byte("trust_store_file: /etc/fps/trust.json\nallow_unsigned_modules: true\n"))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.TrustStoreFile != "/etc/fps/trust.json" || !cfg.AllowUnsignedModules {
		t.Fatalf("parsed = %q, %v", cfg.TrustStoreFile, cfg.AllowUnsignedModules)
	}
}
