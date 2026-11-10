# Handoff AGT-12 Port filter for captures and a real end to end capture

| Field | Value |
|---|---|
| Issue | #297 |
| Branch | `wp/AGT-12` |
| Pull request | none yet |
| Status | in progress |
| Progress | 50% (2 of 4 plan steps) |
| Current holder | claude-a12c7e, lease until 2026-11-10T06:17:28Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent accepts `StartCapture` with a port and records only the sessions that match it, for TCP and UDP. An empty port records every session. `engine/fps-agent/tests/controller_e2e.sh` then runs a capture through the REST API of the real controller against the real agent, downloads it, checks the SHA-256 and parses the pcapng, and checks the decryption secrets block of an https capture with `export_tls_keys`.

## Plan

- [x] 1. Port filter module in the agent (parse a port name or a server port list or range, match frames at assembly).
- [x] 2. Wire it into `StartCapture`, and agent tests for TCP and UDP.
- [ ] 3. `controller_e2e.sh` capture steps (dns and https).
- [ ] 4. Docs (`docs/wiki/Agent.md`, log entry), checks, PR, CI.

## Done and verified

Steps 1 and 2: `src/portfilter.rs` (6 unit tests) and `tests/run_control.rs::a_capture_with_a_port_skips_the_sessions_of_other_ports` pass with `cargo test -p fps-agent --lib` and `--test run_control`.

## Remaining

Steps 3 and 4.

## Next action

Add the capture steps to `engine/fps-agent/tests/controller_e2e.sh`.

## Blockers and open questions

none

## Decisions taken in this package

See the log entry once written.

## Pitfalls

The TCP taps live in `fps-runtime` and read `fps_capture::CaptureCell`, outside this package's paths, so the filter cannot sit at the tap.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-a12c7e | claim | none | claimed #297 |
