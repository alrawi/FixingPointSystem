// SPDX-License-Identifier: GPL-3.0-only

// Package signing signs and verifies module archives and the registry index
// with Ed25519, using only the standard library.
//
// # Archives
//
// A module is identified by its tree digest, the "sha256:<hex>" string that
// contracts/README.md defines for plans and that registry.TreeDigest computes.
// The signature of a module is an Ed25519 signature over the UTF-8 bytes of
// that digest string, which is what the ModuleRef.signature field of the agent
// protocol carries (CON-05), next to signature_key_id. Signer.SignArchive and
// Verifier.VerifyArchive compute the digest from a tar archive, optionally gzip
// compressed, without unpacking it to disk, so a modified archive gets another
// digest and fails verification. The archive rules match registry install:
// only files and directories, relpath characters, no "." or ".." segment.
//
// # Index
//
// The registry index lists every module with its version and digest. The
// signed form canonicalizes the list (see Index.Canonical) and signs those
// bytes, so a reader that parses the file and a reader that checks the bytes
// agree on what was signed. An archive signature covers "sha256:..." text and
// an index signature covers a JSON object that starts with "{", so a signature
// of one kind never verifies as the other.
//
// # Keys
//
// A key id is derived from the public key (see KeyID), so an id cannot name a
// key other than the one it was derived from. A TrustStore holds the public
// keys that a verifier accepts. A key is active, retired until a cutoff, or
// revoked. Several keys may be active at once, which is how a rotation
// overlaps old and new signers. See docs/wiki/Controller.md for the rotation
// procedure.
package signing
