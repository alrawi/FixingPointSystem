# Handoff CON-03 Extension interface v0.1

| Field | Value |
|---|---|
| Issue | #19 |
| Branch | `wp/CON-03` |
| Pull request | #61 |
| Status | in progress |
| Progress | 40% (2 of 5 plan steps) |
| Current holder | claude-con03-67e1, lease until 2026-11-02T18:57:18Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Define the extension world `fps:extension@0.1.0` with the five hooks of the specification and the template functions that modules export. Propose in decision record 0010 that first party modules ship native Rust code behind the same interface, and map the WIT types to that Rust trait.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Write `contracts/extension/fps-extension.wit` and validate it with `wasm-tools component wit`
- [x] 2. Write `contracts/extension/README.md` with the hook semantics and the WIT to Rust mapping table
- [x] 3. Write decision record `docs/decisions/0010-native-extensions.md` (Proposed) and list it in the index
- [ ] 4. Add a pinned wasm-tools CI step, and update `contracts/README.md` and `contracts/CHANGELOG.md`
- [ ] 5. Ingest into the wiki, append the Log entry, and run the local checks

## Done and verified

- Step 2. `contracts/extension/README.md` written. Voice checked by a local scan for em dashes, colons, and contractions outside code and tables.
- Step 3. `docs/decisions/0010-native-extensions.md` written with status Proposed and listed in `docs/decisions/README.md`.
- The WIT file is written but not yet validated, so step 1 stays open.

## Remaining

Validate the WIT (step 1), the CI decision and the contract index and changelog (step 4, drafted), and the final checks (step 5, wiki edits drafted).

## Next action

Run `wasm-tools component wit contracts/extension/fps-extension.wit` and fix any parse error.

## Blockers and open questions

The owner must accept or reject decision record 0010 and approve the contract change.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
wasm-tools component wit contracts/extension/fps-extension.wit
python3 tools/plan/plan.py check && python3 tools/plan/plan.py render --check && python3 tools/plan/plan.py lint
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:27:18Z | claude-con03-67e1 | claim | none | Fresh branch from main, card 6583722346 |
| 2026-11-02T17:50:00Z | claude-con03-67e1 | checkpoint | see git log | WIT, README, D10, changelog, and wiki drafted; WIT validation pending on wasm-tools install |
