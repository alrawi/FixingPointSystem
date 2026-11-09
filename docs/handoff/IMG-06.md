# Handoff IMG-06 Mutual TLS in the image

| Field | Value |
|---|---|
| Issue | #277 |
| Branch | `wp/IMG-06` |
| Pull request | #281 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-img06a, lease until 2026-11-09T14:08:18Z |
| Last verified commit | 5f9c69d |

## Goal

Remove the `insecure_plain_grpc` stopgap from the image: the all-in-one role mints an enrollment token through the admin socket at first boot, an agent role takes a token and CA from cloud-init, the agent dials https:// with the AGT-08 flags, and port 8445 is open and served.

## Plan

- [x] 1. Image files: `fps-enroll-token`, `fps-agent-start`, `fps-role` keys, units, nft, controller.yaml
- [x] 2. Tests: offline tests, static checks, VM checks, boot test
- [x] 3. Docs: README, Image.md, seed example, log entry; PR and CI

## Done and verified

`make lint-img test-img` pass, ShellCheck is clean on the new scripts, `plan.py check`, `render --check`, and `lint` pass.

## Remaining

CI result. The owner host boot test (all-in-one, controller, agent).

## Next action

Poll the CI checks of the PR and run `fps_swarm.py review 277 --pr <PR>` when green.

## Blockers and open questions

none

## Decisions taken in this package

- The token reaches the agent role through a file `/etc/fps/enroll.token` (mode 0600), not through `role.env`, which the VM keeps world readable. `fps-role` refuses a token key.
- The agent role trusts `/etc/fps/controller-ca.pem` or `FPS_CA_SHA256`.
- The registration check wants the agent state `online`, because minting a token also lists the agent.

## Pitfalls

The key pattern of the `fps-role` parser did not accept digits, so `FPS_CA_SHA256` failed. `mk/img.mk` lists scripts by name for ShellCheck.

## Verify the current state

```bash
make lint-img test-img
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-img06a | claim | none | claimed #277 |
