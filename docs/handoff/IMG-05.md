# Handoff IMG-05 Container images

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #201 |
| Branch | `wp/IMG-05` |
| Pull request | #330 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-4643a2, lease until 2026-11-09T21:25:35Z |
| Last verified commit | defa0d8 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build controller and agent container images (Dockerfiles with non root users) and a compose file for an all in one lab. Acceptance: the compose lab runs a short test between two agent containers. The workflow `.github/workflows/containers.yml` builds the images and runs the lab on a GitHub hosted runner.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Dockerfiles for controller and agent
- [x] 2. Compose lab, init scripts, and the test
- [x] 3. Workflow containers.yml
- [x] 4. Docs (Image.md Containers section), log entry
- [x] 5. PR and green CI

## Done and verified

- Final CI of defa0d8: workflow run 42695515905 passed (`RESULT PASS the compose lab ran a short test between two agent containers: ... verdict passed; http1 client=1808 server=1805 dns client=791 server=791`), and every other workflow of the PR is green.
- Steps 1 to 4: workflow run 42694450920 (commit before the summary-line fix) passed: static checks, Hadolint, image builds, and the lab with `RESULT PASS ... http1 client=1808 server=1801 dns client=791 server=791`.
- `python3 tools/plan/plan.py check`, `render --check`, `lint`, and `make lint-img test-img` pass locally.

## Remaining

Nothing. The coordinator applies the shared page notes of the log entry (mk/img.mk, Operations, Testing).

## Next action

Review the PR.

## Blockers and open questions

none

## Decisions taken in this package

- Final stages are `scratch` (no shell, no libc), with a tree built in an alpine stage so owners and modes survive. Distroless was not used because its digest could not be resolved from the worker sandbox.
- The test runs as two runs (server run on agent2 with `--side server`, client run on agent1 with `--side client`), like Gate 1, because the compiler gives an agent only the pools of its own ports.
- Pools are /30 networks of one host whose address is the container address, so no capability is needed (no NET_ADMIN, NET_RAW, IPC_LOCK, or NET_BIND_SERVICE).

## Pitfalls

- The agent counts network and broadcast out of every IPv4 pool, so /31 fails with `0 usable addresses`.
- Docker copies the owner and mode of a mount point into a new volume, and compose creates containers in any order. Every image that mounts a volume must describe the mount point the same way.
- `COPY --from=stage /rootfs/ /` keeps owners and modes, while `COPY --chown --chmod dir /dest` leaves the destination directory at 0755.

## Verify the current state

```bash
image/containers/static-check.sh   # RESULT PASS
image/containers/run-lab.sh        # needs Docker; RESULT PASS
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T21:00Z | claude-4643a2 | claim | - | claimed #201 |
| 2026-11-09T22:12Z | claude-4643a2 | review | defa0d8 | CI green, review requested |
