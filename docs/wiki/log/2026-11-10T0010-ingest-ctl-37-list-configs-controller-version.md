## [2026-11-10] ingest | CTL-37 List configurations and controller version

`GET /v1/configs` (`listConfigs`) no longer answers 501. `store.Handler` serves it from the new `Repository.List`, newest first, with `page_size` (1 to 500, default 50) and an opaque `page_token`, in the design of `listRuns`: the order is a creation sequence, and the token names the last sequence of the page, so a configuration stored between two requests never shifts or repeats an item. The sequence is a new column `seq` (`INTEGER PRIMARY KEY AUTOINCREMENT`, schema version 2), which a migration fills from the old rows in creation order, so tokens survive a restart and `VACUUM`. `GET /v1/capabilities` reports `controller_version` from the version stamp of CTL-33 (`buildVersion()` in `cmd/fps-controller`), passed through `api.WithControllerVersion` and `wiring.Options.Version`. `fps config list` asks the controller and uses its history file only when the first request answers 501. The contract tests validate every `listConfigs` response against the OpenAPI document, and the controller and CLI end to end tests cover the listing and the version. We described the changes in [Controller](Controller.md) and [CLI](CLI.md).

The package changed files outside its catalog paths: `controller/internal/wiring/` (the `pending.ListConfigs` stub, `pendingOps` of the wiring test, and `Options.Version`) and `cli/` including `cli/e2e`, because its deliverables and acceptance name them.

### Shared page notes

- Testing: `TestServerRoutesEveryOperation` now expects two pending operations, `getRunMetrics` and `streamRunMetrics`, if a count is stated anywhere.
- Contracts: none. `listConfigs` and `controller_version` are implemented exactly as the OpenAPI document defines them. A dev build reports `(devel)` as `controller_version`, because `buildVersion()` returns it when no stamp and no module version exist.
- Implementation Plan: CTL-37 is done once merged.
