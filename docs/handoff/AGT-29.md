# Handoff AGT-29 An unforced agent falls back to the kernel backend

| Field | Value |
|---|---|
| Issue | #679 |
| Branch | `wp/AGT-29` |
| Pull request | #691 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-d4a52a, lease until 2026-11-18T23:33:46Z |
| Last verified commit | a02605a |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

An agent whose backend the selection chose (not forced) runs a plan that `check_af_xdp` refuses on the kernel backend and reports the backend that ran and the bound XDP mode of each port in `Heartbeat.run_backend` and `Heartbeat.run_ports` (CON-35). A forced agent refuses with the cause, as before. A pool with both prefixes is refused by the datapath too. `check_af_xdp` refuses the mix entries on the exclusion list of TRN-36 (`engine/fps-datapath/realism/profiles.yaml`, key `excluded`).

## Plan

- [x] 1. `check_af_xdp`: both prefixes, and the exclusion list read from `profiles.yaml` at build time
- [x] 2. The fallback in `prepare` (`resolve_with_fallback`, `prepare_selected`, `Config::fall_back`)
- [x] 3. The heartbeat fields `run_backend` and `run_ports`, set after the backend starts, without `..Default::default()`
- [x] 4. Tests: unit tests per refused shape (forced and unforced), `tests/fallback.rs` on the fake host, `tests/session.rs` heartbeats
- [x] 5. Agent page, log entry, checks, pull request, CI

## Done and verified

- CI of pull request #691 at a02605a: all 81 check runs succeed. The `rust` job failed once on `fps-tcp` `interop_kernel` (`the peer did not report in time`, a crate this package does not touch) and passed on rerun.
- Checks run locally: `cargo fmt`, `cargo clippy -p fps-agent --all-targets -- -D warnings`, `cargo test -p fps-agent` (the AF_XDP veth and router tests ran as root and passed), `plan.py check`, `render --check`, `lint`.
- Steps 1 to 4: `cargo test -p fps-agent --lib backend::` (25 passed), `--test fallback` (8 passed), `--test session` (8 passed), all with `CARGO_TARGET_DIR=$PWD/target-AGT-29 CARGO_BUILD_JOBS=1 --offline`.

## Remaining

Nothing in this package. Open question for the coordinator and the vetting agent: `enterprise_mix_10g.yaml` is accepted by `check_af_xdp` and refused by `assemble` on both backends (see Pitfalls and the log entry), so its half of the acceptance cannot hold as written.

## Next action

The coordinator reviews pull request #691 and the vetting agent settles the `enterprise_mix_10g.yaml` question below before it merges.

## Blockers and open questions

The acceptance names `enterprise_mix_10g.yaml` as a plan that the datapath refuses and that an unforced agent runs on the kernel backend. The datapath serves its network, and the agent refuses the plan on every backend because `web_browse` and `web_get` both accept on tcp 8080 (`mix entries web_browse and web_get both listen on tcp 172.16.0.2:8080`). Options for the vetting agent: a CON change that gives one entry another `server_port` in the example (the datapath then serves it, so it is no example for the fallback), another example for the acceptance, or a real refusal of the datapath. Nothing here was invented to make the file fall back.

## Decisions taken in this package

- Unforced means the selection reason is not `Forced`; `fps-agent connect` sets `Config::fall_back` from it. `prepare_on` keeps its signature and refuses, so the existing tests and the standalone mode are unchanged.
- The exclusion list is the file itself: `include_str!` plus `serde_norway` (already in the lock through the module crates). A list that does not parse refuses every plan for AF_XDP.
- The heartbeat fields are set in the run driver right after `Prepared::start_backend`, so they are `UNSPECIFIED` until the backend is up, and they stay until the next `Prepare` or the release of the plan.
- `select_with_transport` takes the transport fact as an argument so a test can ask for the selection after the flip while `TRANSPORT_AVAILABLE` stays `false`.

## Pitfalls

- `enterprise_mix_10g.yaml` has the port clash above. The golden plan of the controller has the same entries.
- A DHCP pool falls back to the kernel backend and is then refused by `assemble` (neither backend leases an address before HST-04). The test says so.
- The worktree guard refuses long `python3 - <<EOF` commands that also touch git; write test sources with the editor tools.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-29 CARGO_BUILD_JOBS=1 cargo test -p fps-agent --offline --lib backend::
CARGO_TARGET_DIR=$PWD/target-AGT-29 CARGO_BUILD_JOBS=1 cargo test -p fps-agent --offline --test fallback --test session
# expected: every test passes
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T22:04Z | claude-d4a52a | claim | none | Claimed #679 with a 90 minute lease. |
| 2026-11-18T22:20Z | claude-d4a52a | checkpoint | a02605a | Code, tests, Agent page, and log entry. Pull request #691 opened. |
| 2026-11-18T22:58Z | claude-d4a52a | review | a02605a | CI green (one rerun of an unrelated fps-tcp flake). |
