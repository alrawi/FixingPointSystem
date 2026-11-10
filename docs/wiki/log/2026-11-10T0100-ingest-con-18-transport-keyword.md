## [2026-11-10] ingest | CON-18 The transport keyword

We decided the flow keyword `transport` of the scenario contract and moved the contract to v0.5. The keyword is now sugar for a leading `tls_upgrade` on each side, the client role on the client side with `server_name` and the server role on the server side, and the validator and `Scenario::load` both accept it. It applies only to TCP flows, and a UDP flow reports the existing `TlsUpgradeOnUdp` verdict. We chose this over removing the keyword because the engine already built `Op::UpgradeTls`, so the sugar costs one op per side. The change to `engine/fps-flow` is minimal and sits in this package because the two validators must agree. The evidence is `contracts/changes/CON-18.md`, `tools/contracts/test_contracts.py`, and the shared fixtures `transport_tcp.yaml` and `transport_udp.yaml`, which run in the Rust test and in `test_fixtures.py`. We described the rule in [Contracts](Contracts.md#the-transport-keyword).

### Shared page notes

- [Engine](Engine.md): line 46 says that `Scenario::load` still rejects transport modules. It desugars `transport` now, and the TLS upgrade section can name the keyword.
- [Modules](Modules.md) and `modules/http1/README.md`: both say that the loader rejects `transport: {module: tls}`. The MOD-04 scenarios keep `tls_upgrade`, because their sides use different profiles, so only the wording changes.
- Contracts: `contracts/changes/README.md` says the coordinator folds entries into `CHANGELOG.md`. CON-18 also added its own section to `contracts/CHANGELOG.md`, as the package deliverable asked, so the coordinator need not fold it again.
