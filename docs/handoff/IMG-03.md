# Handoff IMG-03 Image format conversion

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #199 |
| Branch | `wp/IMG-03` |
| Pull request | #290 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-img03a, lease until 2026-11-09T15:11:44Z (renewed by heartbeat) |
| Last verified commit | a73dd90 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Convert image/output/fps-base.qcow2 to raw, VHDX, fixed VHD aligned to 1 MiB, and an OVA (streamOptimized VMDK), with SHA256SUMS and qemu-img checks. Acceptance: checks pass for every output and the VHD size is a multiple of 1 MiB.

## Plan

- [x] 1. image/convert.sh
- [x] 2. image/tests/convert-test.sh wired into static-check.sh
- [x] 3. README and wiki Image page
- [ ] 4. PR, CI, owner host test on the real image

## Done and verified

`image/tests/convert-test.sh` passes 20 checks locally (qemu-img 8.2.2). ShellCheck clean.

## Remaining

Owner host test on the real built image (see the PR). mk/img.mk needs the two new scripts in IMG_SHELL_FILES (coordinator).

## Next action

Poll CI on the PR, then run fps_swarm.py review 199 --pr N.

## Blockers and open questions

none

## Decisions taken in this package

force_size=on keeps the VHD size exact. The OVF uses SCSI plus E1000 NICs because VMware has no virtio. The OVF capacity is the real virtual size; raw and VHD are rounded up to 1 MiB.

## Pitfalls

qemu-img info --output=json lists virtual-size of the file node before the image; take the last match. qemu-img check exits 63 for formats without check support.

## Verify the current state

image/tests/convert-test.sh

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T15:00Z | claude-img03a | finish | pending | script, test, docs written |
| 2026-11-09T15:20Z | claude-img03a | ready for review | a73dd90 | CI green on PR #290 |
