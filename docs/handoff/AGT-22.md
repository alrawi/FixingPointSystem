# Handoff AGT-22 The agent selects and reports its backend

| Field | Value |
|---|---|
| Issue | #575 |
| Branch | `wp/AGT-22` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (3 of 4 plan steps) |
| Current holder | claude-f11f46, lease until 2026-11-17T04:58:31Z |
| Last verified commit | none |

## Goal

Wire `select_backend` of TRN-14 into `fps-agent`. The agent builds a `SelectionInput` from `HostCapabilities` (weakest port decides), selects the backend at boot, and reports `kernel_release`, `selected_backend`, and `PortInfo.xdp_driver_mode` (CON-32). A forced backend that cannot run stops the boot with a `SelectError`. Only `engine/fps-agent/` changes.

## Plan

- [x] 1. Claim commit and handoff file
- [x] 2. `select` module: `SelectionInput` from `HostCapabilities`, mapping to the proto types, capabilities fill
- [x] 3. `--backend` flag of `fps-agent connect`, boot failure on `SelectError`, `capabilities` output
- [ ] 4. Tests, Agent page, log entry, checks, pull request

## Done and verified

- `engine/fps-agent/src/select.rs`: `selection_input`, `weakest_driver`, `select`, `backend_of`, `xdp_mode_of`, `parse_backend`, `reason_text`, with 17 unit tests on fake `HostCapabilities` (`cargo test -p fps-agent --lib select`).
- `HostCapabilities::capabilities_for(&Selection)` fills `kernel_release`, `selected_backend`, and per port `xdp_driver_mode` (AF_XDP only). `to_capabilities()` uses the default selection.
- `fps-agent connect --backend kernel|af_xdp`: a failed selection exits 2 before the dial. `fps-agent capabilities` prints the selection.
- `engine/fps-agent/tests/select.rs` (9 tests) passes, `tests/detect.rs`, `tests/contract.rs`, `tests/exit_codes.rs` pass, `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt --all -- --check` are clean.
- `plan.py check`, `render --check`, `lint` pass.

## Remaining

The full `cargo test -p fps-agent` run, the pull request, and CI.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- `--backend` exists on `connect` only. `run` takes its backend from the plan.
- `xdp_driver_mode` is filled only when AF_XDP is selected (CON-32 contract text), per port from the driver table. The selection uses the weakest port.
- A port without a driver name is `XdpSupport::None`, so one bridge keeps AF_XDP out of the default choice.
- `assemble.rs` and `ENGINE_API` (0.2.6) are untouched, as the coordinator asked. The finding that `assemble` follows `plan.backend` and not the selection is in the log entry for TRN-12.

## Pitfalls

- The worktree guard refuses commands that mix `git` with other tools or `$(...)`. Run `git` alone.
- Tests that depend on `TRANSPORT_AVAILABLE` branch on it, so TRN-12 flipping the constant does not break them.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-22 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17 | claude-f11f46 | claim | none | claimed #575 |
| 2026-11-17 | claude-f11f46 | checkpoint | see branch | selection, flag, tests, docs written; clippy and fmt clean |
