# Handoff AGT-28 The agent runs IPv6 host pools on the datapath

| Field | Value |
|---|---|
| Issue | #675 |
| Branch | `wp/AGT-28` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (3 of 4 plan steps) |
| Current holder | claude-a28c5e, lease until 2026-11-18T19:08:36Z |
| Last verified commit | none |

## Goal

The agent refuses a host pool without an IPv4 prefix on AF_XDP (`engine/fps-agent/src/backend.rs`, "IPv6 is TRN-30"). Pass IPv6 pools to the datapath with their gateways for the neighbor resolution of TRN-35, so a forced AF_XDP run of the http1 and dns modules through the agent over IPv6 pools completes on a veth pair in CI.

## Plan

- [x] 1. Read the pool code in `backend.rs` and the AF_XDP tests; claim commit.
- [x] 2. Allow IPv6 pools in `backend.rs` with unit tests on fake hosts.
- [x] 3. Extend `tests/afxdp_veth.rs` with an IPv6 run of http1 and dns.
- [ ] 4. Checks, log entry, Agent page, PR, CI.

## Done and verified

- `cargo test -p fps-agent --lib backend::` passes (17 tests, no root).
- `FPS_REQUIRE_AFXDP=1 cargo test -p fps-agent --test afxdp_veth` passes as root in the sandbox, both families: http1 63 and dns 24 sessions over IPv6, none failed. The sandbox kernel has no IPv6, which this test does not need (AF_XDP takes the frames before the IP layer).
- `cargo fmt --check` and `cargo clippy -p fps-agent --all-targets -D warnings` are clean; `plan.py check`, `render --check`, `lint` pass.

## Remaining

CI result of the PR (the `rust` job and the `afxdp` job of `trn.yml`).

## Next action

Open the PR and poll its check runs.

## Blockers and open questions

none

## Decisions taken in this package

- The agent builds `Pools` from the parsed `HostPool` of each plan pool, which already holds an IPv6 prefix and gateway, so `Pools::pair_v6` and `sized_v6` (test constructors) are not called. The change is in the plan check (`backend::layout`).
- A pool runs over IPv4 when it has an IPv4 prefix (the worker addresses pick IPv4 first), else over IPv6. Two pools of one lane in different families are a plan error, because the datapath picks the lane by family.

## Pitfalls

- Both veth tests run in one process: the namespace name and the plan directory carry the family.
- The test does not pin `other_dropped`: a kernel with IPv6 sends multicast listener reports on a new veth end.

## Verify the current state

```bash
cargo test -p fps-agent --lib backend::
cargo test -p fps-agent --test afxdp_veth   # root, or sudo -n
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T18:38Z | claude-a28c5e | claim | none | lease taken on #675 |
| 2026-11-18T19:05Z | claude-a28c5e | checkpoint | none | IPv6 pools and tests done, opening the PR |
