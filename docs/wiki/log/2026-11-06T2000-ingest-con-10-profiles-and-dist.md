## [2026-11-06] ingest | CON-10 behavior profiles and distributions

Scenario contract v0.3 adds the top level `profiles` object and the template functions `dist.pareto`, `dist.lognormal`, `dist.exponential`, and `dist.uniform` (decision D12 item 18). The specification example now validates as `contracts/examples/modules/http1/scenarios-pending/web_profile.yaml`. The change is additive, and the entry `contracts/changes/CON-10.md` states the compatibility, including the new reserved names `profiles` and `dist`. The validator checks both features, and the example plan and the compiler golden plans carry the new `http1` module digest.

### Shared page notes

- `contracts/README.md` still describes scenario contract v0.2. The change note ends with the grammar text to fold, because the file lies outside this package.
- [Testing](../Testing.md) and the engine: neither the Rust template parser nor the controller compiler knows `profiles` or `dist`, so a follow up package must teach them before the pending example moves into `scenarios/`.
