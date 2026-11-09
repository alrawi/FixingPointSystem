## [2026-11-09] ingest | CTL-31 CA in the enrollment chain and trust store flags

`serverCertificate` in `controller/internal/pki/tlsconfig.go` now presents `[leaf, CA]`, so the gRPC listener (`ServerTLSConfig`) and the enrollment HTTPS listener (`EnrollTLSConfig`) both send the CA certificate after the leaf. This is finding (a) of the AGT-08 log entry. An agent that pins the CA fingerprint can now find the CA in the handshake before it sends the enrollment token, and an agent that trusts a CA file verifies the same chain as before. `TestServerPresentsCAAfterLeaf` (`internal/pki/chain_test.go`) dials both listeners and reads the presented chain from the handshake: two certificates, the leaf first, the CA second, with the fingerprint of the authority. `fps-controller` gained the flags `-trust-store-file` and `-allow-unsigned-modules`, which map to `trust_store_file` and `allow_unsigned_modules` with the usual layering (defaults, file, flags), and `TestLoadConfigLayersTrustStoreSettings` shows that they reach the configuration. The [Controller](../Controller.md) page describes both changes.

### Shared page notes

- [Agent](../Agent.md) and `engine/fps-agent/` (not edited here): AGT-10 can now look for the pinned CA in the presented chain before it sends the token. The existing agent needed no change, and the `controller-e2e` job proves that it still works with the longer chain.
- [Controller](../Controller.md): the AGT-08 note that the agent "has no TLS yet" is stale elsewhere on the page, and the coordinator can reword it together with the AGT-08 notes.
