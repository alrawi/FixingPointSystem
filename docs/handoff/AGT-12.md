# Handoff AGT-12 Port filter for captures and a real end to end capture

| Field | Value |
|---|---|
| Issue | #297 |
| Branch | `wp/AGT-12` |
| Pull request | pending |
| Status | in progress |
| Progress | 90% (4 of 4 plan steps written, CI pending) |
| Current holder | claude-a12c7e, lease until 2026-11-10T06:30:29Z |
| Last verified commit | 74fa53e |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent accepts `StartCapture` with a port and records only the sessions that match it, for TCP and UDP. An empty port records every session. `engine/fps-agent/tests/controller_e2e.sh` then runs a capture through the REST API of the real controller against the real agent, downloads it, checks the SHA-256 and parses the pcapng, and checks the decryption secrets block of an https capture with `export_tls_keys`.

## Plan

- [x] 1. Port filter module in the agent (parse a port name or a server port list or range, match frames at assembly).
- [x] 2. Wire it into `StartCapture`, and agent tests for TCP and UDP.
- [x] 3. `controller_e2e.sh` capture steps (dns and https).
- [x] 4. Docs (`docs/wiki/Agent.md`, log entry), checks, PR, CI.

## Done and verified

- `src/portfilter.rs` with 6 unit tests, and `tests/run_control.rs::a_capture_with_a_port_skips_the_sessions_of_other_ports`: `cargo test -p fps-agent` passes (all suites), `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- `controller_e2e.sh`: `bash -n` and `shellcheck` clean (one old warning), and the embedded `pcap_report` reader was run on real agent captures (with and without the TLS key log). The script itself could not run here, because the sandbox cannot build the Go controller. The PR's `controller-e2e` job is its first run.
- `plan.py check`, `render --check`, and `lint` pass.

## Remaining

The CI result of the PR, in particular the `controller-e2e` job.

## Next action

Poll the checks of the PR, and fix `controller_e2e.sh` if the job fails.

## Blockers and open questions

none

## Decisions taken in this package

- The REST `port` is a port name and the catalog says server port: the agent accepts both (a name wins).
- The filter acts when the ring is joined, because the TCP taps are in `fps-runtime`, outside this package. See the log entry for the limit (the ring and `max_bytes` still count the other sessions) and the follow-up for an ENG package.

## Pitfalls

- The worktree guard refuses `source`, heredocs, and `cd` in a command that also names git. Write helper scripts with the editor tools.
- A test that uses a port number that is also a plan port name (such as `53` in the unit tests) gets a name filter.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-a12c7e | claim | none | claimed #297 |
| 2026-11-10 | claude-a12c7e | checkpoint | 74fa53e | port filter and agent tests |
| 2026-11-10 | claude-a12c7e | checkpoint | pending | e2e script, docs, log entry |
