# Handoff AGT-22 The agent selects and reports its backend

| Field | Value |
|---|---|
| Issue | #575 |
| Branch | `wp/AGT-22` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-f11f46, lease until 2026-11-17T04:58:31Z |
| Last verified commit | none |

## Goal

Wire `select_backend` of TRN-14 into `fps-agent`. The agent builds a `SelectionInput` from `HostCapabilities` (weakest port decides), selects the backend at boot, and reports `kernel_release`, `selected_backend`, and `PortInfo.xdp_driver_mode` (CON-32). A forced backend that cannot run stops the boot with a `SelectError`. Only `engine/fps-agent/` changes.

## Plan

- [ ] 1. Claim commit and handoff file
- [ ] 2. `select` module: `SelectionInput` from `HostCapabilities`, mapping to the proto types, capabilities fill
- [ ] 3. `--backend` flag of `fps-agent connect`, boot failure on `SelectError`, `capabilities` output
- [ ] 4. Tests, Agent page, log entry, checks, pull request

## Done and verified

Nothing yet.

## Remaining

Everything after the claim.

## Next action

Write `engine/fps-agent/src/select.rs`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-22 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17 | claude-f11f46 | claim | none | claimed #575 |
