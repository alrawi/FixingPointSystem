# CON-33 Example tests pair with the example modules

Contract. The example test configurations under `contracts/examples/tests/`, `contracts/README.md`, and the validator `tools/contracts/validate.py`. Issue #482 found that `http_smoke.yaml` names `http1/web_get`, a scenario of the example http1 module under `contracts/examples/modules/http1`, and that the first party http1 module under `modules/http1` has no scenario of that name. A reader could take the test for one of the first party module. The change is a fix inside the examples, with no schema or version change, and the vetting agent approved it under decision D13 on 2026-11-16.

## Versions

No contract version moves. The test configuration stays at v0.3, the scenario contract at v0.6, the module manifest schema, the metric catalog, OpenAPI, and the agent protocol stay as they are. No schema, grammar, or verdict changes. The validator gains one entry in a table of example names.

## Changes

- `contracts/examples/tests/http_smoke.yaml` keeps its name and its scenario `http1/web_get`, because the validator, the plan example, the UI mock, the CLI end to end test, and spike S4 rely on it. A comment on its first line says that it pairs with the example modules, and that the first party http1 scenario is `http1/single_fetch`.
- `contracts/README.md`, the row `examples/`, says that an example test pairs with the example modules of `examples/modules/`, and that a test that names a scenario of a first party module pairs with that module.
- New `contracts/examples/tests/http1_first_party.yaml` runs `http1/single_fetch`. `FIRST_PARTY_TESTS` in `tools/contracts/validate.py` lists it with `http1`, as it lists `dns_zone.yaml` with `dns`, so the validator checks it against `modules/http1`.
- `tools/contracts/test_contracts.py` tests the pairing: the new test passes against the first party module, a scenario that no first party manifest defines (`http1/web_get`) fails with an unknown scenario error, and `http_smoke.yaml` stays outside `FIRST_PARTY_TESTS`.
- `docs/wiki/Contracts.md` has a row for the new example.

## Compatibility

`http1/web_get` is not renamed, so every reader of `http_smoke.yaml` keeps working. A reader that takes every file of `contracts/examples/tests/` sees one more test, and it must resolve `http1/single_fetch` from the first party module, as it resolves `dns/lookup_a` for `dns_zone.yaml`. See the consumers below.

## Consumers

Readers that take every file of `examples/tests/`:

| Reader | Effect of the new file |
|---|---|
| `tools/contracts/validate.py`, `tools/contracts/test_contracts.py` | Handled in this package. The default run validates the file against `modules/http1`, and `python3 -m pytest tools/contracts` passes |
| `engine/fps-hosts/tests/contract_examples.rs` | Reads only the host pools, which have the shape of the other examples. No change |
| `controller/internal/store` tests (`examples()` in `helpers_test.go`, used by `TestValidateEveryExample`, `TestEveryExampleStoresAndValidates`, and others) | They validate every example against the example modules. `exampleModuleScenarios` maps the first party `dns/lookup_a` of `dns_zone.yaml` to `dns/resolve` for that reason, and `http1/single_fetch` needs the same map, to `http1/web_get`. This is a test edit outside the paths of CON-33, so the coordinator decides who makes it |
| `ui/mock/fixtures.ts` `loadExampleConfigs`, `ui/src/pages/ConfigEditor.test.tsx` | The mock stores seven configurations in place of six, so the `toHaveLength(6)` check of the test "validates a stored configuration without storing another copy" becomes 7. This is a test edit outside the paths of CON-33 as well |

Readers that name one file (`http_smoke.yaml` for the UI mock, the CLI end to end test, spike S4, and the plan example, `dns_zone.yaml` and `port_locations.yaml` for the controller settings tests) do not see the new file.

## Verification

`python3 tools/contracts/validate.py` passes, `python3 -m pytest tools/contracts` passes, and `python3 tools/plan/plan.py check` passes.
