# Handoff HST-03 ARP and ND for user space backends

| Field | Value |
|---|---|
| Issue | #564 |
| Branch | `wp/HST-03` |
| Pull request | #591 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-9c41d7, lease until 2026-11-17T01:15:10Z |
| Last verified commit | none |

## Goal

Answer ARP and IPv6 neighbor discovery for host pools of user space backends, and assign MAC addresses per host with an allocator that avoids conflicts. DHCP is HST-04. Acceptance: the host identity check of `realism/hosts.py` passes for 20,000 hosts on one port.

## Plan

- [x] 1. MAC allocator with conflict checks (`mac.rs`)
- [x] 2. Neighbor table with reverse address lookup, ARP and ND responders on bytes (`neighbor.rs`)
- [x] 3. Port loop over `fps_backend_afxdp::FrameIo`, tested on the in memory pair
- [x] 4. 20,000 host capture scored by `realism.hosts`, wiki page, log entry, PR

## Done and verified

- `cargo test -p fps-hosts`: 60 unit tests, 9 integration tests in `tests/neighbor_port.rs`, all pass.
- `cargo clippy -p fps-hosts --all-targets -- -D warnings` and `cargo fmt -p fps-hosts -- --check` pass.
- 20,000 host captures (IPv4, IPv6, two pools) pass `realism.hosts.check` through `tests/score_hosts.py`; a two responder conflict capture fails it.
- `plan.py check`, `render --check`, `lint` pass.

## Remaining

none

## Next action

none, waiting for review

## Blockers and open questions

none

## Decisions taken in this package

- The vetting agent notes say fat-nuc is not needed. Tests use `fps_backend_afxdp::memory::frame_pair`. We do not edit `engine/fps-backend-afxdp/` (bug #588 is fixed there by another worker).

## Pitfalls

- A pool that starts at the router address: the router's request becomes an announcement with no answer. Name the router as `gateway`.
- The worktree guard refuses heredocs that share a command with cargo or git; write files with the editor tools.

- `tests/kernel_port.rs` (HST-02) snapshot compared `valid_lft` and `preferred_lft` of the runner's DHCP address, which tick down, so `routed_port_binds_and_connects_from_20000_pool_addresses` failed on CI when a second passed between snapshots. The snapshot now drops those two fields. The flake is not caused by HST-03.

## Verify the current state

```bash
cargo test -p fps-hosts
```

- Host test: none needed. Everything runs on the in memory frame pair without root. Argv for the scoring step alone: `python3 -I engine/fps-hosts/tests/score_hosts.py <capture.pcap> --hosts 20001 --macs 20001`.
- Replies for the gateway or for hosts outside the slice are never sent. ARP probes are defended.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16 | claude-9c41d7 | claim | none | claimed #564 with 4 plan steps |
| 2026-11-17 | claude-9c41d7 | implementation done | pending | code, tests, wiki, log entry |
| 2026-11-17 | claude-9c41d7 | CI green on 6432b90 (62 checks), merged origin/main | see PR | ready for review |
