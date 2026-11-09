// SPDX-License-Identifier: GPL-3.0-only

package signing

import (
	"bytes"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sort"
	"sync"
	"time"
)

// Status is the standing of a key in a trust store.
type Status string

// The three standings of a key.
const (
	// StatusActive keys verify at any time. A store may hold several, so a
	// rotation can overlap the old and the new signer.
	StatusActive Status = "active"
	// StatusRetired keys verify until their cutoff, strictly before it, and
	// never after. Retiring a key lets modules that it signed stay
	// installable while they are re-signed.
	StatusRetired Status = "retired"
	// StatusRevoked keys never verify, whatever the time.
	StatusRevoked Status = "revoked"
)

// trustStoreVersion is the format version of the trust store file.
const trustStoreVersion = 1

// Key is one public key in a trust store.
type Key struct {
	// ID is KeyID(Public).
	ID string
	// Public is the verification key.
	Public ed25519.PublicKey
	// Status is the standing of the key.
	Status Status
	// RetiredAfter is the cutoff of a retired key. It is zero for the other
	// standings.
	RetiredAfter time.Time
}

// TrustStore holds the public keys that a verifier accepts. It is safe for
// concurrent use. The zero value is not usable; call NewTrustStore.
type TrustStore struct {
	mu   sync.RWMutex
	keys map[string]Key
}

// NewTrustStore returns an empty trust store.
func NewTrustStore() *TrustStore {
	return &TrustStore{keys: map[string]Key{}}
}

// Add trusts a public key as active and returns its key id. Adding a key that
// the store already holds, in any standing, is an error, so a revoked key
// cannot come back.
func (t *TrustStore) Add(pub ed25519.PublicKey) (string, error) {
	if len(pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("%w: public key has %d bytes, want %d", ErrKey, len(pub), ed25519.PublicKeySize)
	}
	id := KeyID(pub)
	t.mu.Lock()
	defer t.mu.Unlock()
	if old, ok := t.keys[id]; ok {
		return "", fmt.Errorf("%w: key %s is already %s", ErrTrustStore, id, old.Status)
	}
	t.keys[id] = Key{ID: id, Public: append(ed25519.PublicKey(nil), pub...), Status: StatusActive}
	return id, nil
}

// Retire marks an active or retired key as retired with a cutoff. The key
// verifies strictly before the cutoff. A revoked key stays revoked.
func (t *TrustStore) Retire(id string, cutoff time.Time) error {
	if cutoff.IsZero() {
		return fmt.Errorf("%w: retire %s without a cutoff", ErrTrustStore, id)
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	k, ok := t.keys[id]
	switch {
	case !ok:
		return fmt.Errorf("retire %s: %w", id, ErrUnknownKey)
	case k.Status == StatusRevoked:
		return fmt.Errorf("retire %s: %w", id, ErrKeyRevoked)
	}
	k.Status, k.RetiredAfter = StatusRetired, cutoff.UTC().Truncate(time.Second)
	t.keys[id] = k
	return nil
}

// Revoke marks a key as revoked. The standing is final.
func (t *TrustStore) Revoke(id string) error {
	t.mu.Lock()
	defer t.mu.Unlock()
	k, ok := t.keys[id]
	if !ok {
		return fmt.Errorf("revoke %s: %w", id, ErrUnknownKey)
	}
	k.Status, k.RetiredAfter = StatusRevoked, time.Time{}
	t.keys[id] = k
	return nil
}

// Key returns the entry for an id without judging its standing.
func (t *TrustStore) Key(id string) (Key, bool) {
	t.mu.RLock()
	defer t.mu.RUnlock()
	k, ok := t.keys[id]
	return k, ok
}

// Keys returns every entry sorted by id.
func (t *TrustStore) Keys() []Key {
	t.mu.RLock()
	defer t.mu.RUnlock()
	out := make([]Key, 0, len(t.keys))
	for _, k := range t.keys {
		out = append(out, k)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// Lookup returns the public key for id if it may verify at time now. It
// returns an error that wraps ErrUnknownKey, ErrKeyRevoked, or ErrKeyExpired
// otherwise.
func (t *TrustStore) Lookup(id string, now time.Time) (ed25519.PublicKey, error) {
	k, ok := t.Key(id)
	switch {
	case !ok:
		return nil, fmt.Errorf("key %q: %w", id, ErrUnknownKey)
	case k.Status == StatusRevoked:
		return nil, fmt.Errorf("key %s: %w", id, ErrKeyRevoked)
	case k.Status == StatusRetired && !now.Before(k.RetiredAfter):
		return nil, fmt.Errorf("key %s retired after %s: %w", id, k.RetiredAfter.Format(time.RFC3339), ErrKeyExpired)
	}
	return k.Public, nil
}

type trustFile struct {
	Version int            `json:"version"`
	Keys    []trustFileKey `json:"keys"`
}

type trustFileKey struct {
	KeyID        string `json:"key_id"`
	PublicKey    []byte `json:"public_key"`
	Status       Status `json:"status"`
	RetiredAfter string `json:"retired_after,omitempty"`
}

// Encode writes the store as JSON with the keys sorted by id. The file holds
// public keys only, and it is meant to be reviewed and kept in version
// control.
func (t *TrustStore) Encode() ([]byte, error) {
	f := trustFile{Version: trustStoreVersion, Keys: []trustFileKey{}}
	for _, k := range t.Keys() {
		e := trustFileKey{KeyID: k.ID, PublicKey: k.Public, Status: k.Status}
		if k.Status == StatusRetired {
			e.RetiredAfter = k.RetiredAfter.UTC().Format(time.RFC3339)
		}
		f.Keys = append(f.Keys, e)
	}
	data, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return nil, fmt.Errorf("encode trust store: %w", err)
	}
	return append(data, '\n'), nil
}

// ParseTrustStore reads the JSON that Encode writes. It rejects unknown
// fields, a key id that is not derived from its public key, a retired key
// without a cutoff, a cutoff on another standing, and duplicate keys.
func ParseTrustStore(data []byte) (*TrustStore, error) {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	var f trustFile
	if err := dec.Decode(&f); err != nil {
		return nil, fmt.Errorf("%w: %s", ErrTrustStore, err.Error())
	}
	if err := dec.Decode(new(json.RawMessage)); !errors.Is(err, io.EOF) {
		return nil, fmt.Errorf("%w: data after the trust store", ErrTrustStore)
	}
	if f.Version != trustStoreVersion {
		return nil, fmt.Errorf("%w: version %d, want %d", ErrTrustStore, f.Version, trustStoreVersion)
	}
	ts := NewTrustStore()
	for i, e := range f.Keys {
		if len(e.PublicKey) != ed25519.PublicKeySize {
			return nil, fmt.Errorf("%w: key %d has a public key of %d bytes", ErrTrustStore, i, len(e.PublicKey))
		}
		pub := ed25519.PublicKey(e.PublicKey)
		if got := KeyID(pub); got != e.KeyID {
			return nil, fmt.Errorf("%w: key %d has id %q but its public key has id %s", ErrTrustStore, i, e.KeyID, got)
		}
		if _, dup := ts.keys[e.KeyID]; dup {
			return nil, fmt.Errorf("%w: key %s appears twice", ErrTrustStore, e.KeyID)
		}
		k := Key{ID: e.KeyID, Public: pub, Status: e.Status}
		switch e.Status {
		case StatusActive, StatusRevoked:
			if e.RetiredAfter != "" {
				return nil, fmt.Errorf("%w: key %s is %s and has a cutoff", ErrTrustStore, e.KeyID, e.Status)
			}
		case StatusRetired:
			cutoff, err := time.Parse(time.RFC3339, e.RetiredAfter)
			if err != nil {
				return nil, fmt.Errorf("%w: key %s is retired and needs an RFC 3339 retired_after", ErrTrustStore, e.KeyID)
			}
			k.RetiredAfter = cutoff.UTC()
		default:
			return nil, fmt.Errorf("%w: key %s has status %q", ErrTrustStore, e.KeyID, e.Status)
		}
		ts.keys[e.KeyID] = k
	}
	return ts, nil
}
