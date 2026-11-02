# Handoff AGT-03 Capability detection

| Field | Value |
|---|---|
| Issue | #38 |
| Branch | `wp/AGT-03` |
| Pull request | #69 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-agt03-bb7a, lease until 2026-11-02T21:43:01Z |
| Last verified commit | the head of `wp/AGT-03` at finish |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent reports cores, NUMA nodes, memory, NIC drivers, kernel version, and XDP support in the capabilities message. Detection reads `/proc` and `/sys` under an injectable system root, so tests run against fake roots for a virtio-net guest, a host without XDP, and a host with two NUMA nodes.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Create the `engine/fps-agent` crate with the lint header, the error enum, and the Rust mirror of the proto `Capabilities` message
- [x] 2. Implement detection of cores, NUMA nodes, memory, and kernel version under a system root
- [x] 3. Implement detection of ports, drivers, and AF_XDP support
- [x] 4. Add the three fake system roots and the component tests, the minimal binary, and the `test-agt` Makefile block
- [x] 5. Run fmt, clippy, deny, and tests, ingest into the wiki, and finish

## Done and verified

The crate has four library modules and one binary.

| Path | Contents |
|---|---|
| `engine/fps-agent/src/sysroot.rs` | `SystemRoot`, which resolves every path relative to an injectable root |
| `engine/fps-agent/src/proto.rs` | `Capabilities`, `PortInfo`, `ModuleRef`, and `Backend`, mirrored field for field from `agent.proto`, with the type mapping table |
| `engine/fps-agent/src/capabilities.rs` | `detect`, `HostCapabilities`, `to_capabilities`, the source table, and the driver XDP table |
| `engine/fps-agent/src/error.rs` | `DetectError` |
| `engine/fps-agent/src/main.rs` | `fps-agent capabilities [--root DIR]`, which prints one fact per line |
| `engine/fps-agent/tests/detect.rs` | Component tests against `testdata/virtio-guest`, `testdata/host-no-xdp`, and `testdata/two-numa`, plus a check on the running host |
| `engine/fps-agent/tests/contract.rs` | Reads `agent.proto` and compares field names and numbers of the three messages and the `Backend` enum |

Each check below passed on the finish commit.

| Command | Result |
|---|---|
| `cargo test -p fps-agent` (also `make test-agt`) | 17 tests pass, 7 unit, 5 contract, 5 component |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo fmt --all -- --check` | clean |
| `cargo deny check advisories bans licenses sources` | ok, the only new dependency is `thiserror` 2 (MIT or Apache-2.0) |
| `python3 tools/plan/plan.py check`, `render --check`, `lint` | ok |
| `cargo run -p fps-agent -- capabilities` on a Firecracker guest | reports virtio_net on eth0 and AF_XDP from kallsyms |

## Remaining

Two gaps remain, and neither blocks review.

1. NUMA nodes, the kernel release, and the per port NUMA node and XDP hints have no field in the `Capabilities` message of contract version 0.1.0. They live on `HostCapabilities`, and follow up #74 proposes the contract fields. `to_capabilities` needs a few lines once they exist.
2. The AGT workstream has no `.github/workflows/agt.yml` yet. The `rust` job of `ci.yml` runs `cargo test --workspace`, which covers this crate, so we left the workflow file to the package that owns `.github/`.

## Next action

A reviewer agent reviews pull request #69 against the Definition of Done.

## Blockers and open questions

none

## Decisions taken in this package

1. **Hand written mirror instead of generated types.** The package description allows it, and adding `prost` and a build script for one message would outgrow a size S package. `tests/contract.rs` reads the proto file and destructures each struct without `..`, so drift fails either compilation or the test.
2. **Facts outside the message stay on `HostCapabilities`.** We did not encode NUMA or kernel facts into `labels`, because the contract defines labels as operator assigned.
3. **Cores are online CPUs.** The proto says cores is the number of workers the agent can run, so we count `sys/devices/system/cpu/online` and fall back to `processor` lines of `proc/cpuinfo`. CPU affinity masks and cgroup quotas are not applied.
4. **Memory is `MemAvailable`.** The proto says memory available for sessions, and `MemFree` is the fallback for kernels older than 3.14.
5. **AF_XDP evidence order.** A kernel older than 4.18 is unsupported. Otherwise `boot/config-<release>` decides through `CONFIG_XDP_SOCKETS=y`, then `xsk_create` in `proc/kallsyms`, which unprivileged readers still see by name. Unknown support does not list `BACKEND_AF_XDP`, so the agent never advertises a backend it cannot prove.
6. **Ports are every non loopback interface.** The namespace harness uses veth pairs without a device, so we keep virtual interfaces and report an empty driver for them instead of dropping them.
7. **Driver XDP table is a hint.** Native XDP and zero copy come from a static table in `capabilities.rs`. virtio_net zero copy starts with kernel 6.11, after the virtio-net AF_XDP series of July 2024. Backend selection must confirm zero copy with a bind.
8. **Driver from `uevent`.** The fake roots avoid symlinks by reading `DRIVER=` from `device/uevent`. The `device/driver` link is the fallback on a real host.

## Pitfalls

1. The root `Cargo.toml` glob matches every directory under `engine/`, so every such directory needs a `Cargo.toml`. The fixtures therefore live inside the crate at `engine/fps-agent/testdata/`.
2. `Cargo.lock` is shared with ENG-01, which creates engine crates in parallel. A merge of `origin/main` may conflict there, and `cargo update -w` regenerates the workspace entries.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/tmp/fps-target   # any directory outside the tree
make test-agt                             # 17 tests pass
cargo clippy -p fps-agent --all-targets -- -D warnings
cargo run -q -p fps-agent -- capabilities --root engine/fps-agent/testdata/two-numa
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:05:01Z | claude-agt03-bb7a | claim | none | Claimed issue #38, card 6587192907 |
| 2026-11-02T20:13:01Z | claude-agt03-bb7a | checkpoint | see branch | Crate, fixtures, and tests pass |
| 2026-11-02T20:14:05Z | claude-agt03-bb7a | finish | see branch | Makefile target, wiki ingest, follow up #74, ready for review |
