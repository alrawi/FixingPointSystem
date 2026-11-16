## [2026-11-16] ingest | CON-33 Example tests pair with the example modules

Issue #482 found that `contracts/examples/tests/http_smoke.yaml` names `http1/web_get`, a scenario of the example http1 module, and not one of the first party http1 module. The fix stays inside the examples (`contracts/changes/CON-33.md`, approved by the vetting agent under D13 on 2026-11-16, with no schema or version change). `http_smoke.yaml` keeps its name and gets a first line that says it pairs with the example modules, `contracts/README.md` says that an example test pairs with the example modules unless it names a first party scenario, and the new `contracts/examples/tests/http1_first_party.yaml` runs `http1/single_fetch`. `tools/contracts/validate.py` validates it against `modules/http1` through `FIRST_PARTY_TESTS`, as it does `dns_zone.yaml`. [Contracts](Contracts.md) has a row for it.

Decisions.

- `http1/web_get` is not renamed. The validator, the plan example, the UI mock, the CLI end to end test, and spike S4 rely on it.
- The tests in `tools/contracts/test_contracts.py` show that the new test passes against the first party module and that a scenario no first party manifest defines (`http1/web_get`) fails with an unknown scenario error.

Findings.

- Two readers of the whole `examples/tests/` directory sit outside the paths of CON-33 and need an edit with the new file: the controller store tests (`exampleModuleScenarios` in `controller/internal/store/helpers_test.go` maps `dns/lookup_a` to `dns/resolve` and needs `http1/single_fetch` to `http1/web_get`) and the UI test `ConfigEditor.test.tsx` (the stored configuration count goes from 6 to 7). The change entry lists them.

### Shared page notes

None.
