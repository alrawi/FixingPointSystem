// SPDX-License-Identifier: GPL-3.0-only

package signing

import (
	"bytes"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"regexp"
	"sort"
	"strconv"
)

// IndexFormat names the canonical form below. A change of the form needs a new
// name, and a verifier refuses a name it does not know.
const IndexFormat = "fps-registry-index/1"

var (
	// namePattern and versionPattern are the identifier and semver patterns of
	// contracts/schemas/module-manifest.schema.json. They allow only ASCII
	// letters, digits, and "_", "-", ".", "+", so the canonical form never
	// needs a JSON escape and any language can produce the same bytes.
	namePattern    = regexp.MustCompile(`^[a-z][a-z0-9_]{0,62}$`)
	versionPattern = regexp.MustCompile(`^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$`)
)

// IndexEntry is one module version in the signed index.
type IndexEntry struct {
	// Name is the module name.
	Name string `json:"name"`
	// Version is the module version.
	Version string `json:"version"`
	// Digest is the module tree digest.
	Digest string `json:"digest"`
}

// Index is the content that an index signature covers: the modules of a
// registry with their versions and digests. It carries no summaries or
// scenario names, which a client reads from the manifest after it verified
// the digest.
type Index struct {
	// Format is IndexFormat.
	Format string `json:"format"`
	// Serial grows with every published index, so a client that remembers the
	// highest serial it saw can refuse an older index that an attacker replays.
	Serial uint64 `json:"serial"`
	// Modules lists the modules in any order. Canonical sorts them.
	Modules []IndexEntry `json:"modules"`
}

// NewIndex builds an index in the current format.
func NewIndex(serial uint64, modules []IndexEntry) Index {
	return Index{Format: IndexFormat, Serial: serial, Modules: append([]IndexEntry(nil), modules...)}
}

// Find returns the entry for a module version.
func (x Index) Find(name, version string) (IndexEntry, bool) {
	for _, m := range x.Modules {
		if m.Name == name && m.Version == version {
			return m, true
		}
	}
	return IndexEntry{}, false
}

// Canonical returns the bytes that an index signature covers. The form is
// JSON with no whitespace and these rules:
//
//   - The object has the keys "format", "modules", and "serial", in that order.
//   - "format" is IndexFormat, and "serial" is a decimal integer with no sign
//     or leading zero.
//   - "modules" is an array of objects with the keys "digest", "name", and
//     "version", in that order, sorted by name and then by version, comparing
//     bytes. A name and version pair appears once.
//   - Every value is a string without escapes, because the patterns that
//     Canonical enforces allow no character that JSON escapes. A digest is
//     "sha256:" and 64 lowercase hex digits.
//
// Example: {"format":"fps-registry-index/1","modules":[{"digest":"sha256:...","name":"dns","version":"1.0.0"}],"serial":3}
func (x Index) Canonical() ([]byte, error) {
	if x.Format != IndexFormat {
		return nil, fmt.Errorf("%w: format %q, want %q", ErrIndex, x.Format, IndexFormat)
	}
	mods := append([]IndexEntry(nil), x.Modules...)
	sort.Slice(mods, func(i, j int) bool {
		if mods[i].Name != mods[j].Name {
			return mods[i].Name < mods[j].Name
		}
		return mods[i].Version < mods[j].Version
	})
	var b bytes.Buffer
	b.WriteString(`{"format":"` + IndexFormat + `","modules":[`)
	for i, m := range mods {
		switch {
		case !namePattern.MatchString(m.Name):
			return nil, fmt.Errorf("%w: module name %q is not an identifier", ErrIndex, m.Name)
		case !versionPattern.MatchString(m.Version):
			return nil, fmt.Errorf("%w: module %s has version %q, which is not a semantic version", ErrIndex, m.Name, m.Version)
		case !digestPattern.MatchString(m.Digest):
			return nil, fmt.Errorf("%w: module %s %s has digest %q, want sha256: and 64 lowercase hex digits", ErrIndex, m.Name, m.Version, m.Digest)
		case i > 0 && mods[i-1].Name == m.Name && mods[i-1].Version == m.Version:
			return nil, fmt.Errorf("%w: module %s %s appears twice", ErrIndex, m.Name, m.Version)
		}
		if i > 0 {
			b.WriteByte(',')
		}
		b.WriteString(`{"digest":"` + m.Digest + `","name":"` + m.Name + `","version":"` + m.Version + `"}`)
	}
	b.WriteString(`],"serial":` + strconv.FormatUint(x.Serial, 10) + `}`)
	return b.Bytes(), nil
}

// SignedIndex is the published form of the index: the index, the id of the
// signing key, and the signature over Index.Canonical. It encodes as JSON
// with the signature in base64.
type SignedIndex struct {
	// Index is the signed content.
	Index Index `json:"index"`
	// KeyID is the id of the signing key.
	KeyID string `json:"key_id"`
	// Signature is the Ed25519 signature of the canonical bytes of Index.
	Signature []byte `json:"signature"`
}

// SignIndex canonicalizes the index and signs it.
func (s *Signer) SignIndex(idx Index) (SignedIndex, error) {
	data, err := idx.Canonical()
	if err != nil {
		return SignedIndex{}, fmt.Errorf("sign index: %w", err)
	}
	return SignedIndex{Index: idx, KeyID: s.id, Signature: ed25519.Sign(s.priv, data)}, nil
}

// Encode writes the signed index as indented JSON.
func (si SignedIndex) Encode() ([]byte, error) {
	data, err := json.MarshalIndent(si, "", "  ")
	if err != nil {
		return nil, fmt.Errorf("encode signed index: %w", err)
	}
	return append(data, '\n'), nil
}

// ParseSignedIndex reads the JSON that Encode writes. It rejects unknown
// fields and trailing data, so nothing unsigned rides along in the file. It
// does not verify the signature; Verifier.VerifyIndex does.
func ParseSignedIndex(data []byte) (SignedIndex, error) {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	var si SignedIndex
	if err := dec.Decode(&si); err != nil {
		return SignedIndex{}, fmt.Errorf("%w: %s", ErrIndex, err.Error())
	}
	if err := dec.Decode(new(json.RawMessage)); !errors.Is(err, io.EOF) {
		return SignedIndex{}, fmt.Errorf("%w: data after the signed index", ErrIndex)
	}
	return si, nil
}
