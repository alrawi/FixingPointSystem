# Handoff TRN-14 Backend selection at boot

| Field | Value |
|---|---|
| Issue | #566 |
| Branch | `wp/TRN-14` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (2 of 3 plan steps) |
| Current holder | claude-7b14e3, lease until 2026-11-17T02:57:12Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Select the fastest working backend from detected capabilities and report the choice. The input is a type of the backend crates, because `HostCapabilities` lives in `fps-agent`. AGT-22 tests it next and wires it into the agent.

## Plan

- [x] 1. `select_backend`, its input and error types, and `TRANSPORT_AVAILABLE` in both crates
- [x] 2. Tests for a virtio-net guest with and without zero copy, forced backends, and the wiki page
- [ ] 3. Checks, pull request, and CI

## Done and verified

- `cargo test -p fps-backend-afxdp --lib --test select`: 28 unit tests and 13 selection tests passed.
- `cargo clippy -p fps-backend-afxdp -p fps-backend-kernel --all-targets -- -D warnings`: clean.

## Remaining

Pull request and CI.

## Next action

Run the checks of `AGENTS.md` and open the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- The code lives in `fps-backend-afxdp/src/select.rs`. The kernel crate gets only the constant `TRANSPORT_AVAILABLE`, so neither backend crate depends on the other.
- The default is the kernel backend. AF_XDP wins with kernel support, a transport, and native or zero copy XDP. Generic XDP wins only when forced.
- A forced backend that cannot run is a typed `SelectError`, never a fallback.
- `fps_backend_afxdp::TRANSPORT_AVAILABLE` is false until TRN-12, so every host selects the kernel backend today.

## Pitfalls

The worktree guard refuses `source` and `export` in a command, so pass the variables inline.

## Verify the current state

```bash
cargo test -p fps-backend-afxdp --lib --test select   # 28 and 13 tests pass
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17T02:27Z | claude-7b14e3 | claim | none | claimed issue #566 |
