# Handoff AGT-03 Capability detection

| Field | Value |
|---|---|
| Issue | #38 |
| Branch | `wp/AGT-03` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-agt03-bb7a, lease until 2026-11-02T21:34:21Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent reports cores, NUMA nodes, memory, NIC drivers, kernel version, and XDP support in the capabilities message. Detection reads `/proc` and `/sys` under an injectable system root, so tests run against fake roots for a virtio-net guest, a host without XDP, and a host with two NUMA nodes.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Create the `engine/fps-agent` crate with the lint header, the error enum, and the Rust mirror of the proto `Capabilities` message
- [ ] 2. Implement detection of cores, NUMA nodes, memory, and kernel version under a system root
- [ ] 3. Implement detection of ports, drivers, and AF_XDP support
- [ ] 4. Add the three fake system roots and the component tests, the minimal binary, and the `test-agt` Makefile block
- [ ] 5. Run fmt, clippy, deny, and tests, ingest into the wiki, and finish

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Create `engine/fps-agent/Cargo.toml` and `src/lib.rs` with the lint header.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/tmp/fps-target   # any directory outside the tree
cargo test -p fps-agent
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:05:01Z | claude-agt03-bb7a | claim | none | Claimed issue #38, card 6587192907 |
