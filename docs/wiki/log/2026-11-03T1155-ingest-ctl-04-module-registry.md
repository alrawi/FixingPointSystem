## [2026-11-03] ingest | CTL-04 Module registry

We added the package `controller/internal/registry/`, which keeps installed modules in a local directory with an `index.json` file and serves `listModules`, `installModule`, `getModule`, and `deleteModule` through a type that embeds `api.Unimplemented`. An install unpacks a tar archive or copies a directory, validates the manifest against `contracts/schemas/module-manifest.schema.json`, checks the SPDX license expression against a GPL-3.0 compatible allow list, checks the files that the manifest names, and records the module tree digest that plans carry. The example `http1` and `dns` modules install with the digests of `contracts/examples/plans/http_dns_smoke.json`, and manifests with unknown or incompatible licenses fail with a 422 problem whose finding names `/license`. `registry.Lookup` gives CTL-03 the manifest of a module by name and version, or the latest version. We added the module registry section to [Controller](Controller.md). The evidence is `docs/handoff/CTL-04.md` and the tests in `controller/internal/registry/`.

## Shared page notes

- [Home](Home.md), Controller row. Add "module registry" to the summary.
- [Testing](Testing.md), the CTL rows. `make test-ctl` now also runs the registry tests, including the check that the embedded manifest schema matches the contract and that the example module digests match the example plan.
- Follow up for the CTL workstream. The binary does not mount the registry yet. The package that wires the API into `controller/cmd/fps-controller` opens `registry.Open(<data dir>/modules, registry.Limits{})` and passes `registry.NewAPI(reg, logger)`, or a type that embeds it, to `api.New`. The controller configuration needs a data directory setting for that.
- Follow up for CTL-03. The configuration store resolves mix entries with `registry.Lookup` and `Registry.List`, which returns the scenario names of every installed module.
- Follow up for CTL-11 (signing). Archives are not signed yet, and `ModuleSummary.signed_by` stays empty until the signing package verifies archives before `Registry.Install`.
