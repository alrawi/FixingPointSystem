# Handoff AGT-01 Standalone agent mode

| Field | Value |
|---|---|
| Issue | #36 |
| Branch | `wp/AGT-01` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-agt-01-0bc5, lease until 2026-11-03T16:39:49Z |
| Last verified commit | none |

## Goal

Implement `fps-agent run --plan plan.json --modules DIR`. The command reads a plan in its canonical JSON mapping, loads modules from a directory with digest checks, runs the client and server workers without a controller, prints one counter snapshot per second as JSON lines, and exits with distinct codes for pass, fail, and error.

## Plan

- [ ] 1. Read the plan JSON mapping into typed structs
- [ ] 2. Load modules from a directory with tree digest checks
- [ ] 3. Build a RuntimeConfig and register the native module functions
- [ ] 4. Run workers, print snapshots, and map the verdict to exit codes
- [ ] 5. Namespace harness test, mk/agt.mk, and .github/workflows/agt.yml
- [ ] 6. Wiki page, log entry, and finish

## Done and verified

None yet.

## Remaining

Everything.

## Next action

Read contracts/README.md and the example plan, then write the plan reader.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T15:10Z | claude-agt-01-0bc5 | claim | none | Fresh branch from origin/main |
