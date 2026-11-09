// SPDX-License-Identifier: GPL-3.0-only

package signing

import "errors"

// Verification failures. Callers test them with errors.Is.
var (
	// ErrNoSignature reports a module or index without a signature. Whether
	// that is acceptable is the caller's policy, for example developer mode.
	ErrNoSignature = errors.New("no signature")
	// ErrSignatureLength reports a signature that is not 64 bytes long, such
	// as a truncated one.
	ErrSignatureLength = errors.New("signature has the wrong length")
	// ErrUnknownKey reports a key id that the trust store does not hold.
	ErrUnknownKey = errors.New("unknown signing key")
	// ErrKeyRevoked reports a key that the trust store revoked. A revoked key
	// never verifies.
	ErrKeyRevoked = errors.New("signing key is revoked")
	// ErrKeyExpired reports a retired key whose cutoff has passed.
	ErrKeyExpired = errors.New("signing key is retired and its cutoff has passed")
	// ErrBadSignature reports a signature that does not match the signed
	// bytes under the key that the key id names.
	ErrBadSignature = errors.New("signature does not verify")
	// ErrDigest reports a string that is not a "sha256:" digest.
	ErrDigest = errors.New("invalid digest")
	// ErrArchive reports an archive that cannot be digested, such as a link,
	// an entry with an unusable path, or a broken tar stream.
	ErrArchive = errors.New("invalid module archive")
	// ErrArchiveTooLarge reports an archive over the limits.
	ErrArchiveTooLarge = errors.New("module archive is too large")
	// ErrIndex reports an index that is malformed, for example with a
	// duplicate module, an unknown format, or a name that is not an
	// identifier.
	ErrIndex = errors.New("invalid registry index")
	// ErrTrustStore reports a trust store that is malformed or a change that
	// it refuses.
	ErrTrustStore = errors.New("invalid trust store")
	// ErrKey reports a malformed private or public key.
	ErrKey = errors.New("invalid key")
)
