// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"sync"

	"go.yaml.in/yaml/v3"

	"github.com/alrawi/FixingPointSystem/controller/internal/registry/signing"
)

// ErrModuleNotFound reports that no installed module has the requested name
// and version.
var ErrModuleNotFound = errors.New("module not found")

// ErrModuleExists reports an install of a name and version that the registry
// already holds. A module version is immutable, so a new build needs a new
// version.
var ErrModuleExists = errors.New("module version already installed")

// IndexFile is the name of the index file at the registry root.
const IndexFile = "index.json"

// indexVersion is the format version of the index file.
const indexVersion = 1

// Entry is one installed module as the index records it.
type Entry struct {
	Name    string `json:"name"`
	Version string `json:"version"`
	Type    string `json:"type"`
	License string `json:"license"`
	Summary string `json:"summary,omitempty"`
	// Scenarios holds the scenario names, from the scenario field of each
	// scenario file, in manifest order.
	Scenarios []string `json:"scenarios,omitempty"`
	// Digest is the module tree digest that plans carry.
	Digest string `json:"digest"`
	// Signature is the base64 encoding of the 64 byte Ed25519 signature over
	// the UTF-8 bytes of Digest, which install verified against the trust
	// store. It is empty for a module that was installed without a signature.
	// It is not part of the module tree, so it does not change Digest.
	Signature string `json:"signature,omitempty"`
	// SignatureKeyID names the key that made Signature, and is empty when
	// Signature is.
	SignatureKeyID string `json:"signature_key_id,omitempty"`
}

type indexFile struct {
	Version int     `json:"version"`
	Modules []Entry `json:"modules"`
}

// Lookup resolves module references to manifests. The configuration
// validator depends on this interface rather than on *Registry, so its tests
// can pass a map backed fake.
type Lookup interface {
	// Manifest returns the manifest of the installed module with this name
	// and version, or an error that wraps ErrModuleNotFound.
	Manifest(ctx context.Context, name, version string) (Manifest, error)
	// Latest returns the installed version of the module with the highest
	// semver precedence, or an error that wraps ErrModuleNotFound.
	Latest(ctx context.Context, name string) (Manifest, error)
}

var _ Lookup = (*Registry)(nil)

// Registry is a module registry in one local directory. Its methods are safe
// for concurrent use within one process. Two processes must not share a root.
type Registry struct {
	root   string
	limits Limits
	trust  TrustPolicy
	logger *slog.Logger

	mu      sync.RWMutex
	entries []Entry // sorted by name, then version precedence
}

// Open opens the registry at root and creates the directory and an empty
// index when they do not exist. A zero lim selects DefaultLimits. WithTrust
// turns on signature checks at install, and Open then fails when the trust
// store file is unreadable or malformed.
func Open(root string, lim Limits, opts ...Option) (*Registry, error) {
	if lim.MaxBytes <= 0 {
		lim.MaxBytes = DefaultLimits.MaxBytes
	}
	if lim.MaxFiles <= 0 {
		lim.MaxFiles = DefaultLimits.MaxFiles
	}
	r := &Registry{root: filepath.Clean(root), limits: lim, logger: slog.New(slog.DiscardHandler)}
	for _, o := range opts {
		o(r)
	}
	if r.trust.Enforced() {
		if _, err := r.trust.verifier(); err != nil {
			return nil, fmt.Errorf("open registry %s: %w", root, err)
		}
	} else {
		r.logger.Warn("no trust store is configured, so the registry installs modules without checking signatures", "setting", "trust_store")
	}
	for _, d := range []string{r.root, r.modulesDir(), r.stagingDir()} {
		if err := os.MkdirAll(d, 0o750); err != nil {
			return nil, fmt.Errorf("open registry %s: %w", root, err)
		}
	}
	data, err := os.ReadFile(r.indexPath())
	switch {
	case errors.Is(err, os.ErrNotExist):
		if err := r.writeIndex(nil); err != nil {
			return nil, err
		}
		return r, nil
	case err != nil:
		return nil, fmt.Errorf("open registry %s: %w", root, err)
	}
	var idx indexFile
	if err := json.Unmarshal(data, &idx); err != nil {
		return nil, fmt.Errorf("open registry %s: read %s: %w", root, IndexFile, err)
	}
	if idx.Version != indexVersion {
		return nil, fmt.Errorf("open registry %s: %s has version %d, want %d", root, IndexFile, idx.Version, indexVersion)
	}
	r.entries = idx.Modules
	sortEntries(r.entries)
	return r, nil
}

// Root returns the registry directory.
func (r *Registry) Root() string { return r.root }

func (r *Registry) indexPath() string  { return filepath.Join(r.root, IndexFile) }
func (r *Registry) modulesDir() string { return filepath.Join(r.root, "modules") }
func (r *Registry) stagingDir() string { return filepath.Join(r.root, "staging") }

// Dir returns the directory of an installed module tree.
func (r *Registry) Dir(name, version string) string {
	return filepath.Join(r.modulesDir(), name, version)
}

// List returns the installed modules sorted by name and version precedence.
// A non empty typ keeps only modules of that type.
func (r *Registry) List(_ context.Context, typ string) []Entry {
	r.mu.RLock()
	defer r.mu.RUnlock()
	out := make([]Entry, 0, len(r.entries))
	for _, e := range r.entries {
		if typ == "" || e.Type == typ {
			out = append(out, e)
		}
	}
	return out
}

// Entry returns the index entry of one installed module.
func (r *Registry) Entry(_ context.Context, name, version string) (Entry, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	if i := r.find(name, version); i >= 0 {
		return r.entries[i], nil
	}
	return Entry{}, fmt.Errorf("module %s %s: %w", name, version, ErrModuleNotFound)
}

// Manifest returns the validated manifest of one installed module.
func (r *Registry) Manifest(ctx context.Context, name, version string) (Manifest, error) {
	if _, err := r.Entry(ctx, name, version); err != nil {
		return Manifest{}, err
	}
	data, err := os.ReadFile(filepath.Join(r.Dir(name, version), ManifestFile))
	if err != nil {
		return Manifest{}, fmt.Errorf("module %s %s: read manifest: %w", name, version, err)
	}
	m, err := ParseManifest(data)
	if err != nil {
		return Manifest{}, fmt.Errorf("module %s %s: %w", name, version, err)
	}
	return m, nil
}

// Latest returns the manifest of the installed version of name with the
// highest precedence.
func (r *Registry) Latest(ctx context.Context, name string) (Manifest, error) {
	r.mu.RLock()
	version := ""
	for _, e := range r.entries {
		if e.Name == name {
			version = e.Version // entries sort ascending, so the last match wins
		}
	}
	r.mu.RUnlock()
	if version == "" {
		return Manifest{}, fmt.Errorf("module %s: %w", name, ErrModuleNotFound)
	}
	return r.Manifest(ctx, name, version)
}

// Verify recomputes the tree digest of an installed module and compares it
// with the index.
func (r *Registry) Verify(ctx context.Context, name, version string) error {
	e, err := r.Entry(ctx, name, version)
	if err != nil {
		return err
	}
	got, err := TreeDigest(r.Dir(name, version))
	if err != nil {
		return err
	}
	if got != e.Digest {
		return fmt.Errorf("module %s %s: tree digest is %s, index says %s", name, version, got, e.Digest)
	}
	return nil
}

// Install unpacks the module archive in archive, validates it, and adds it to
// the registry. The archive is a tar file, optionally gzip compressed, with
// manifest.yaml at its root. A manifest or tree problem is a *ValidationError,
// and an unreadable archive wraps ErrArchive or ErrArchiveTooLarge.
//
// With a trust store (see TrustPolicy), Install verifies the signature that
// WithSignature supplies against the digest of the archive before it unpacks
// anything. It refuses a failure with an error that wraps ErrUntrusted, and
// stores the signature in the index entry of the module.
func (r *Registry) Install(ctx context.Context, archive io.Reader, opts ...InstallOption) (Entry, error) {
	sg, err := r.gate(opts)
	if err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	if sg == nil {
		return r.install(ctx, func(dst string) error { return unpack(archive, dst, r.limits) }, nil)
	}
	spool, err := r.spool(archive)
	if err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	defer func() {
		_ = spool.Close()
		_ = os.Remove(spool.Name())
	}()
	sg.digest, err = sg.verifier.VerifyArchive(spool, sg.sig, signing.Limits{MaxBytes: r.limits.MaxBytes, MaxFiles: r.limits.MaxFiles})
	if err != nil {
		return Entry{}, fmt.Errorf("install module: %w", refuse(err))
	}
	if _, err := spool.Seek(0, io.SeekStart); err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	return r.install(ctx, func(dst string) error { return unpack(spool, dst, r.limits) }, sg)
}

// InstallDir installs the module tree in the directory src, for example a
// module under modules/ of the repository. The trust policy applies as it does
// to Install, and the signature is verified against the digest of the staged
// tree before the module enters the registry.
func (r *Registry) InstallDir(ctx context.Context, src string, opts ...InstallOption) (Entry, error) {
	sg, err := r.gate(opts)
	if err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	return r.install(ctx, func(dst string) error { return copyTree(src, dst, r.limits) }, sg)
}

// install fills a staging directory, validates it, and moves it into place. A
// non nil sg holds a signature that must verify, and install stores it.
func (r *Registry) install(ctx context.Context, fill func(dst string) error, sg *signed) (Entry, error) {
	stage, err := os.MkdirTemp(r.stagingDir(), "install-")
	if err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	defer func() { _ = os.RemoveAll(stage) }()
	if err := fill(stage); err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	if err := ctx.Err(); err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	e, err := checkTree(stage)
	if err != nil {
		return Entry{}, fmt.Errorf("install module: %w", err)
	}
	if sg != nil {
		switch {
		case sg.digest == "":
			if err := sg.verifier.VerifyDigest(e.Digest, sg.sig); err != nil {
				return Entry{}, fmt.Errorf("install module %s %s: %w", e.Name, e.Version, refuse(err))
			}
		case sg.digest != e.Digest:
			// The unpacked tree must be the tree that the signature covers.
			return Entry{}, fmt.Errorf("install module %s %s: %w: the unpacked tree has digest %s and the signed archive %s",
				e.Name, e.Version, ErrUntrusted, e.Digest, sg.digest)
		}
		sg.encode(&e)
	} else if r.trust.Enforced() {
		r.logger.Warn("installed a module without a signature", "module", e.Name, "version", e.Version, "setting", "allow_unsigned_modules")
	} else {
		r.logger.Warn("installed a module without checking its signature", "module", e.Name, "version", e.Version)
	}

	r.mu.Lock()
	defer r.mu.Unlock()
	if r.find(e.Name, e.Version) >= 0 {
		return Entry{}, fmt.Errorf("install module %s %s: %w", e.Name, e.Version, ErrModuleExists)
	}
	dst := r.Dir(e.Name, e.Version)
	// A tree without an index entry is left over from an install that
	// stopped before it wrote the index, so it is safe to replace.
	if err := os.RemoveAll(dst); err != nil {
		return Entry{}, fmt.Errorf("install module %s %s: %w", e.Name, e.Version, err)
	}
	if err := os.MkdirAll(filepath.Dir(dst), 0o750); err != nil {
		return Entry{}, fmt.Errorf("install module %s %s: %w", e.Name, e.Version, err)
	}
	if err := os.Rename(stage, dst); err != nil {
		return Entry{}, fmt.Errorf("install module %s %s: %w", e.Name, e.Version, err)
	}
	next := append(append([]Entry(nil), r.entries...), e)
	sortEntries(next)
	if err := r.writeIndex(next); err != nil {
		_ = os.RemoveAll(dst)
		return Entry{}, fmt.Errorf("install module %s %s: %w", e.Name, e.Version, err)
	}
	r.entries = next
	return e, nil
}

// Delete removes an installed module from the index and then its tree.
func (r *Registry) Delete(_ context.Context, name, version string) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	i := r.find(name, version)
	if i < 0 {
		return fmt.Errorf("delete module %s %s: %w", name, version, ErrModuleNotFound)
	}
	next := append(append([]Entry(nil), r.entries[:i]...), r.entries[i+1:]...)
	if err := r.writeIndex(next); err != nil {
		return fmt.Errorf("delete module %s %s: %w", name, version, err)
	}
	r.entries = next
	if err := os.RemoveAll(r.Dir(name, version)); err != nil {
		return fmt.Errorf("delete module %s %s: remove tree: %w", name, version, err)
	}
	// Remove the name directory when its last version is gone.
	_ = os.Remove(filepath.Join(r.modulesDir(), name))
	return nil
}

func (r *Registry) find(name, version string) int {
	for i, e := range r.entries {
		if e.Name == name && e.Version == version {
			return i
		}
	}
	return -1
}

// writeIndex replaces the index file through a temporary file and a rename.
func (r *Registry) writeIndex(entries []Entry) error {
	if entries == nil {
		entries = []Entry{}
	}
	data, err := json.MarshalIndent(indexFile{Version: indexVersion, Modules: entries}, "", "  ")
	if err != nil {
		return fmt.Errorf("encode %s: %w", IndexFile, err)
	}
	f, err := os.CreateTemp(r.root, IndexFile+".tmp-")
	if err != nil {
		return fmt.Errorf("write %s: %w", IndexFile, err)
	}
	tmp := f.Name()
	_, err = f.Write(append(data, '\n'))
	if err == nil {
		err = f.Sync()
	}
	if cerr := f.Close(); err == nil {
		err = cerr
	}
	if err == nil {
		err = os.Rename(tmp, r.indexPath())
	}
	if err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("write %s: %w", IndexFile, err)
	}
	return nil
}

func sortEntries(es []Entry) {
	sort.SliceStable(es, func(i, j int) bool {
		if es[i].Name != es[j].Name {
			return es[i].Name < es[j].Name
		}
		return compareVersions(es[i].Version, es[j].Version) < 0
	})
}

// checkTree validates the module tree in dir and returns its index entry.
// Every read goes through an os.Root, so no path in the manifest can reach
// outside the tree.
func checkTree(dir string) (Entry, error) {
	root, err := os.OpenRoot(dir)
	if err != nil {
		return Entry{}, fmt.Errorf("open module tree: %w", err)
	}
	defer func() { _ = root.Close() }()
	data, err := fs.ReadFile(root.FS(), ManifestFile)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return Entry{}, invalid("", "file:"+ManifestFile, "the module has no manifest at its root")
		}
		return Entry{}, fmt.Errorf("read manifest: %w", err)
	}
	m, err := ParseManifest(data)
	if err != nil {
		return Entry{}, err
	}
	verr := &ValidationError{Module: moduleLabel(m)}
	for _, p := range m.Files() {
		info, err := root.Lstat(filepath.FromSlash(p))
		if err != nil || !info.Mode().IsRegular() {
			verr.Findings = append(verr.Findings, Finding{Path: "file:" + p, Message: "the manifest names a file that the module does not contain"})
		}
	}
	names := make([]string, 0, len(m.Scenarios))
	seen := map[string]string{}
	for i, p := range m.Scenarios {
		name, err := scenarioName(root.FS(), p)
		if err != nil {
			if !errors.Is(err, os.ErrNotExist) {
				verr.Findings = append(verr.Findings, Finding{Path: fmt.Sprintf("/scenarios/%d", i), Message: err.Error()})
			}
			continue
		}
		if prev, dup := seen[name]; dup {
			verr.Findings = append(verr.Findings, Finding{
				Path:    fmt.Sprintf("/scenarios/%d", i),
				Message: fmt.Sprintf("scenario name %s is also used by %s", name, prev),
			})
			continue
		}
		seen[name] = p
		names = append(names, name)
		found, err := checkScenarioFile(root.FS(), p)
		if err != nil {
			verr.Findings = append(verr.Findings, Finding{Path: "file:" + p, Message: "read scenario: " + err.Error()})
		}
		verr.Findings = append(verr.Findings, found...)
	}
	// Scenarios that use syntax the engine does not compile yet sit in
	// scenarios-pending. The manifest does not list them and the index does
	// not carry them, but they follow the same contract.
	pending, err := fs.Glob(root.FS(), pendingDir+"/*.yaml")
	if err != nil {
		return Entry{}, fmt.Errorf("list %s: %w", pendingDir, err)
	}
	for _, p := range pending {
		found, err := checkScenarioFile(root.FS(), p)
		if err != nil {
			verr.Findings = append(verr.Findings, Finding{Path: "file:" + p, Message: "read scenario: " + err.Error()})
		}
		verr.Findings = append(verr.Findings, found...)
	}
	if len(verr.Findings) > 0 {
		return Entry{}, verr
	}
	digest, err := TreeDigest(dir)
	if err != nil {
		return Entry{}, err
	}
	return Entry{
		Name: m.Name, Version: m.Version, Type: m.Type, License: m.License,
		Summary: m.Summary, Scenarios: names, Digest: digest,
	}, nil
}

// identifier is the identifier pattern of the scenario schema.
var identifier = regexp.MustCompile(`^[a-z][a-z0-9_]{0,62}$`)

// scenarioName reads the scenario field of a scenario file. Full scenario
// validation belongs to the configuration validator.
func scenarioName(fsys fs.FS, p string) (string, error) {
	data, err := fs.ReadFile(fsys, p)
	if err != nil {
		return "", err
	}
	var doc struct {
		Scenario string `yaml:"scenario"`
	}
	if err := yaml.Unmarshal(data, &doc); err != nil {
		return "", fmt.Errorf("scenario file is not valid YAML: %s", err.Error())
	}
	if !identifier.MatchString(doc.Scenario) {
		return "", fmt.Errorf("scenario file has no scenario name that is an identifier")
	}
	return doc.Scenario, nil
}
