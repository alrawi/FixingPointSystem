# Handoff CON-35 The agent reports the backend that ran and the bound XDP mode

| Field | Value |
|---|---|
| Issue | #663 |
| Branch | `wp/CON-35` |
| Pull request | none yet |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-9c41e7, lease until 2026-11-18T22:50:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add fields that carry the backend that ran a plan and the bound XDP mode of each port, in the message that the agent sends after it starts the plan, so a benchmark never measures the wrong backend unseen. AGT-29 fills them and CTL-55 consumes them. The change is additive and backward compatible.

## Plan

- [x] 1. Read the issue, the TRN-31 decision entry, the AGT-24 entry, and the contract rules, and choose the message
- [x] 2. Proto fields, REST mirror, version bumps, and the contract tests
- [x] 3. Change entry `contracts/changes/CON-35.md`, the Contracts page, the log entry, and this file
- [ ] 4. Open the pull request with the rationale, and follow CI

## Done and verified

- Step 2 and 3: `python3 tools/contracts/validate.py` (0 errors), `python3 tools/contracts/check_plans.py` (0 errors), `python3 -m pytest -q tools/contracts` (377 passed, 1 skipped), `python3 -m pytest -q sdk/python` (30 passed). The proto file compiles with `grpc_tools.protoc`, and a comparison of the compiled descriptors of `origin/main` and the branch shows `RunPort` and `Heartbeat.run_backend` (7) and `run_ports` (8) as the only additions.
- `sdk/python/fixingpoint/_generated.py` and `ui/src/api/schema.d.ts` are regenerated mechanically (the UI generator ran from a scratch install of openapi-typescript 7.13.0).

## Remaining

- The pull request and its CI. `buf` and `oasdiff` are not installed in the sandbox, so CI checks the breaking changes.
- Consumer steps that the brief keeps out of this package, listed in `contracts/changes/CON-35.md`: Go bindings (`regen-agentpb`), `api.gen.go`, `APIVersion` 0.14.0, the `pb::Heartbeat` literal in `engine/fps-agent/src/session.rs`, and `ENGINE_API` 0.2.7 in the agent. Until they land, the Rust and Go jobs that build or compare these fail.

## Next action

Open the pull request against `main` and poll its check runs.

## Blockers and open questions

none

## Decisions taken in this package

- The fields sit on `Heartbeat`, not on `StartResult`: a heartbeat repeats on every session, so a controller that lost the stream during the run learns the values again, and it carries the other per run state. See the rationale in `contracts/changes/CON-35.md`.
- The package edits `tools/contracts/test_plans.py` and `tools/contracts/test_contracts.py`, which are outside the catalog path `contracts/`, because the acceptance names contract tests and every earlier CON package kept them there. It also regenerates `sdk/python/fixingpoint/_generated.py` and `ui/src/api/schema.d.ts`, which the version bump of the OpenAPI document requires.
- The REST mirror is `Run.agents` (schema `AgentRun`), because CTL-55 shows the values in the run report and the API, and its paths are `controller/` only.

## Pitfalls

- `Heartbeat` is built without `..Default::default()` in `engine/fps-agent/src/session.rs`, so any new field breaks the agent build. This package did not touch agent code.
- A flow style YAML mapping with a backtick and a colon in a description breaks `sdk/python/scripts/generate_client.py`. Use a block mapping.
- `npm install` in `ui/` failed in the sandbox (`edgesOut`), so the UI types were generated from a scratch directory.

## Verify the current state

```bash
python3 tools/contracts/validate.py          # contracts: 0 error(s)
python3 -m pytest -q tools/contracts         # 377 passed, 1 skipped
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-9c41e7 | claim | none | Claimed #663, branch wp/CON-35 from origin/main f136dc8 |
| 2026-11-18 | claude-9c41e7 | checkpoint | none | Proto, OpenAPI, SDK and UI types, tests, change entry, wiki page, and log entry written |
