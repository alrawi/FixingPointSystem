# Handoff AGT-20 SIP with RTP in the agent

| Field | Value |
|---|---|
| Issue | #423 |
| Branch | `wp/AGT-20` |
| Pull request | #445 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-720ee5, lease until 2026-11-11T05:13:53Z |
| Last verified commit | 3cef205 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent refuses multi flow scenarios with a UDP flow and has no `sip_rtp` registry entry. Lift the refusal for datagram flows that the runtime can open (ENG-26), register `sip_rtp` with one `SipRtp` per flow side, and run `sip_rtp/call_pcmu` through the real agent in the controller e2e with a passed verdict, succeeded sessions, and RTP in the capture.

## Plan

- [x] 1. Lift the refusal in `check_flows`, route multi flow UDP scenarios to the runtime workers, add the registry entry and its tests.
- [x] 2. Add the first_party test and the controller_e2e run and capture, the Agent page, and the log entry.
- [x] 3. Checks, PR, CI.

## Done and verified

- `cargo test -p fps-agent --lib`: 81 passed, with the new registry list and per-side tests.
- `cargo test -p fps-agent --test first_party`: 5 passed, including `a_sip_call_with_a_media_flow_runs_through_the_agent`.
- `bash -n engine/fps-agent/tests/controller_e2e.sh`: syntax ok. The script itself needs the Go controller, which cannot build here.

## Remaining

none; CI green (controller-e2e success).

## Next action

none; PR #445 is green.

## Blockers and open questions

none

## Decisions taken in this package

- A scenario with several flows goes to the runtime workers whatever the L4 of its entry flow, because the agent's UDP workers run one flow per session. The `taken` port check still keys on the entry flow's L4.
- `SipRtp` is `Clone` with no shared state, so the registry stores it directly and needs no `SshSide` style wrapper.

## Pitfalls

The first obvious fix, removing only the refusal, would have sent a UDP entry flow to the agent's UDP workers, which run flows[0] alone and would never open the media flow.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent --offline   # expect all pass
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-720ee5 | claim | none | claimed #423 |
