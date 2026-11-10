# CON-18 The transport keyword of the scenario contract

Contract. Scenario schema `contracts/schemas/scenario.schema.json` and the scenario rules of `tools/contracts/validate.py`. The change takes finding 1 of MOD-04. The owner approved it.

Version. The scenario contract moves from v0.4 to v0.5. The keyword was in the schema since v0.1, and no loader ever accepted it, so no scenario in the wild uses it. The change gives it a meaning and tightens its body, which takes a minor number, as CON-17 (v0.4) did. Every v0.4 scenario that a loader accepted stays valid and keeps its meaning. The module manifest schema (v0.3), the test configuration schema (v0.2), the controller API, and the agent protocol keep their versions.

Decision. Option A, which defines `transport` as sugar for a leading `tls_upgrade`, and not option B, which removes the keyword.

- The engine cost is low. The compiler of `fps-flow` already builds `Op::UpgradeTls` for the action, so the sugar is one op pushed in front of each side's body (about 40 lines with the body check) and one flow level rule for UDP that reuses `TlsUpgradeOnUdp` of ENG-22. The runtime, the interpreter, and the agent need no change, because a desugared scenario holds the same ops as one that writes the actions.
- Option B would be a breaking schema change for no gain. It would leave `docs/spec/specification.md` (the `web_browse` style example with `transport: { module: tls, profile: chrome_desktop }`) describing a keyword that the contract rejects, and every module that wants TLS would repeat two near identical actions per flow.
- The keyword reads better than two actions where both sides want the same profile, which is the common case of an implicit TLS service such as HTTPS on 443, SMTPS on 465, and IMAPS on 993.

Change. `$defs/flow/properties/transport` becomes an object with `additionalProperties: false`, the required `module` (the constant `tls`, the only transport module) and `profile` (a non empty string, a name of a TLS profile of the run), and the optional `server_name` (a non empty string). Before, `module` was any identifier and `profile` was optional and unconstrained. The old shape never ran, so the tightening breaks nothing that worked.

Semantics. `transport: {module: tls, profile: p, server_name: n}` is exactly this rewrite of the flow, which the loader performs:

- The client side gets `tls_upgrade: {profile: p, server_name: n}` as the action right after `connect`, so the client role starts the handshake.
- The server side, when the flow declares one, gets `tls_upgrade: {profile: p}` right after `accept`, so the server role answers it. `server_name` is for the client handshake, so the server side leaves it out.
- Both sides use the same profile name. A flow whose sides need different profiles (the HTTPS scenarios of MOD-04 do, with a browser profile on the client and `server-tls13` on the server) keeps writing `tls_upgrade` on each side, which stays valid.
- The sugar composes with explicit actions. They run after it, in declaration order, so a `tls_upgrade` that follows the sugar wraps the TLS session again. `tls_upgrade` rules apply to the sugar unchanged.

Semantic rule. `tools/contracts/validate.py` reports `transport` in a flow with `l4: udp` as an error at `/flows/<i>/transport` with the message "transport (sugar for tls_upgrade) applies only to TCP flows". The validator does not check that the profile exists, because the profiles belong to the run. The tests in `tools/contracts/test_contracts.py` cover a valid keyword with and without `server_name`, the composition with `tls_upgrade`, a UDP flow, and malformed bodies (missing and unknown fields, another module, empty strings, a `role`, a non mapping).

Loader parity. `engine/fps-flow` is outside the catalog paths of CON-18, and the validator and the loader must agree, so this pull request makes the minimal change there. `Scenario::load` no longer answers `unsupported: a transport module`. The compiler parses the keyword (the same rules as `tls_upgrade`, plus `module: tls`) and pushes the `UpgradeTls` op after the opener of each side. The check reports `TlsUpgradeOnUdp` at `/flows/<i>/transport`. The shared fixtures `transport_tcp.yaml` (accept) and `transport_udp.yaml` (reject, pointer `/flows/0/transport`) in `engine/fps-flow/tests/fixtures/verdicts.yaml` run in the Rust test and in `test_fixtures.py`, and a unit test in `program.rs` pins the desugared ops and the jump targets.

Compatibility. The controller embeds a copy of the scenario schema (`controller/internal/registry/schema/scenario.schema.json`), which this change refreshes byte for byte, because `TestEmbeddedScenarioSchemaMatchesContract` compares the two. A session of a plan that uses the keyword needs the agent to build TLS profiles (AGT-14), as for `tls_upgrade`.

Outside the catalog paths, for the coordinator. The text of `modules/http1/README.md` and `docs/wiki/Modules.md` says that `Scenario::load` rejects the keyword, which is no longer true. The MOD-04 scenarios keep `tls_upgrade` because their sides use different profiles, so no scenario needs to change, and a later MOD change can reword the two passages. `docs/spec/specification.md` already shows the keyword, so it now matches the contract. Line 46 of `docs/wiki/Engine.md` says that `Scenario::load` still rejects transport modules, and its TLS upgrade section can name the keyword. Both belong to the ENG page.
