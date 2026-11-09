// SPDX-License-Identifier: GPL-3.0-only

package app

import (
	"testing"
	"time"
)

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

// TestParseConfigReadsCaptureRetention covers the setting of CTL-32: seven days
// by default, and a Go duration from YAML.
func TestParseConfigReadsCaptureRetention(t *testing.T) {
	def, err := ParseConfig(nil)
	if err != nil {
		t.Fatal(err)
	}
	if time.Duration(def.CaptureRetention) != 7*24*time.Hour {
		t.Fatalf("default CaptureRetention = %v, want 168h", time.Duration(def.CaptureRetention))
	}
	cfg, err := ParseConfig([]byte("capture_retention: 36h\n"))
	if err != nil {
		t.Fatal(err)
	}
	if time.Duration(cfg.CaptureRetention) != 36*time.Hour {
		t.Fatalf("CaptureRetention = %v, want 36h", time.Duration(cfg.CaptureRetention))
	}
}
