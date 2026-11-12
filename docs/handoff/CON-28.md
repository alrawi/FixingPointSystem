# Handoff CON-28 Packet gap and payload size series in the metric catalog

| Field | Value |
|---|---|
| Issue | #496 |
| Branch | `wp/CON-28` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-4d21ba, lease until 2026-11-12T15:15:55Z |
| Last verified commit | none |

## Goal

The property label of `realism.ks_statistic` and `realism.ks_p_value` gets values for packet gaps (catalog 0.8.0), and the realism runner emits those series, so packet gaps reach `RunReport.realism`. The owner approved the contract change on 2026-11-12 (comment on #496). The coordinator authorized the consumer follow ups outside `contracts/` in this PR.

## Plan

- [x] 1. Contract: catalog 0.8.0, examples, `contracts/changes/CON-28.md`, changelog
- [x] 2. Consumers: controller catalog copy, version pins, `catalog.rs`
- [x] 3. Realism runner: series, schema, report format, tests, controller schema copy, `schema.d.ts`
- [x] 4. Docs: Realism page, README, log entry
- [ ] 5. Checks, PR, CI, review

## Done and verified

Steps 1 to 4: `validate.py` 0 errors, `pytest realism tools/contracts` 1064 passed, ruff and mypy clean, `plan.py check`, `render --check`, `lint` ok, `gofmt -l` clean, `ui` `gen-api.mjs` regenerated. Go and Rust could not be built here (no module proxy), so CI is the loop for `controller` and `engine/fps-stats`.

## Remaining

Open the pull request, wait for CI, review step.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

`payload_size` is not added (request_bytes, response_bytes, both already name it). Direction folded into the value: `packet_gap` (both), `request_gap`, `response_gap`. Paced comparisons carry no KS series (their statistic and p value are not KS values). The controller `comparisonProperty` in `realism.go` maps the new values, which the coordinator list did not name but the CON-23 rule needs.

## Pitfalls

The sandbox refuses `.`/`source` and heredocs in a command that runs git. `ui/node_modules` is ignored, `npm ci --ignore-scripts` in `ui` then `node scripts/gen-api.mjs` regenerates `schema.d.ts`.

## Verify the current state

```bash
python3 tools/contracts/validate.py
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T14:46Z | claude-4d21ba | claim | none | claimed #496 |
| 2026-11-12T15:05Z | claude-4d21ba | checkpoint | 0d9b3db | contract, consumers, runner, docs done locally |
