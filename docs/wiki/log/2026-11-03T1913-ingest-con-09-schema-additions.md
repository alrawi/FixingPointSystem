## [2026-11-03] ingest | CON-09 Test configuration and manifest schema additions

We applied decision D12 items 7, 9, 10, 14, and 22 to the contracts and documented them in the section Test configuration and manifest additions of [Contracts](Contracts.md). The test configuration schema v0.2 requires a seed of at least 1, documents the port location rule with the new spread forms `*/<interface>` and `<interface>`, adds an optional `where` label filter to criteria, and adds `modules.<name>.settings`. The module manifest schema v0.3 adds `settings`, a JSON Schema for those settings, and `expected_bytes` per scenario. The example `dns` manifest describes `zone_file`, and both example manifests carry byte estimates. The new example `seedless_smoke.yaml` has no seed, and the examples under `contracts/examples/tests/pending/` use the other additions. The entry `contracts/changes/CON-09.md` lists the changes. The evidence is `docs/handoff/CON-09.md` on the branch `wp/CON-09`.

The change reached files outside the package paths in four mechanical ways, with no logic change. The controller embeds byte identical copies of both schemas, which we refreshed as `go generate` would. The example manifests changed, so the module digests in `contracts/examples/plans/http_dns_smoke.json` and in the golden plans under `controller/internal/compiler/testdata/` changed. The fixture `controller/internal/store/testdata/invalid/bad_location.yaml` used the bare location `lo`, which is now valid, so it now uses `local/lo/extra`.

## Shared page notes

The coordinator may apply the notes below.

1. [Testing](Testing.md) or `mk/con.mk` may add a validator run over `contracts/examples/tests/pending/*.yaml`, so CI checks the pending examples until CTL-18 and CTL-20 move them.
2. CTL-18 and CTL-20 should move the pending examples into `contracts/examples/tests/` as their acceptance, and CTL-18 should drop the rule in the compiler that rejects bare and `*/` locations.
3. A later CON package may teach `tools/contracts/validate.py` three semantic checks, which are module settings against the manifest `settings` schema, `where` keys against the catalog labels of the metric, and `expected_bytes` keys against the scenarios of the module.
