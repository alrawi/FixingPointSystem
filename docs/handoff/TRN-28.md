# Handoff TRN-28 OS profile options for the own stack

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #629 |
| Branch | `wp/TRN-28` |
| Pull request | #0 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-7a7cf0, lease until 2026-11-18T12:59:34Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Map the OS profile fields of the test configuration (TTL, window, MSS, window scale, and option order) onto the SYN and SYN-ACK of the own stack, as the kernel backend does through socket options today. The acceptance: the SYN and SYN-ACK of the own stack match the option order and values of every OS profile in Transport.md.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the own stack handshake code and the kernel profile table
- [x] 2. Profile type and the mapping onto the SYN and SYN-ACK
- [x] 3. Tests that compare the SYN and SYN-ACK bytes of each profile with the table
- [ ] 4. Wiki ingest (log entry, Transport page), checks, pull request

## Done and verified

- Steps 1 to 3: `cargo test -p fps-tcp-own` passes (177 unit, 25 in `tests/own.rs`, 3 in `tests/profile_table.rs`), `cargo clippy -p fps-tcp-own --all-targets -- -D warnings` and `cargo fmt -p fps-tcp-own -- --check` are clean.
- `profile.rs` has `OsProfile` (windows11, linux6, macos14), `wire.rs` has `Style` (TTL and SYN option order) and `encode_styled`, `host.rs` has `Host::with_profile`, `OwnConfig` has `client_os_profile` and `server_os_profile`.

## Remaining

Step 4: log entry, Transport page, PR and CI. Optional: an example that writes a pcapng of the handshakes, so the capture checks of the kernel backend can score the own stack.

## Next action

Write the log entry `docs/wiki/log/2026-11-18T1300-ingest-trn-28-os-profiles-own-stack.md` and the Transport page section, run the checks of AGENTS.md, open the PR.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- Profiles are `&'static` data in `engine/fps-tcp-own/src/profile.rs`, not YAML, so the crate takes no parser dependency. `tests/profile_table.rs` reads the table of `docs/wiki/Transport.md` and compares every value, so the two cannot drift.
- A profile is per side (`client_os_profile`, `server_os_profile`), because a test configuration names a profile per host pool.
- A profile wins over the `window_scale`, `timestamps`, and `sack` fields of `OwnConfig`.
- The MSS of a profile only lowers to the link (`min(profile.mss, mtu - 40)`), as `TCP_MAXSEG` does.
- A SYN-ACK uses the order of the profile and leaves out what the peer did not offer, with the NOPs that belong to the left out option. Linux 6 keeps the writer the stack already had (it pads `nop nop sok` as the kernel does).

## Pitfalls

Worktree guard: no heredocs or `source` in a command that names git; use the Edit tool and inline `VAR=value command`.

## Verify the current state

```bash
cargo test -p fps-tcp-own      # 177 + 25 + 3 tests pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T11:30Z | claude-7a7cf0 | claim | | Claimed #629 with 4 steps |
| 2026-11-18T11:55Z | claude-7a7cf0 | checkpoint | | Profile type, wire style, host mapping, tests (steps 1 to 3) |
