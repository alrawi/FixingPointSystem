// SPDX-License-Identifier: GPL-3.0-only
//! The trust store and the signature check of module tree digests (AGT-04).
//!
//! This is the Rust side of `controller/internal/registry/signing` (CTL-11).
//! The formats are the same, byte for byte.
//!
//! - A module is identified by its tree digest, the `sha256:<64 hex>` string
//!   that [`crate::modules::tree_digest`] computes by rule 1 of
//!   `contracts/README.md`. Its signature is an Ed25519 signature over the
//!   UTF-8 bytes of that string, which `ModuleRef.signature` carries next to
//!   `ModuleRef.signature_key_id` (CON-05).
//! - A key id is `ed25519:` and the hex of the first 16 bytes of the SHA-256
//!   of the 32 byte public key.
//! - The trust store file is the JSON of `TrustStore.Encode`: `version` 1 and
//!   `keys`, each with `key_id`, `public_key` (standard base64), `status`
//!   (`active`, `retired`, or `revoked`), and for a retired key `retired_after`
//!   in RFC 3339. An active key verifies at any time, a retired key strictly
//!   before its cutoff, and a revoked key never.
//! - [`TrustStore::verify`] checks in the order of the Go verifier, so the
//!   error names the first thing that is wrong: no signature, a signature of
//!   the wrong length, an unknown key, a revoked key, a retired key past its
//!   cutoff, and last the signature itself.
//!
//! The library reads no clock. [`ModulePolicy`] holds the clock function that
//! the binary sets to the system time.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use base64::Engine as _;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The length of an Ed25519 signature in bytes.
pub const SIGNATURE_SIZE: usize = 64;
/// The length of an Ed25519 public key in bytes.
pub const PUBLIC_KEY_SIZE: usize = 32;
/// The format version of the trust store file.
const TRUST_STORE_VERSION: u32 = 1;
/// Starts every key id.
pub const KEY_ID_PREFIX: &str = "ed25519:";

/// Why a trust store or a signature is refused. The variants of the checks
/// follow `errors.go` of the signing package.
#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    /// The module has no signature. Developer mode accepts this.
    #[error("no signature")]
    NoSignature,
    /// The signature is not 64 bytes long.
    #[error("signature has {got} bytes, want {SIGNATURE_SIZE}")]
    SignatureLength {
        /// The length that the plan carried.
        got: usize,
    },
    /// The trust store does not hold the key id.
    #[error("unknown signing key {0:?}")]
    UnknownKey(String),
    /// The trust store revoked the key.
    #[error("signing key {0} is revoked")]
    KeyRevoked(String),
    /// The key is retired and its cutoff has passed.
    #[error("signing key {0} is retired and its cutoff has passed")]
    KeyExpired(String),
    /// The signature does not match the digest under the named key.
    #[error("signature does not verify under key {0}")]
    BadSignature(String),
    /// The digest is not `sha256:` and 64 lowercase hex digits.
    #[error("invalid digest {0:?}")]
    Digest(String),
    /// The trust store file is malformed.
    #[error("invalid trust store: {0}")]
    Store(String),
    /// The trust store file could not be read.
    #[error("read trust store {path}")]
    Read {
        /// The file.
        path: String,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
}

/// The standing of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Verifies at any time.
    Active,
    /// Verifies strictly before the cutoff.
    Retired(SystemTime),
    /// Never verifies.
    Revoked,
}

#[derive(Debug, Clone)]
struct Key {
    public: [u8; PUBLIC_KEY_SIZE],
    standing: Standing,
}

/// The public keys that the agent accepts, as the controller's trust store
/// file lists them.
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    keys: HashMap<String, Key>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDoc {
    version: u32,
    keys: Vec<FileKey>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileKey {
    key_id: String,
    public_key: String,
    status: String,
    #[serde(default)]
    retired_after: String,
}

/// Returns the id of a public key: `ed25519:` and the hex of the first 16
/// bytes of its SHA-256.
pub fn key_id(public: &[u8]) -> String {
    let sum = Sha256::digest(public);
    let mut s = String::from(KEY_ID_PREFIX);
    for b in &sum[..16] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Returns whether `d` is `sha256:` and 64 lowercase hex digits.
pub fn is_digest(d: &str) -> bool {
    d.strip_prefix("sha256:").is_some_and(|h| {
        h.len() == 64
            && h.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}

impl TrustStore {
    /// Reads the file of `--module-trust-store`.
    pub fn load(path: &Path) -> Result<Self, TrustError> {
        let data = std::fs::read(path).map_err(|source| TrustError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&data)
    }

    /// Parses the JSON that `TrustStore.Encode` of the controller writes. It
    /// rejects unknown fields, trailing data, a key id that is not derived
    /// from its public key, a retired key without a cutoff, a cutoff on
    /// another standing, and duplicates, as the Go parser does.
    pub fn parse(data: &[u8]) -> Result<Self, TrustError> {
        let doc: FileDoc =
            serde_json::from_slice(data).map_err(|e| TrustError::Store(e.to_string()))?;
        if doc.version != TRUST_STORE_VERSION {
            return Err(TrustError::Store(format!(
                "version {}, want {TRUST_STORE_VERSION}",
                doc.version
            )));
        }
        let mut keys = HashMap::new();
        for (i, e) in doc.keys.iter().enumerate() {
            let public = base64::engine::general_purpose::STANDARD
                .decode(&e.public_key)
                .map_err(|err| TrustError::Store(format!("key {i}: public_key: {err}")))?;
            let public: [u8; PUBLIC_KEY_SIZE] = public.as_slice().try_into().map_err(|_| {
                TrustError::Store(format!(
                    "key {i} has a public key of {} bytes",
                    public.len()
                ))
            })?;
            let got = key_id(&public);
            if got != e.key_id {
                return Err(TrustError::Store(format!(
                    "key {i} has id {:?} but its public key has id {got}",
                    e.key_id
                )));
            }
            let standing = match (e.status.as_str(), e.retired_after.is_empty()) {
                ("active", true) => Standing::Active,
                ("revoked", true) => Standing::Revoked,
                ("active" | "revoked", false) => {
                    return Err(TrustError::Store(format!(
                        "key {} is {} and has a cutoff",
                        e.key_id, e.status
                    )))
                }
                ("retired", _) => {
                    let t = time::OffsetDateTime::parse(
                        &e.retired_after,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .map_err(|_| {
                        TrustError::Store(format!(
                            "key {} is retired and needs an RFC 3339 retired_after",
                            e.key_id
                        ))
                    })?;
                    Standing::Retired(SystemTime::from(t))
                }
                (other, _) => {
                    return Err(TrustError::Store(format!(
                        "key {} has status {other:?}",
                        e.key_id
                    )))
                }
            };
            if keys
                .insert(e.key_id.clone(), Key { public, standing })
                .is_some()
            {
                return Err(TrustError::Store(format!("key {} appears twice", e.key_id)));
            }
        }
        Ok(Self { keys })
    }

    /// Returns the number of keys, in any standing.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns whether the store holds no key.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Checks the signature of a module tree digest at the time `now`. It is
    /// the check that `Verifier.VerifyDigest` of the controller makes.
    pub fn verify(
        &self,
        digest: &str,
        key_id: &str,
        signature: &[u8],
        now: SystemTime,
    ) -> Result<(), TrustError> {
        if !is_digest(digest) {
            return Err(TrustError::Digest(digest.to_owned()));
        }
        if signature.is_empty() && key_id.is_empty() {
            return Err(TrustError::NoSignature);
        }
        if signature.len() != SIGNATURE_SIZE {
            return Err(TrustError::SignatureLength {
                got: signature.len(),
            });
        }
        let key = self
            .keys
            .get(key_id)
            .ok_or_else(|| TrustError::UnknownKey(key_id.to_owned()))?;
        match key.standing {
            Standing::Active => {}
            Standing::Revoked => return Err(TrustError::KeyRevoked(key_id.to_owned())),
            // Strictly before the cutoff, as `!now.Before(cutoff)` fails in Go.
            Standing::Retired(cutoff) if now >= cutoff => {
                return Err(TrustError::KeyExpired(key_id.to_owned()))
            }
            Standing::Retired(_) => {}
        }
        UnparsedPublicKey::new(&ED25519, &key.public)
            .verify(digest.as_bytes(), signature)
            .map_err(|_| TrustError::BadSignature(key_id.to_owned()))
    }
}

/// What the agent requires of the modules of a plan.
///
/// The default has no trust store and no developer mode. It behaves like a
/// controller that nobody configured for signing: signatures are not
/// required and not checked, and the load logs one warning.
#[derive(Debug, Clone)]
pub struct ModulePolicy {
    /// The keys that `--module-trust-store` named, or `None`.
    pub store: Option<TrustStore>,
    /// `--dev-modules`: load a module whose signature check fails, with a
    /// warning for each.
    pub dev_modules: bool,
    /// The clock that decides whether a retired key still verifies.
    pub now: fn() -> SystemTime,
}

impl Default for ModulePolicy {
    fn default() -> Self {
        Self {
            store: None,
            dev_modules: false,
            now: SystemTime::now,
        }
    }
}

impl ModulePolicy {
    /// Returns a policy that requires signatures from the keys of `store`.
    pub fn with_store(store: TrustStore) -> Self {
        Self {
            store: Some(store),
            ..Self::default()
        }
    }

    /// Judges the signature of one module. `Ok(None)` means the module is
    /// accepted. `Ok(Some(warning))` means it is accepted with a warning, and
    /// `Err` means it is refused.
    pub fn judge(
        &self,
        module: &str,
        digest: &str,
        key_id: &str,
        signature: &[u8],
    ) -> Result<Option<String>, TrustError> {
        let Some(store) = &self.store else {
            return Ok(None);
        };
        match store.verify(digest, key_id, signature, (self.now)()) {
            Ok(()) => Ok(None),
            Err(e) if self.dev_modules => Ok(Some(format!(
                "module {module} ({digest}) is not trusted: {e}; loading it because of --dev-modules"
            ))),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Golden values of controller/internal/registry/signing/golden_test.go:
    // the key with seed 0x07 repeated 32 times. A second implementation
    // (Python cryptography) made the same bytes.
    const KEY7_PUBLIC: &str = "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=";
    const KEY7_ID: &str = "ed25519:fe812c12f3ab4ce6ac5db69ac352f906";
    const GOLDEN_DIGEST: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000001";
    const GOLDEN_SIGNATURE: &str = "b6077b349e70b3bd4901a0464136bfdab325845510be59c11f10baf577d9a9e10452b825683efb58dd7db1a75d3ce3dbbeb003193b374464a19a01692e8ffe0e";
    // Keys of seeds 8 and 9, for the retired and the revoked standing.
    const KEY8_PUBLIC: &str = "E5j2LG0aRXxRumpLXz29L2n8qTIWIY3ImX5Ba9F9k8o=";
    const KEY8_ID: &str = "ed25519:5c29b78f10a35a49a6231d08ee840a04";
    const KEY9_PUBLIC: &str = "/RckOFqgx1tk+3jNYC+h2ZH96/drE8WO1wLqyDXp9hg=";
    const KEY9_ID: &str = "ed25519:dbc298251c51321b7266e78d1c151c2b";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn b64(s: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(s).unwrap()
    }

    fn store_json(cutoff: &str) -> String {
        format!(
            r#"{{"version": 1, "keys": [
  {{"key_id": "{KEY7_ID}", "public_key": "{KEY7_PUBLIC}", "status": "active"}},
  {{"key_id": "{KEY8_ID}", "public_key": "{KEY8_PUBLIC}", "status": "retired", "retired_after": "{cutoff}"}},
  {{"key_id": "{KEY9_ID}", "public_key": "{KEY9_PUBLIC}", "status": "revoked"}}
]}}"#
        )
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn key_id_matches_the_golden_vector() {
        assert_eq!(key_id(&b64(KEY7_PUBLIC)), KEY7_ID);
        assert_eq!(key_id(&b64(KEY8_PUBLIC)), KEY8_ID);
        assert_eq!(key_id(&b64(KEY9_PUBLIC)), KEY9_ID);
    }

    #[test]
    fn golden_digest_signature_verifies() {
        let store = TrustStore::parse(store_json("2030-01-01T00:00:00Z").as_bytes()).unwrap();
        assert_eq!(store.len(), 3);
        store
            .verify(GOLDEN_DIGEST, KEY7_ID, &unhex(GOLDEN_SIGNATURE), at(0))
            .unwrap();
    }

    #[test]
    fn a_flipped_bit_or_another_digest_fails() {
        let store = TrustStore::parse(store_json("2030-01-01T00:00:00Z").as_bytes()).unwrap();
        let mut sig = unhex(GOLDEN_SIGNATURE);
        sig[10] ^= 1;
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, KEY7_ID, &sig, at(0)),
            Err(TrustError::BadSignature(_))
        ));
        let other = format!("sha256:{}", "0".repeat(64));
        assert!(matches!(
            store.verify(&other, KEY7_ID, &unhex(GOLDEN_SIGNATURE), at(0)),
            Err(TrustError::BadSignature(_))
        ));
    }

    #[test]
    fn checks_run_in_the_order_of_the_go_verifier() {
        let store = TrustStore::parse(store_json("1970-01-01T00:01:40Z").as_bytes()).unwrap();
        let sig = unhex(GOLDEN_SIGNATURE);
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, "", &[], at(0)),
            Err(TrustError::NoSignature)
        ));
        // A key id without a signature is a length error, as in Go.
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, KEY7_ID, &[], at(0)),
            Err(TrustError::SignatureLength { got: 0 })
        ));
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, KEY7_ID, &sig[..63], at(0)),
            Err(TrustError::SignatureLength { got: 63 })
        ));
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, "ed25519:00", &sig, at(0)),
            Err(TrustError::UnknownKey(_))
        ));
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, KEY9_ID, &sig, at(0)),
            Err(TrustError::KeyRevoked(_))
        ));
        // Retired at second 100: valid at 99, refused at 100 and after.
        // The signature is not key 8's, so a pass of the standing check shows
        // as a bad signature.
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, KEY8_ID, &sig, at(99)),
            Err(TrustError::BadSignature(_))
        ));
        assert!(matches!(
            store.verify(GOLDEN_DIGEST, KEY8_ID, &sig, at(100)),
            Err(TrustError::KeyExpired(_))
        ));
        assert!(matches!(
            store.verify("sha256:abc", KEY7_ID, &sig, at(0)),
            Err(TrustError::Digest(_))
        ));
    }

    #[test]
    fn the_parser_refuses_what_the_go_parser_refuses() {
        let good = store_json("2030-01-01T00:00:00Z");
        assert!(TrustStore::parse(good.as_bytes()).is_ok());
        let cases = [
            good.replace("\"version\": 1", "\"version\": 2"),
            good.replace("\"keys\"", "\"extra\": 1, \"keys\""),
            format!("{good} {{}}"),
            good.replace(KEY7_ID, "ed25519:00000000000000000000000000000000"),
            good.replace("\"status\": \"active\"", "\"status\": \"gone\""),
            good.replace(
                "\"status\": \"active\"",
                "\"status\": \"active\", \"retired_after\": \"2030-01-01T00:00:00Z\"",
            ),
            good.replace("2030-01-01T00:00:00Z", "tomorrow"),
            good.replace(KEY7_PUBLIC, "AAAA"),
            good.replace(
                "\"status\": \"revoked\"",
                "\"status\": \"revoked\", \"unknown\": 1",
            ),
        ];
        for (i, c) in cases.iter().enumerate() {
            assert!(TrustStore::parse(c.as_bytes()).is_err(), "case {i}");
        }
        let dup = format!(
            r#"{{"version": 1, "keys": [
  {{"key_id": "{KEY7_ID}", "public_key": "{KEY7_PUBLIC}", "status": "active"}},
  {{"key_id": "{KEY7_ID}", "public_key": "{KEY7_PUBLIC}", "status": "active"}}]}}"#
        );
        assert!(TrustStore::parse(dup.as_bytes()).is_err());
        let empty = TrustStore::parse(br#"{"version": 1, "keys": []}"#).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn policy_without_a_store_accepts_everything() {
        let p = ModulePolicy::default();
        assert_eq!(p.judge("m", GOLDEN_DIGEST, "", &[]).unwrap(), None);
    }

    #[test]
    fn policy_refuses_in_strict_mode_and_warns_in_developer_mode() {
        let store = TrustStore::parse(store_json("2030-01-01T00:00:00Z").as_bytes()).unwrap();
        let mut p = ModulePolicy::with_store(store);
        assert!(matches!(
            p.judge("m", GOLDEN_DIGEST, "", &[]),
            Err(TrustError::NoSignature)
        ));
        assert_eq!(
            p.judge("m", GOLDEN_DIGEST, KEY7_ID, &unhex(GOLDEN_SIGNATURE))
                .unwrap(),
            None
        );
        p.dev_modules = true;
        let w = p.judge("m", GOLDEN_DIGEST, "", &[]).unwrap().unwrap();
        assert!(w.contains("module m") && w.contains("--dev-modules"), "{w}");
        // A valid signature raises no warning in developer mode.
        assert_eq!(
            p.judge("m", GOLDEN_DIGEST, KEY7_ID, &unhex(GOLDEN_SIGNATURE))
                .unwrap(),
            None
        );
    }
}
