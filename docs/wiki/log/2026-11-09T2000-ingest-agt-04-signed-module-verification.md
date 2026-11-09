## [2026-11-09] ingest | AGT-04 signed module verification

The agent verifies the signature of every plan module against a trust store before it loads the module. `--module-trust-store FILE` takes the file format of the controller's `trust_store_file` (CTL-11), and `--dev-modules` accepts a module that fails the signature check with one warning per module. The signature is Ed25519 over the UTF-8 bytes of the tree digest, which `ModuleRef.signature` and `signature_key_id` carry. The plan mirror of the agent now reads both fields, which its strict JSON reader refused before. Without a trust store the agent requires no signature and logs one warning, like a controller without a trust store, so the controller end to end test with unsigned example modules is unchanged. A tree digest that differs from the plan digest fails in both modes. Tests reuse the seed 0x07 vectors of CTL-11 and a tree digest vector that Python computed. See [Agent](../Agent.md#signed-modules).

### Shared page notes

- Controller page: the sentence "The agent verifies the signature against its own trust store (AGT-04)" is true now. The key rotation steps need one more line: a rotation reaches an agent only when the new trust store file is on the agent and the agent restarted, because the agent reads the file at startup.
- Testing: `tests/signing.rs` of `fps-agent` is a component test that needs no root and no network.
- Implementation Plan: AGT-04 of phase 2 is done once merged.
- Paths outside the catalog entry: `Cargo.lock` gains `ring` in the dependency list of `fps-agent`. The package was in the lock already (rustls).
