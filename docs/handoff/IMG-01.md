# Handoff IMG-01 qcow2 base image

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #197 |
| Branch | `wp/IMG-01` |
| Pull request | #224 |
| Status | waiting for host test |
| Progress | 95% (host test rerun pending) |
| Current holder | claude-c81634, lease until 2026-11-07T18:51:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build an Ubuntu 24.04 LTS qcow2 with Packer that contains the controller, the agent, cloud-init NoCloud defaults, and an isolated management interface, with firewall rules that block routing between management and test interfaces. Acceptance: the image boots under QEMU and serves /v1/capabilities on the management interface.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Packer template, files, scripts, nftables rules, units
- [x] 2. build.sh, boot-test.sh, README, tests, mk/img.mk
- [x] 3. Wiki page Image, log entry, PR
- [ ] 4. Host build and boot test by the coordinator, then CI green

## Done and verified

`shellcheck -x` on every script: clean. `nft -c -f image/files/etc/fps/isolation.nft`: ok. `make lint-img test-img`: pass. `plan.py check/render --check/lint`: ok. Not verified here (no KVM, Packer, ip, or Go modules): `packer validate`, the build, the boot test, the namespace test.

## Remaining

The owner host test failed before the build on the glibc check. That check is gone: the agent now builds for x86_64-unknown-linux-musl and the controller with CGO off, and build.sh verifies both with `file`. The musl build itself was NOT run locally (static.rust-lang.org is blocked here, so the rust-std musl component cannot be installed). All agent dependencies are pure Rust, so it is expected to build.

The host build and boot test (`image/build.sh`, `image/boot-test.sh`). Packer HCL was written without `packer validate`.

## Next action

Run `image/build.sh` then `image/boot-test.sh` on the KVM host and fix any failure inside `image/`.

## Blockers and open questions

none. Waiting for host test.

## Decisions taken in this package

The controller binds `${FPS_MGMT_ADDR}` from DHCP of `mgmt0` (a unit orders it after `fps-mgmt-addr`). nftables also drops service ports on other interfaces, because Linux uses the weak host model. mgmt0 is chosen by lowest PCI address or FPS_MGMT_MAC. Default user is `fps-admin`, not `fps`, which is the service user.

## Pitfalls

The sandbox blocks packer and go.dev downloads, so the Go toolchain checksum comes from the go.dev index at build time and is not hard coded. Both binaries are static (agent for musl, controller with CGO off), so any x86_64 Linux host builds. The musl target needs rustup.

## Verify the current state

```bash
make lint-img test-img   # all PASS
image/build.sh           # PASS image built
image/boot-test.sh       # RESULT: PASS
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T22:00Z | claude-0dabfe | claim, implement, open PR | see branch | Waiting for host test |
| 2026-11-07T17:40Z | claude-c81634 | takeover, fix | see branch | Static musl agent, static controller, glibc check removed, README rewritten |
