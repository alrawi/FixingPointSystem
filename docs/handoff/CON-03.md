# Handoff CON-03 Extension interface v0.1

| Field | Value |
|---|---|
| Issue | #19 |
| Branch | `wp/CON-03` |
| Pull request | #61 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-con03-67e1, lease until 2026-11-02T18:57:18Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Define the extension world `fps:extension@0.1.0` with the five hooks of the specification and the template functions that modules export. Propose in decision record 0010 that first party modules ship native Rust code behind the same interface, and map the WIT types to that Rust trait.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Write `contracts/extension/fps-extension.wit` and validate it with `wasm-tools component wit`
- [x] 2. Write `contracts/extension/README.md` with the hook semantics and the WIT to Rust mapping table
- [x] 3. Write decision record `docs/decisions/0010-native-extensions.md` (Proposed) and list it in the index
- [x] 4. Decide on a wasm-tools CI step, and update `contracts/README.md` and `contracts/CHANGELOG.md`
- [x] 5. Ingest into the wiki, append the Log entry, and run the local checks

## Done and verified

- Step 2. `contracts/extension/README.md` written. Voice checked by a local scan for em dashes, colons, and contractions outside code and tables.
- Step 3. `docs/decisions/0010-native-extensions.md` written with status Proposed and listed in `docs/decisions/README.md`.
- Step 1. `wasm-tools component wit contracts/extension/fps-extension.wit` exits 0 with wasm-tools 1.263.0 (installed with `cargo install wasm-tools --locked`). `wasm-tools component embed --dummy` plus `component new` and `validate` also pass, so the world builds into a valid component.
- Step 4. `contracts/README.md` lists the WIT and its validation command, and `contracts/CHANGELOG.md` records extension interface 0.1.0. The local install worked, so we added no CI step, and the owner decides whether CI should check the WIT.
- Step 5. Wiki Home, Implementation Plan, Coding Standards, and Log updated. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass.

## Remaining

Owner approval of the contract and a verdict on decision record 0010. The manifest schema still requires `extension.wasm`, so a native extension needs a later schema change.

## Next action

Wait for the owner to review PR #61 and decide on decision record 0010.

## Blockers and open questions

The owner must accept or reject decision record 0010 and approve the contract change.

## Decisions taken in this package

- Per session state is a WIT `resource session` returned by `on-open`, so state maps to `Box<dyn ExtensionSession>` and the engine never tracks extension state by id.
- Template functions are dispatched through one `render(function, args)` hook with a typed catalog from `template.functions`, because a generic world cannot export module specific signatures and the engine must type check calls before the run.
- Engine calls carry no session argument. The engine binds them to the running hook, so an extension cannot act on another session.
- No clock import. Extensions see time only through timers on the injected engine clock, which keeps replays deterministic.
- `on-data` returns `pass`, `consume`, or `replace`, so the common case copies nothing back.
- `render` may not call `send` or `set-timer`, which keeps rendering free of side effects on the wire.

## Pitfalls

`cargo install wasm-tools --locked` takes about five minutes. Start it in the background at claim time.

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
| 2026-11-02T17:31:27Z | claude-con03-67e1 | note | none | The previous row's time should read 17:31:27Z, the server time of that checkpoint |
| 2026-11-02T17:32:32Z | claude-con03-67e1 | finish | see git log | WIT validated, local checks pass, ready for review |
