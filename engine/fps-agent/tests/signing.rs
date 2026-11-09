// SPDX-License-Identifier: GPL-3.0-only
//! Signed module verification (AGT-04).
//!
//! The tests use the module trees and the plan of `contracts/examples`. They
//! sign the tree digests with the key of seed 0x07, the key of the CTL-11
//! golden vectors of `controller/internal/registry/signing`, so a signature
//! here is the signature that the controller would make. The golden test at
//! the top pins a tree digest and its signature that Python `cryptography`
//! made from the rule of `contracts/README.md`, so the Rust tree digest, the
//! Ed25519 check, and the trust store format agree with the Go code.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use fps_agent::modules::{load, tree_digest, ModuleError};
use fps_agent::plan::Plan;
use fps_agent::trust::{key_id, ModulePolicy, TrustError, TrustStore};
use ring::signature::{Ed25519KeyPair, KeyPair};

fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

struct Signer {
    pair: Ed25519KeyPair,
    id: String,
}

fn signer(seed: u8) -> Signer {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("seed");
    let id = key_id(pair.public_key().as_ref());
    Signer { pair, id }
}

impl Signer {
    fn sign(&self, digest: &str) -> Vec<u8> {
        self.pair.sign(digest.as_bytes()).as_ref().to_vec()
    }

    fn entry(&self, status: &str, cutoff: Option<&str>) -> String {
        let public = base64::engine::general_purpose::STANDARD.encode(self.pair.public_key());
        let retired = cutoff
            .map(|c| format!(r#", "retired_after": "{c}""#))
            .unwrap_or_default();
        format!(
            r#"{{"key_id": "{}", "public_key": "{public}", "status": "{status}"{retired}}}"#,
            self.id
        )
    }
}

fn store(entries: &[String]) -> TrustStore {
    let text = format!(r#"{{"version": 1, "keys": [{}]}}"#, entries.join(","));
    TrustStore::parse(text.as_bytes()).expect("trust store")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            fs::copy(e.path(), target).unwrap();
        }
    }
}

/// A fresh copy of the example module directory.
fn modules_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("fps-agent-signing-{tag}"));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&contracts().join("examples/modules"), &dir);
    dir
}

/// The example plan with every module signed by `by`.
fn signed_plan(by: &Signer) -> Plan {
    let text = fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json")).unwrap();
    let mut plan = Plan::parse(&text).unwrap();
    assert!(plan.modules.len() >= 2);
    for m in &mut plan.modules {
        m.signature = by.sign(&m.digest);
        m.signature_key_id = by.id.clone();
    }
    plan
}

fn strict(store: TrustStore) -> ModulePolicy {
    ModulePolicy::with_store(store)
}

fn dev(store: TrustStore) -> ModulePolicy {
    ModulePolicy {
        dev_modules: true,
        ..ModulePolicy::with_store(store)
    }
}

fn untrusted(err: ModuleError) -> TrustError {
    match err {
        ModuleError::Untrusted { source, .. } => source,
        other => panic!("want Untrusted, got {other}"),
    }
}

#[test]
fn golden_tree_digest_and_signature_agree_with_the_go_implementation() {
    // The tree of the vector: manifest.yaml, scenarios/a.yaml, and b.bin with
    // the bytes 0..=255. Python computed rule 1 over it independently.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-signing-golden");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("scenarios")).unwrap();
    fs::write(dir.join("manifest.yaml"), "name: demo\nversion: 0.1.0\n").unwrap();
    fs::write(dir.join("scenarios/a.yaml"), "x: 1\n").unwrap();
    fs::write(dir.join("b.bin"), (0..=255u8).collect::<Vec<_>>()).unwrap();
    let digest = tree_digest("demo", &dir).unwrap();
    assert_eq!(
        digest,
        "sha256:4b2ff2efafa228cf448d2e1e63f97a94c6a1157219f653e00d5a21a21a59ccb7"
    );

    let key7 = signer(7);
    assert_eq!(key7.id, "ed25519:fe812c12f3ab4ce6ac5db69ac352f906");
    let sig = hex(&key7.sign(&digest));
    assert_eq!(
        sig,
        "63779998a0c944bc269557b291398d3b89ac481f76b70b006cde83e6d48f78db4a6e8e7534f3e25f9302fc4008ed1efad6aac1fca7f0c718ba01e70cb6691007"
    );
    // The CTL-11 golden signature of the golden digest, as the controller
    // test pins it.
    let golden = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
    assert_eq!(
        hex(&key7.sign(golden)),
        "b6077b349e70b3bd4901a0464136bfdab325845510be59c11f10baf577d9a9e10452b825683efb58dd7db1a75d3ce3dbbeb003193b374464a19a01692e8ffe0e"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn a_valid_signature_loads_without_a_warning() {
    let key = signer(7);
    let dir = modules_dir("valid");
    let plan = signed_plan(&key);
    let loaded = load(&plan, &dir, &strict(store(&[key.entry("active", None)]))).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.set.len(), plan.modules.len());
}

#[test]
fn a_signature_in_the_canonical_json_plan_reaches_the_check() {
    // The plan JSON carries the signature as base64 under the proto3 JSON
    // names, and the reader refuses no field of it.
    let key = signer(7);
    let dir = modules_dir("json");
    let text = fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    for m in json["modules"].as_array_mut().unwrap() {
        let sig = key.sign(m["digest"].as_str().unwrap());
        m["signature"] = base64::engine::general_purpose::STANDARD
            .encode(sig)
            .into();
        m["signatureKeyId"] = key.id.clone().into();
    }
    let plan = Plan::parse(&json.to_string()).unwrap();
    assert_eq!(plan.modules[0].signature_key_id, key.id);
    load(&plan, &dir, &strict(store(&[key.entry("active", None)]))).unwrap();
}

#[test]
fn a_tampered_tree_is_refused_even_in_developer_mode() {
    let key = signer(7);
    let dir = modules_dir("tamper");
    let plan = signed_plan(&key);
    let trust = store(&[key.entry("active", None)]);
    // A byte of the http1 manifest changes after signing, so the tree digest
    // no longer equals the digest that the plan names and the key signed.
    let manifest = dir.join("http1/manifest.yaml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("# changed\n");
    fs::write(&manifest, text).unwrap();
    for policy in [strict(trust.clone()), dev(trust)] {
        let err = load(&plan, &dir, &policy).unwrap_err();
        assert!(matches!(err, ModuleError::Digest { .. }), "{err}");
    }
}

#[test]
fn a_tree_that_the_plan_digest_was_rewritten_for_fails_the_signature() {
    // An attacker who controls the plan changes the tree and writes the new
    // digest into the plan, which passes the digest check. The signature
    // still covers the old digest, so it fails, and developer mode accepts
    // the module with a warning.
    let key = signer(7);
    let dir = modules_dir("rewrite");
    let mut plan = signed_plan(&key);
    let manifest = dir.join("http1/manifest.yaml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("# changed\n");
    fs::write(&manifest, text).unwrap();
    let m = plan.modules.iter_mut().find(|m| m.name == "http1").unwrap();
    m.digest = tree_digest("http1", &dir.join("http1")).unwrap();
    let trust = store(&[key.entry("active", None)]);

    let err = untrusted(load(&plan, &dir, &strict(trust.clone())).unwrap_err());
    assert!(matches!(err, TrustError::BadSignature(_)), "{err}");

    let loaded = load(&plan, &dir, &dev(trust)).unwrap();
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(loaded.warnings[0].contains("module http1"));
}

#[test]
fn an_unknown_key_is_refused_and_developer_mode_warns() {
    let signed_by = signer(7);
    let other = signer(8);
    let dir = modules_dir("unknown");
    let plan = signed_plan(&signed_by);
    let trust = store(&[other.entry("active", None)]);
    let err = untrusted(load(&plan, &dir, &strict(trust.clone())).unwrap_err());
    assert!(matches!(err, TrustError::UnknownKey(ref id) if *id == signed_by.id), "{err}");
    let loaded = load(&plan, &dir, &dev(trust)).unwrap();
    assert_eq!(loaded.warnings.len(), plan.modules.len(), "one per module");
    assert!(loaded.warnings.iter().all(|w| w.contains("unknown signing key")));
}

#[test]
fn an_unsigned_module_is_refused_with_a_trust_store_and_developer_mode_warns() {
    let key = signer(7);
    let dir = modules_dir("unsigned");
    let mut plan = signed_plan(&key);
    plan.modules[0].signature.clear();
    plan.modules[0].signature_key_id.clear();
    let trust = store(&[key.entry("active", None)]);
    let err = untrusted(load(&plan, &dir, &strict(trust.clone())).unwrap_err());
    assert!(matches!(err, TrustError::NoSignature), "{err}");
    let loaded = load(&plan, &dir, &dev(trust)).unwrap();
    // Only the unsigned module warns, the signed one verifies.
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(loaded.warnings[0].contains("no signature"));
}

#[test]
fn a_revoked_key_never_verifies_and_a_retired_key_stops_at_its_cutoff() {
    let key = signer(7);
    let dir = modules_dir("standing");
    let plan = signed_plan(&key);

    let err = untrusted(
        load(&plan, &dir, &strict(store(&[key.entry("revoked", None)]))).unwrap_err(),
    );
    assert!(matches!(err, TrustError::KeyRevoked(_)), "{err}");

    // The cutoff is 1970-01-01T00:01:40Z, second 100.
    let retired = store(&[key.entry("retired", Some("1970-01-01T00:01:40Z"))]);
    let clock = |secs: u64| -> fn() -> SystemTime {
        match secs {
            99 => || SystemTime::UNIX_EPOCH + Duration::from_secs(99),
            _ => || SystemTime::UNIX_EPOCH + Duration::from_secs(100),
        }
    };
    let before = ModulePolicy {
        now: clock(99),
        ..strict(retired.clone())
    };
    load(&plan, &dir, &before).unwrap();
    let after = ModulePolicy {
        now: clock(100),
        ..strict(retired.clone())
    };
    let err = untrusted(load(&plan, &dir, &after).unwrap_err());
    assert!(matches!(err, TrustError::KeyExpired(_)), "{err}");
    // Developer mode accepts the retired key with a warning.
    let after_dev = ModulePolicy {
        dev_modules: true,
        ..after
    };
    let loaded = load(&plan, &dir, &after_dev).unwrap();
    assert_eq!(loaded.warnings.len(), plan.modules.len());
}

#[test]
fn without_a_trust_store_signatures_are_not_required_and_one_warning_is_logged() {
    let dir = modules_dir("nostore");
    // The example plan is unsigned, like the plans of a controller that
    // nobody configured for signing.
    let text = fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json")).unwrap();
    let plan = Plan::parse(&text).unwrap();
    let loaded = load(&plan, &dir, &ModulePolicy::default()).unwrap();
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(loaded.warnings[0].contains("--module-trust-store"));
    // A signature that does not verify is ignored without a store, as the
    // controller registry ignores it.
    let mut junk = Plan::parse(&text).unwrap();
    junk.modules[0].signature = vec![1; 64];
    junk.modules[0].signature_key_id = "ed25519:00".into();
    load(&junk, &dir, &ModulePolicy::default()).unwrap();
}
