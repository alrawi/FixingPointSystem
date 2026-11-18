# Handoff AGT-24 The agent starts the selected backend

| Field | Value |
|---|---|
| Issue | #633 |
| Branch | `wp/AGT-24` |
| Pull request | #655 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-393220, lease until 2026-11-18T12:33:38Z |
| Last verified commit | 7e2f49f |

## Goal

`fps_agent::assemble` starts the selected backend instead of `plan.backend`, and the bound mode of a real AF_XDP socket is reported in `PortInfo.xdp_driver_mode` (AGT-22 finding). A forced `--backend af_xdp` run works while `TRANSPORT_AVAILABLE` is false: `select_backend` stops refusing it with `NoTransport`. Without a forced backend the selection stays the kernel backend until TRN-31. Only `engine/fps-agent/` changes.

## Plan

- [x] 1. Claim commit and handoff file
- [x] 2. Selection: a forced AF_XDP run sets `transport_available`, and the tests that pin the refusal change
- [x] 3. `backend` module: bind the queues of the plan ports, start `fps_datapath::Datapath`, read the bound mode
- [x] 4. `assemble` and `prepare` follow the selected backend, `connect` passes the selection to the session
- [x] 5. Root test on a veth pair, Agent page, log entry, full checks, pull request, CI

## Done and verified

- `cargo test -p fps-agent --lib -- backend:: select::` 27 passed.
- `FPS_REQUIRE_AFXDP=1 cargo test -p fps-agent --test afxdp_veth -- --nocapture` passes as root in the sandbox (kernel 6.18): both ports bind native and copy mode (`XDP_MODE_NATIVE`), a 4096 byte session crosses the veth pair through the datapath, the standalone run logs the bound mode and ends with `Unimplemented`.
- `tests/select.rs` (11) and `tests/session.rs` (5) pass. `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt` are clean.

## Remaining

Nothing. PR #655 is open and CI is green at 7e2f49f (the `rust` job ran `afxdp_veth` as root with `CI=true`).

## Next action

The coordinator reviews PR #655 and applies the shared page notes of the log entry.

## Blockers and open questions

none. Finding for the coordinator (also in the log entry): `fps-runtime` opens sessions through `fps-backend-kernel` only, so no worker can run on the datapath. A forced AF_XDP run starts the datapath, reports the bound mode, and ends with `Unimplemented`. TRN-12 cannot add the seam within its paths.

## Decisions taken in this package

- The standalone mode has no selection, so the plan names the backend there (`prepare_with` passes `None`). `fps-agent connect` passes the selection (`Config::backend`), and a plan for another backend is `FAILED_PRECONDITION`.
- `assemble` takes the resolved `BackendKind`. For AF_XDP it checks the shape of the plan (`backend::check_af_xdp`) and refuses UDP flows, so a plan fails at `prepare` before a queue binds.
- The AF_XDP backend starts in `Prepared::start_backend`, not at prepare. The bound mode is read from the sockets (`BoundPort`) and written into `PortInfo` by `StartedBackend::report`.
- `start_on` stops the datapath and returns `Unimplemented` for AF_XDP until the runtime can open sessions on it.

## Pitfalls

- The worktree guard refuses heredocs and compound `git` commands. Use the editor tools and plain commands.
- The first build of `fps-agent` takes about 5 minutes with one job (wasmtime).

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-24 CARGO_BUILD_JOBS=1 cargo test -p fps-agent --lib -- backend:: select::
FPS_REQUIRE_AFXDP=1 CARGO_TARGET_DIR=$PWD/target-AGT-24 CARGO_BUILD_JOBS=1 cargo test -p fps-agent --test afxdp_veth -- --nocapture
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-393220 | claim | none | claimed #633 |
| 2026-11-18 | claude-393220 | checkpoint | see branch | selection, backend module, assembly, root test, docs written |
| 2026-11-18 | claude-393220 | ready for review | 7e2f49f | PR #655, CI green |
