# Handoff AGT-07 Module settings, side label, and client default in the agent

| Field | Value |
|---|---|
| Issue | #216 |
| Branch | `wp/AGT-07` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-a7c3e1, lease until 2026-11-09T14:07:01Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply `ModuleRef.settings` in the agent (dns `zone_file`, relative paths resolved against and confined to the module directory), register the UDP session series through `fps_stats::SessionCounters` with the side label, and make the agent criteria count the client side unless a side is named.

## Plan

- [x] 1. Module settings reach the dns native functions, with path confinement and tests
- [x] 2. UDP worker registers through SessionCounters; snapshots hold both sides; criteria default to the client side
- [x] 3. Controller e2e run of a dns zone test in controller_e2e.sh
- [ ] 4. Agent wiki page, log entry, checks, PR, CI

## Done and verified

- Step 1: `NativeFunctions::for_modules` applies `ModuleRef.settings` (dns `zone_file`), confined to the module directory. `cargo test -p fps-agent --lib modules` and `--test standalone` pass, including a plan run whose zone differs from the shipped zone and a negative control without the setting.
- Step 2: `udp.rs` registers through `SessionCounters` with the side label, the run snapshot holds both sides, `criteria::evaluate_for` counts the client side by default. `cargo test -p fps-agent` passes, `cargo clippy -p fps-agent --all-targets -D warnings` is clean.

## Remaining

PR and CI. The controller e2e script ran only in CI (no Go modules in the sandbox).

## Next action

Open the PR and watch the controller-e2e job.

## Blockers and open questions

none

## Decisions taken in this package

- Settings the agent cannot apply (module without native functions, http1) are an error, not ignored, because CON-11 says a silent skip measures another test.
- CON-11 says nothing about setting paths, so the agent refuses `..` and any path whose canonical form lies outside the module directory, absolute paths included.
- The wire `Criterion` has no labels, so a criterion cannot name a side. `evaluate_for` takes the default side: client, or server for a process that runs no client workers.
- The run snapshot now holds both sides (merged totals), so the JSON lines of `fps-agent run` and the controller snapshots carry client and server series.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-a7c3e1 | claim | none | Claimed #216 |
| 2026-11-09 | claude-a7c3e1 | checkpoint | see branch | Steps 1 and 2 done |
| 2026-11-09 | claude-a7c3e1 | checkpoint | see branch | Steps 3 and 4 written, opening the PR |
