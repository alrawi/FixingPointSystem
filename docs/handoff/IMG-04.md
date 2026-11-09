# Handoff IMG-04 Image boot test in CI

| Field | Value |
|---|---|
| Issue | #200 |
| Branch | `wp/IMG-04` |
| Pull request | not yet opened |
| Status | in progress |
| Progress | 5% (0 of 5 plan steps) |
| Current holder | claude-fcbcc3, lease until 2026-11-09T19:41:57Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A workflow `.github/workflows/image.yml` that builds the controller and the static agent, builds the image with Packer and QEMU on a GitHub hosted runner (KVM when `/dev/kvm` exists, TCG otherwise), boots the all-in-one role, runs the boot checks and a short HTTP and DNS run through the controller REST API with the image's own agent over mutual TLS, uploads the logs and reports, and writes a job summary.

## Plan

- [ ] 1. Claim commit and handoff.
- [ ] 2. `image/tests/vm-run.sh` (the short run inside the VM) and the hooks in `image/boot-test.sh` (artifact directory, optional run).
- [ ] 3. `image/ci-summary.sh` and `.github/workflows/image.yml`.
- [ ] 4. `docs/wiki/Image.md`, `image/README.md`, log entry.
- [ ] 5. Open the PR, run the workflow, fix failures, report run id, duration, KVM or TCG.

## Done and verified

Nothing yet.

## Remaining

Everything after step 1.

## Next action

Write `image/tests/vm-run.sh`.

## Blockers and open questions

none

## Decisions taken in this package

- The short run uses the controller on the VM and the agent `local` on loopback pools (`127.x.0.0/24`), the shape of `engine/fps-agent/tests/controller_e2e.sh`. The Gate 1 suite needs the namespace harness with two agents and realism tools, which the VM does not have, so the run is a reduced variant with the same mix entries (`http1/single_fetch`, `dns/lookup_a`).

## Pitfalls

- Go modules cannot be downloaded in the authoring sandbox, so only the PR's CI builds and tests this.

## Verify the current state

```bash
gh run list --workflow image.yml --branch wp/IMG-04
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-fcbcc3 | claim | none | Claimed #200 |
