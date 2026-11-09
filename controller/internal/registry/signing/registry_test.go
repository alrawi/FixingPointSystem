// SPDX-License-Identifier: GPL-3.0-only

package signing_test

import (
	"archive/tar"
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/registry"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry/signing"
)

// TestArchiveDigestEqualsTreeDigest checks that the digest of an archive is
// the digest that the registry computes for the unpacked tree, so a signature
// made over an archive is a signature over the digest that the index and plans
// carry. The package keeps its own copy of the rule so that the agent side can
// follow it, and this test keeps the two copies equal.
func TestArchiveDigestEqualsTreeDigest(t *testing.T) {
	dir := t.TempDir()
	var buf bytes.Buffer
	tw := tar.NewWriter(&buf)
	for _, f := range []file{
		{"manifest.yaml", "name: demo\n"},
		{"scenarios/a.yaml", "scenario: a\n"},
		{"scenarios/b.yaml", "scenario: b\n"},
		{"assets/body.bin", "0123456789"},
		{"assets/empty", ""},
		{"Zeta.txt", "upper case sorts before lower case"},
		{"a-b.txt", "dash"},
		{"a/b.txt", "slash sorts after dash"},
	} {
		if err := tw.WriteHeader(&tar.Header{Name: f.name, Typeflag: tar.TypeReg, Mode: 0o644, Size: int64(len(f.body))}); err != nil {
			t.Fatal(err)
		}
		if _, err := tw.Write([]byte(f.body)); err != nil {
			t.Fatal(err)
		}
		p := filepath.Join(dir, filepath.FromSlash(f.name))
		if err := os.MkdirAll(filepath.Dir(p), 0o750); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(f.body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	want, err := registry.TreeDigest(dir)
	if err != nil {
		t.Fatal(err)
	}
	got, err := signing.ArchiveDigest(&buf, signing.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("ArchiveDigest = %s, TreeDigest = %s", got, want)
	}
}

// TestDefaultLimitsMatchTheRegistry keeps the two default limits equal.
func TestDefaultLimitsMatchTheRegistry(t *testing.T) {
	if signing.DefaultLimits.MaxBytes != registry.DefaultLimits.MaxBytes || signing.DefaultLimits.MaxFiles != registry.DefaultLimits.MaxFiles {
		t.Fatalf("signing %+v, registry %+v", signing.DefaultLimits, registry.DefaultLimits)
	}
}
