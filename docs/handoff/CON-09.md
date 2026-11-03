# Handoff CON-09 Test configuration and manifest schema additions

| Field | Value |
|---|---|
| Issue | #134 |
| Branch | `wp/CON-09` |
| Pull request | #144 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-con-09-2000, lease until 2026-11-03T20:33:15Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 7, 9, 10, 14, and 22 to the test configuration and module manifest schemas. The additions are a seed of at least 1, the port location rule, an optional where filter on criteria, module settings described by a JSON schema in each manifest, and an expected_bytes estimate per scenario.

## Plan

- [x] 1. Test configuration schema changes for seed, port location, where, and module settings
- [x] 2. Module manifest schema changes for settings_schema and expected_bytes
- [x] 3. New examples for each addition and the change entry
- [x] 4. Embedded controller copy of the schema, if CI compares it
- [x] 5. Wiki ingest, checks, and pull request review

## Done and verified

- Steps 1 to 3. `make test-con` passes (153 tests, validator 0 errors), and `python3 tools/contracts/validate.py --tests contracts/examples/tests/*.yaml contracts/examples/tests-pending/*.yaml` reports 0 errors. A negative check confirmed that seed 0, `a/b/c`, `*/`, an empty `where`, and an unknown key under `modules.<name>` fail the schema.
- Step 4. The embedded copies under `controller/internal/store/schema/` and `controller/internal/registry/schema/` equal the contract. The example module digests changed in `contracts/examples/plans/http_dns_smoke.json` and the compiler golden plans. The fixture `bad_location.yaml` uses `local/lo/extra`, because a bare `lo` is now valid. `go test ./...` in `controller/` and `cargo test -p fps-agent -p fps-flow` pass, and the spike S4 tests pass.
- Step 5. Wiki section in Contracts and log entry written, and `plan.py check`, `render --check`, and `lint` pass.

## Remaining

The validator has no semantic checks for the new fields, and CI does not check the pending examples. The log entry lists both as shared page notes.

## Next action

The owner reviews the contract change on pull request #144.

## Blockers and open questions

The issue carries needs:owner, so the owner approves the contract change.

## Decisions taken in this package

- The manifest key is `settings`, a JSON Schema like `parameters`, so the two schema keys read alike.
- `expected_bytes` is a map from scenario name to bytes, so the `scenarios` list of paths stays unchanged and every manifest reader keeps working.
- The estimates count application payload bytes in both directions with default parameters. The http1 values are computed from the scenario messages, and `web_get` matches the 17,244 bytes of spike S4.
- Examples that the current controller compiler rejects live under `contracts/examples/tests-pending/`, because the controller store tests compile every file in `contracts/examples/tests/` with known fields only.

## Pitfalls

- `engine/fps-hosts/tests/contract_examples.rs` reads every entry of `contracts/examples/tests/` as a file, so that directory takes no subdirectory.
- The OpenAPI document refers to the schemas, so `ui/src/api/schema.d.ts` needs `npm run gen:api` after a schema change.

- Any edit to an example module tree changes its digest, which the example plan and seven compiler golden plans pin.
- A schema change needs the embedded copies in both `controller/internal/store/schema/` and `controller/internal/registry/schema/`.

## Verify the current state

```bash
make test-con
python3 tools/contracts/validate.py --tests contracts/examples/tests/*.yaml contracts/examples/tests-pending/*.yaml
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T19:04 | claude-con-09-2000 | claim | none | Claimed from status ready |
| 2026-11-03T19:30 | claude-con-09-2000 | finish | pending | All steps verified, ready for review |
