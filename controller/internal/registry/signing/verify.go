// SPDX-License-Identifier: GPL-3.0-only

package signing

import (
	"crypto/ed25519"
	"fmt"
	"io"
	"time"
)

// Verifier checks signatures against a trust store at the time that its clock
// reports. The clock matters only for retired keys.
type Verifier struct {
	store *TrustStore
	now   func() time.Time
}

// NewVerifier returns a verifier. A nil now selects time.Now, which is the
// process edge; tests and the orchestrator inject their own clock.
func NewVerifier(store *TrustStore, now func() time.Time) *Verifier {
	if now == nil {
		now = time.Now
	}
	return &Verifier{store: store, now: now}
}

// verify checks one signature over msg. The checks run in this order, so the
// error names the first thing that is wrong: no signature, a signature of the
// wrong length, a key id the store does not know, a revoked key, a retired key
// past its cutoff, and last the signature itself.
func (v *Verifier) verify(msg []byte, keyID string, sig []byte) error {
	if len(sig) == 0 && keyID == "" {
		return ErrNoSignature
	}
	if len(sig) != SignatureSize {
		return fmt.Errorf("%w: %d bytes, want %d", ErrSignatureLength, len(sig), SignatureSize)
	}
	pub, err := v.store.Lookup(keyID, v.now())
	if err != nil {
		return err
	}
	if !ed25519.Verify(pub, msg, sig) {
		return fmt.Errorf("%w under key %s", ErrBadSignature, keyID)
	}
	return nil
}

// VerifyDigest checks the signature of a module tree digest. It is what the
// agent calls with ModuleRef.digest, signature, and signature_key_id.
func (v *Verifier) VerifyDigest(digest string, sig Signature) error {
	if !digestPattern.MatchString(digest) {
		return fmt.Errorf("verify %q: %w", digest, ErrDigest)
	}
	if err := v.verify([]byte(digest), sig.KeyID, sig.Bytes); err != nil {
		return fmt.Errorf("verify module %s: %w", digest, err)
	}
	return nil
}

// VerifyArchive digests the archive in r and checks the signature against the
// digest. It returns the digest on success, which the caller compares with the
// digest of the index or of the plan. A modified archive has another digest and
// fails with ErrBadSignature.
func (v *Verifier) VerifyArchive(r io.Reader, sig Signature, lim Limits) (string, error) {
	digest, err := ArchiveDigest(r, lim)
	if err != nil {
		return "", fmt.Errorf("verify archive: %w", err)
	}
	if err := v.VerifyDigest(digest, sig); err != nil {
		return "", err
	}
	return digest, nil
}

// VerifyIndex checks a signed index and returns the index it covers. The
// signature is checked over the canonical bytes of the parsed index, so the
// layout of the file does not matter and every value does.
func (v *Verifier) VerifyIndex(si SignedIndex) (Index, error) {
	data, err := si.Index.Canonical()
	if err != nil {
		return Index{}, fmt.Errorf("verify index: %w", err)
	}
	if err := v.verify(data, si.KeyID, si.Signature); err != nil {
		return Index{}, fmt.Errorf("verify index serial %d: %w", si.Index.Serial, err)
	}
	return si.Index, nil
}
