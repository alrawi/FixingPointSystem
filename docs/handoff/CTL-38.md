# Handoff CTL-38 Distributions in the compiler and scenario counts in the run report

| Field | Value |
|---|---|
| Issue | #376 |
| Branch | `wp/CTL-38` |
| Pull request | #396 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-c38a7e, lease until 2026-11-10T15:52:57Z |
| Last verified commit | 1a2e89f |

## Goal

A test configuration with the CON-19 arrival or size fields passes the store's schema and now also the compiler's strict decoder, and the declarations reach the report as realism targets. The registry types the scenario builtins `int_bytes` and `now`. The run report gets a scenarios table with sessions and bytes per scenario, which the RLM-07 mix check reads. The REST API shape stays as it is (CON-21 and CTL-39 own it).

## Plan

- [x] 1. Compiler: `Arrival`, `SizeDist`, `Size` on `Profile` and `MixConfig`, `Config.Declarations()`, a golden plan and tests.
- [x] 2. Registry: `coreResult` types `int_bytes` (bytes) and `now` (by its literal format), with a test.
- [x] 3. Report: `Report.Scenarios`, `Report.Declared`, `FileJSON`, CSV `scenario` rows, `DeclarationSource`, tests, golden `labeled.csv`.
- [x] 4. Wiki page, log entry, CI green, review.

## Done and verified

`gofmt -l` is clean for the three packages. CI on 1a2e89f is green: controller test, controller-e2e, contracts, rust, scripts, licenses, proto, ops, standalone-netns, the compose lab (after a re-run of one proxy.golang.org stream error), and gate. The first push failed two of my own tests (`TestDeclarations` loaded the configuration twice and compared pointers, and `TestFileJSONCarriesDeclarations` compared indented JSON with compact text), fixed in 1a2e89f. The hand derived goldens passed.

## Remaining

Review and merge. CTL-39 adds `Report.Scenarios` to the REST report after CON-21.

## Next action

Review PR 396.

## Blockers and open questions

none

## Decisions taken in this package

- The declarations are not in the plan. `fps.agent.v1.Plan` has no field for them and CON-19 says that they add no command, so `Config.Declarations()` carries them to the report.
- An entry's `arrival` is its own and never inherits `profile.arrival`, because the schema says `profile.arrival` describes the gaps of the whole run. `size` inherits per direction, as CON-19 says.
- The scenarios table counts the client side of `sessions.attempted`, `bytes.tx`, and `bytes.rx`, as the report totals and the scenario view do, which is what the RLM-07 check reads from the scenario view.
- The REST JSON stays `api.RunReport`. The table is in `Report.Scenarios`, `Report.FileJSON()`, and the CSV rows of kind `scenario`.
- No mix series are emitted. If a later package emits run metrics mix series, they carry no `source` label (catalog 0.4.0).
- `UseObjectives` adopts a `DeclarationSource`, so `internal/wiring` (outside the paths) needs no change.

## Pitfalls

- The worktree guard refuses heredocs in a command, so write files with the editor tools.

## Verify the current state

```bash
gofmt -l controller/internal/compiler controller/internal/registry controller/internal/report
# expected: no output
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T14:23:00Z | claude-c38a7e | claim | 6b0781c | Claimed issue 376 with 4 plan steps |
| 2026-11-10T14:30:00Z | claude-c38a7e | checkpoint | none | Steps 1 to 3 written, not yet built |
| 2026-11-10T14:34:00Z | claude-c38a7e | checkpoint | 1a2e89f | PR 396 open, test fixes pushed |
| 2026-11-10T14:43:00Z | claude-c38a7e | finish | 1a2e89f | CI green, ready for review |
