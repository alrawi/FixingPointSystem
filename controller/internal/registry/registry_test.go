// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Paths of contract files, relative to this package directory.
const (
	contractsDir    = "../../../contracts"
	examplesDir     = contractsDir + "/examples/modules"
	examplePlanPath = contractsDir + "/examples/plans/http_dns_smoke.json"
	schemaPath      = contractsDir + "/schemas/module-manifest.schema.json"
)

// readTree returns the files of the module tree at dir by relative path.
func readTree(t *testing.T, dir string) map[string][]byte {
	t.Helper()
	files := map[string][]byte{}
	fsys := os.DirFS(dir)
	err := fs.WalkDir(fsys, ".", func(p string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		data, err := fs.ReadFile(fsys, p)
		files[p] = data
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	return files
}

// exampleTree returns the files of an example module, with edit applied to
// the manifest text when edit is not nil.
func exampleTree(t *testing.T, name string, edit func(string) string) map[string][]byte {
	t.Helper()
	files := readTree(t, filepath.Join(examplesDir, name))
	if edit != nil {
		files[ManifestFile] = []byte(edit(string(files[ManifestFile])))
	}
	return files
}

// tarFiles builds a tar archive, gzip compressed when gz is true.
func tarFiles(t *testing.T, files map[string][]byte, gz bool) []byte {
	t.Helper()
	var buf bytes.Buffer
	var tw *tar.Writer
	var zw *gzip.Writer
	if gz {
		zw = gzip.NewWriter(&buf)
		tw = tar.NewWriter(zw)
	} else {
		tw = tar.NewWriter(&buf)
	}
	for name, data := range files {
		if err := tw.WriteHeader(&tar.Header{Name: name, Mode: 0o644, Size: int64(len(data)), Typeflag: tar.TypeReg}); err != nil {
			t.Fatal(err)
		}
		if _, err := tw.Write(data); err != nil {
			t.Fatal(err)
		}
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if zw != nil {
		if err := zw.Close(); err != nil {
			t.Fatal(err)
		}
	}
	return buf.Bytes()
}

func writeTree(t *testing.T, files map[string][]byte) string {
	t.Helper()
	dir := t.TempDir()
	for name, data := range files {
		p := filepath.Join(dir, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(p), 0o750); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, data, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func openRegistry(t *testing.T) *Registry {
	t.Helper()
	r, err := Open(filepath.Join(t.TempDir(), "registry"), Limits{})
	if err != nil {
		t.Fatal(err)
	}
	return r
}

// planDigests returns the module digests of the example plan by name.
func planDigests(t *testing.T) map[string]string {
	t.Helper()
	data, err := os.ReadFile(examplePlanPath)
	if err != nil {
		t.Fatal(err)
	}
	var plan struct {
		Modules []struct{ Name, Version, Digest string }
	}
	if err := json.Unmarshal(data, &plan); err != nil {
		t.Fatal(err)
	}
	out := map[string]string{}
	for _, m := range plan.Modules {
		out[m.Name] = m.Digest
	}
	return out
}

func TestEmbeddedSchemaMatchesContract(t *testing.T) {
	want, err := os.ReadFile(schemaPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(manifestSchemaJSON, want) {
		t.Fatal("schema/module-manifest.schema.json differs from the contract, run go generate ./internal/registry")
	}
}

func TestTreeDigestMatchesExamplePlan(t *testing.T) {
	want := planDigests(t)
	for _, name := range []string{"http1", "dns"} {
		got, err := TreeDigest(filepath.Join(examplesDir, name))
		if err != nil {
			t.Fatal(err)
		}
		if got != want[name] {
			t.Errorf("TreeDigest(%s) = %s, want %s from the example plan", name, got, want[name])
		}
	}
}

func TestTreeDigestRejectsSymlink(t *testing.T) {
	dir := writeTree(t, map[string][]byte{"a.txt": []byte("a")})
	if err := os.Symlink("a.txt", filepath.Join(dir, "b.txt")); err != nil {
		t.Skip("symlinks unavailable:", err)
	}
	if _, err := TreeDigest(dir); !errors.Is(err, ErrSymlink) {
		t.Fatalf("TreeDigest = %v, want ErrSymlink", err)
	}
}

func TestParseManifestExamples(t *testing.T) {
	for _, name := range []string{"http1", "dns"} {
		data, err := os.ReadFile(filepath.Clean(filepath.Join(examplesDir, name, ManifestFile)))
		if err != nil {
			t.Fatal(err)
		}
		m, err := ParseManifest(data)
		if err != nil {
			t.Fatalf("ParseManifest(%s) = %v", name, err)
		}
		if m.Name != name || m.Version != "0.1.0" || len(m.Functions) == 0 {
			t.Errorf("ParseManifest(%s) = %+v", name, m)
		}
	}
}

// findingPaths returns the paths of a *ValidationError, or fails the test.
func findingPaths(t *testing.T, err error) []string {
	t.Helper()
	var verr *ValidationError
	if !errors.As(err, &verr) {
		t.Fatalf("error %v is not a *ValidationError", err)
	}
	var paths []string
	for _, f := range verr.Findings {
		paths = append(paths, f.Path)
	}
	return paths
}

func TestParseManifestFindings(t *testing.T) {
	base, err := os.ReadFile(filepath.Join(examplesDir, "http1", ManifestFile))
	if err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name string
		old  string
		new  string
		path string
	}{
		{"unknown license", "license: GPL-3.0-only", "license: LicenseRef-acme", "/license"},
		{"incompatible license", "license: GPL-3.0-only", "license: GPL-2.0-only", "/license"},
		{"bad version", "version: 0.1.0", "version: one", "/version"},
		{"unknown field", "kind: Module", "kind: Module\ncolor: blue", ""},
		{"escaping scenario path", "scenarios/web_get.yaml", "../web_get.yaml", "/scenarios/0"},
		{"reserved name", "name: http1", "name: vars", "/name"},
		{"reserved name profiles", "name: http1", "name: profiles", "/name"},
		{"reserved name dist", "name: http1", "name: dist", "/name"},
		{"reserved name all", "name: http1", "name: all", "/name"},
		{"capture without consent", "source: specification", "source: capture", "/provenance"},
		{"duplicate function", "  - name: response", "  - name: request", "/functions/1/name"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			text := strings.Replace(string(base), tt.old, tt.new, 1)
			if text == string(base) {
				t.Fatalf("edit %q did not apply", tt.old)
			}
			_, err := ParseManifest([]byte(text))
			paths := findingPaths(t, err)
			for _, p := range paths {
				if p == tt.path {
					return
				}
			}
			t.Fatalf("findings %v do not name %q (%v)", paths, tt.path, err)
		})
	}
}

func TestParseManifestRenderHook(t *testing.T) {
	base, err := os.ReadFile(filepath.Join(examplesDir, "dns", ManifestFile))
	if err != nil {
		t.Fatal(err)
	}
	text := string(base) + "extension:\n  wasm: extension.wasm\n  hooks: [on_open]\n"
	paths := findingPaths(t, func() error { _, err := ParseManifest([]byte(text)); return err }())
	if len(paths) != 1 || paths[0] != "/extension/hooks" {
		t.Fatalf("findings %v, want /extension/hooks", paths)
	}
}

func TestParseManifestRejectsGarbage(t *testing.T) {
	for _, in := range []string{"", "[", "- a\n- b\n", "{1: 2}", "\x00\xff"} {
		if _, err := ParseManifest([]byte(in)); err == nil {
			t.Errorf("ParseManifest(%q) succeeded", in)
		}
	}
}

// FuzzParseManifest checks that untrusted manifests never panic.
func FuzzParseManifest(f *testing.F) {
	data, err := os.ReadFile(filepath.Join(examplesDir, "dns", ManifestFile))
	if err != nil {
		f.Fatal(err)
	}
	f.Add(data)
	f.Add([]byte("name: [1, {a: b}]"))
	f.Fuzz(func(_ *testing.T, in []byte) {
		_, _ = ParseManifest(in)
	})
}

func TestInstallExampleModules(t *testing.T) {
	ctx := context.Background()
	r := openRegistry(t)
	want := planDigests(t)
	e, err := r.InstallDir(ctx, filepath.Join(examplesDir, "http1"))
	if err != nil {
		t.Fatalf("InstallDir(http1) = %v", err)
	}
	if e.Digest != want["http1"] {
		t.Errorf("http1 digest %s, want %s", e.Digest, want["http1"])
	}
	if got := strings.Join(e.Scenarios, ","); got != "web_get,web_browse,web_page" {
		t.Errorf("http1 scenarios %s", got)
	}
	e, err = r.Install(ctx, bytes.NewReader(tarFiles(t, exampleTree(t, "dns", nil), true)))
	if err != nil {
		t.Fatalf("Install(dns archive) = %v", err)
	}
	if e.Digest != want["dns"] {
		t.Errorf("dns digest %s, want %s", e.Digest, want["dns"])
	}
	if err := r.Verify(ctx, "dns", "0.1.0"); err != nil {
		t.Error(err)
	}

	list := r.List(ctx, "")
	if len(list) != 2 || list[0].Name != "dns" || list[1].Name != "http1" {
		t.Fatalf("List = %+v", list)
	}
	if got := r.List(ctx, "transport"); len(got) != 0 {
		t.Errorf("List(transport) = %+v", got)
	}

	// The index survives a reopen.
	r2, err := Open(r.Root(), Limits{})
	if err != nil {
		t.Fatal(err)
	}
	m, err := r2.Manifest(ctx, "http1", "0.1.0")
	if err != nil {
		t.Fatal(err)
	}
	if m.License != "GPL-3.0-only" || len(m.Functions) != 2 {
		t.Errorf("Manifest(http1) = %+v", m)
	}

	if _, err := r2.InstallDir(ctx, filepath.Join(examplesDir, "http1")); !errors.Is(err, ErrModuleExists) {
		t.Errorf("second install = %v, want ErrModuleExists", err)
	}
	if err := r2.Delete(ctx, "http1", "0.1.0"); err != nil {
		t.Fatal(err)
	}
	if _, err := r2.Manifest(ctx, "http1", "0.1.0"); !errors.Is(err, ErrModuleNotFound) {
		t.Errorf("Manifest after delete = %v, want ErrModuleNotFound", err)
	}
	if err := r2.Delete(ctx, "http1", "0.1.0"); !errors.Is(err, ErrModuleNotFound) {
		t.Errorf("second delete = %v, want ErrModuleNotFound", err)
	}
	if _, err := os.Stat(r2.Dir("http1", "0.1.0")); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("tree remains after delete: %v", err)
	}
	staging, err := os.ReadDir(filepath.Join(r2.Root(), "staging"))
	if err != nil || len(staging) != 0 {
		t.Errorf("staging holds %d entries, err %v", len(staging), err)
	}
}

func TestLatest(t *testing.T) {
	ctx := context.Background()
	r := openRegistry(t)
	for _, v := range []string{"0.10.0", "0.2.0", "0.10.0-rc.1"} {
		files := exampleTree(t, "dns", func(s string) string { return strings.Replace(s, "version: 0.1.0", "version: "+v, 1) })
		if _, err := r.Install(ctx, bytes.NewReader(tarFiles(t, files, false))); err != nil {
			t.Fatal(err)
		}
	}
	var l Lookup = r
	m, err := l.Latest(ctx, "dns")
	if err != nil || m.Version != "0.10.0" {
		t.Fatalf("Latest(dns) = %s, %v", m.Version, err)
	}
	if _, err := l.Latest(ctx, "http1"); !errors.Is(err, ErrModuleNotFound) {
		t.Errorf("Latest(http1) = %v", err)
	}
	if _, err := l.Manifest(ctx, "dns", "0.2.0"); err != nil {
		t.Error(err)
	}
}

func TestInstallRejects(t *testing.T) {
	ctx := context.Background()
	license := func(id string) func(string) string {
		return func(s string) string { return strings.Replace(s, "license: GPL-3.0-only", "license: "+id, 1) }
	}
	tests := []struct {
		name  string
		files map[string][]byte
		want  error  // sentinel, or nil for a *ValidationError
		path  string // finding path for a *ValidationError
	}{
		{"unknown license", exampleTree(t, "http1", license("LicenseRef-acme")), nil, "/license"},
		{"incompatible license", exampleTree(t, "dns", license("BUSL-1.1")), nil, "/license"},
		{"missing scenario", func() map[string][]byte {
			f := exampleTree(t, "http1", nil)
			delete(f, "scenarios/web_page.yaml")
			return f
		}(), nil, "file:scenarios/web_page.yaml"},
		{"duplicate scenario name", func() map[string][]byte {
			f := exampleTree(t, "dns", nil)
			f["scenarios/resolve.yaml"] = f["scenarios/lookup.yaml"]
			return f
		}(), nil, "/scenarios/1"},
		{"module named dist", exampleTree(t, "http1", func(s string) string {
			return strings.Replace(s, "name: http1", "name: dist", 1)
		}), nil, "/name"},
		{"misspelled expected_bytes key", exampleTree(t, "http1", func(s string) string {
			return strings.Replace(s, "  web_get: 17244", "  web_gte: 17244", 1)
		}), nil, "/expected_bytes/web_gte"},
		{"no manifest", map[string][]byte{"scenarios/a.yaml": []byte("scenario: a\n")}, ErrArchive, ""},
		{"absolute path", map[string][]byte{ManifestFile: []byte("x"), "/etc/passwd": []byte("x")}, ErrArchive, ""},
		{"parent path", map[string][]byte{ManifestFile: []byte("x"), "a/../../b": []byte("x")}, ErrArchive, ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := openRegistry(t)
			_, err := r.Install(ctx, bytes.NewReader(tarFiles(t, tt.files, false)))
			if tt.want != nil {
				if !errors.Is(err, tt.want) {
					t.Fatalf("Install = %v, want %v", err, tt.want)
				}
			} else {
				paths := findingPaths(t, err)
				if len(paths) == 0 || paths[0] != tt.path {
					t.Fatalf("findings %v, want %s (%v)", paths, tt.path, err)
				}
			}
			if got := r.List(ctx, ""); len(got) != 0 {
				t.Errorf("a rejected install left %+v", got)
			}
		})
	}
}

func TestInstallRejectsLinksAndLimits(t *testing.T) {
	ctx := context.Background()
	r := openRegistry(t)
	var buf bytes.Buffer
	tw := tar.NewWriter(&buf)
	if err := tw.WriteHeader(&tar.Header{Name: "evil", Typeflag: tar.TypeSymlink, Linkname: "/etc/passwd"}); err != nil {
		t.Fatal(err)
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := r.Install(ctx, &buf); !errors.Is(err, ErrArchive) {
		t.Errorf("symlink entry = %v, want ErrArchive", err)
	}
	if _, err := r.Install(ctx, strings.NewReader("not a tar archive at all")); !errors.Is(err, ErrArchive) {
		t.Errorf("garbage = %v, want ErrArchive", err)
	}

	small, err := Open(filepath.Join(t.TempDir(), "small"), Limits{MaxBytes: 1024, MaxFiles: 3})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := small.Install(ctx, bytes.NewReader(tarFiles(t, exampleTree(t, "http1", nil), false))); !errors.Is(err, ErrArchiveTooLarge) {
		t.Errorf("large archive = %v, want ErrArchiveTooLarge", err)
	}
}

func TestOpenRejectsBadIndex(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, IndexFile), []byte(`{"version": 9, "modules": []}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Open(root, Limits{}); err == nil {
		t.Error("Open accepted index version 9")
	}
	if err := os.WriteFile(filepath.Join(root, IndexFile), []byte(`{`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Open(root, Limits{}); err == nil {
		t.Error("Open accepted a truncated index")
	}
}

func TestUnknownEstimateKeys(t *testing.T) {
	scenarios := map[string]string{"a": "scenarios/a.yaml", "b": "scenarios/b.yaml"}
	if got := unknownEstimateKeys(map[string]int64{"a": 1, "b": 2}, scenarios); len(got) != 0 {
		t.Errorf("keys that name scenarios gave %v", got)
	}
	got := unknownEstimateKeys(map[string]int64{"a": 1, "z": 2, "c": 3}, scenarios)
	if len(got) != 2 || got[0].Path != "/expected_bytes/c" || got[1].Path != "/expected_bytes/z" {
		t.Errorf("findings = %v, want c and z in order", got)
	}
}
