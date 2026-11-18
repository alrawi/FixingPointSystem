# Handoff AGT-24 The agent starts the selected backend

| Field | Value |
|---|---|
| Issue | #633 |
| Branch | `wp/AGT-24` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-393220, lease until 2026-11-18T12:13:07Z |
| Last verified commit | none |

## Goal

`fps_agent::assemble` starts the selected backend instead of `plan.backend`, and the bound mode of a real AF_XDP socket is reported in `PortInfo.xdp_driver_mode` (AGT-22 finding). A forced `--backend af_xdp` run works while `TRANSPORT_AVAILABLE` is false: `select_backend` stops refusing it with `NoTransport`. Without a forced backend the selection stays the kernel backend until TRN-31. Only `engine/fps-agent/` changes.

## Plan

- [ ] 1. Claim commit and handoff file
- [ ] 2. Selection: a forced AF_XDP run sets `transport_available`, and the tests that pin the refusal change
- [ ] 3. `backend` module: bind the queues of the plan ports, start `fps_datapath::Datapath`, read the bound mode
- [ ] 4. `assemble` and `prepare` follow the selected backend, `connect` passes the selection to the session
- [ ] 5. Root test on a veth pair in the namespace harness, Agent page, log entry, checks, pull request

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Step 2.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-24 CARGO_BUILD_JOBS=1 cargo test -p fps-agent --lib select
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-393220 | claim | none | claimed #633 |
