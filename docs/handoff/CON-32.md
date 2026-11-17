# Handoff CON-32 Kernel release, selected backend, and XDP mode in the agent capabilities

| Field | Value |
|---|---|
| Issue | #576 |
| Branch | `wp/CON-32` |
| Pull request | none yet |
| Status | in progress |
| Progress | 85% (3 of 4 plan steps) |
| Current holder | claude-8eee15, lease until 2026-11-17T01:15:17Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Additive contract change approved by the vetting agent under D13 (issue #74 in part). Add `Capabilities.kernel_release` (9), `Capabilities.selected_backend` (10), and `PortInfo.xdp_driver_mode` (6) with the enum `XdpMode`, and their REST mirror. Agent protocol 0.2.4 to 0.2.5 and OpenAPI 0.12.0 to 0.13.0. Write `contracts/changes/CON-32.md` with a Consumers table, AGT-22 first.

## Plan

- [x] 1. Claim, branch, handoff file
- [x] 2. Proto fields, enum, and version 0.2.5 in the proto header, buf.yaml, and README
- [x] 3. OpenAPI mirror and version 0.13.0, wiki page, log entry, change entry
- [ ] 4. Checks, pull request, and CI

## Done and verified

- Steps 1 to 3. `python3 tools/contracts/validate.py` (0 errors), `python3 -m pytest tools/contracts` (354 passed, 1 skipped), `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The proto file compiles with `grpc_tools.protoc`. Fields 9 and 10 of `Capabilities` and 6 of `PortInfo` were free, and the file has no `reserved` range.

## Remaining

Open the pull request and wait for CI. Expected CI failures outside `contracts/` until AGT-22 and the coordinator's regeneration land: `engine/fps-agent/tests/contract.rs` (field tables of `Capabilities` and `PortInfo`, and `ENGINE_API` against the header) and the `APIVersion` check in `controller/internal/api/capabilities_test.go`. The change entry lists the edits.

## Next action

Run `python3 tools/plan/plan.py check`, push, and open the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- `XdpMode` maps TRN-11 as the vetting agent said: generic to `GENERIC`, native copy to `NATIVE`, native zero copy to `ZERO_COPY`.
- A port with queues in several modes reports the slowest mode. This rule is mine, written in the change entry for AGT-22, and is not in the proto text. It is easy to change before AGT-22 starts.
- `Agent.backend` falls back to the first entry of `backends` when `selected_backend` is `BACKEND_UNSPECIFIED`, so an agent of 0.2.4 shows what it showed before.
- The package CTL-53 named by the vetting agent does not exist in `plan/work-packages.yaml`. The change entry says the coordinator adds it.

## Pitfalls

- The sandbox guard refuses heredocs and several commands in one call when `git` appears. Use the editor tools to write files.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16 | claude-8eee15 | claim | none | Claimed #576, branch wp/CON-32 from origin/main 9a6c495 |
| 2026-11-17 | claude-8eee15 | checkpoint | none | Proto, OpenAPI, change entry, wiki page, and log entry written |
