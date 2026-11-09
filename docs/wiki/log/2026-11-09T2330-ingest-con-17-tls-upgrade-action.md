## [2026-11-09] ingest | CON-17 The tls_upgrade action in the scenario contract

CON-17 adds the flow action `tls_upgrade` to the scenario schema and moves the scenario contract from v0.3 to v0.4, an additive minor change like CON-04 and CON-10. The validator reports the action in a UDP flow as an error, at any depth of `loop` and `branch`, and `tools/contracts/test_contracts.py` covers a valid TCP scenario and the UDP failure. The controller's embedded copy of the scenario schema is refreshed so the Go registry accepts the keyword. We described the change in [Contracts](../Contracts.md#the-tls_upgrade-action).

### Shared page notes

- Engine, for an ENG package: `check_action` of `fps-flow` should reject `tls_upgrade` in a UDP flow with a new `LoadError` kind (for example `TlsUpgradeOnUdp`), and `engine/fps-flow/tests/fixtures/verdicts.yaml` should gain a case so the Rust test and `test_fixtures.py` agree with the validator. It was left out of CON-17, because `engine/fps-flow/` is outside its paths.
- Go: no code change is needed. `controller/internal/registry/schema/scenario.schema.json` is a byte copy of the contract and was refreshed in this pull request, as `TestEmbeddedScenarioSchemaMatchesContract` requires. Go cannot build in the worker sandbox, so CI runs it.
- [Testing](../Testing.md) and the plan: no change. MOD-20 (STARTTLS variants) depends on CON-17 and AGT-14.
