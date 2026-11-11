# Handoff MOD-16 NTP module

| Field | Value |
|---|---|
| Issue | #183 |
| Branch | `wp/MOD-16` |
| Pull request | #447 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-b74ad8, lease until 2026-11-11T05:13:57Z |
| Last verified commit | 1345a6b |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship ntp client and server scenarios in client and server modes, with NTPv4 packet encoding with realistic poll intervals. Acceptance: tshark parses every fixture packet without errors.

## Plan

- [x] 1. Read the dns module, contracts, and the `now` builtin; design the manifest and scenarios
- [x] 2. Native crate with the NTPv4 codec and template functions, with unit tests
- [x] 3. Scenarios, interpreter tests, and golden transcripts
- [x] 4. Fixture peer, capture.sh, fixtures, expected.yaml, and the realism score
- [x] 5. README, wiki page and log entry with shared page notes, checks, PR, and CI

## Done and verified

- `cargo test -p fps-module-ntp`: 24 unit tests, 2 catalog, 5 scenario, and 2 transcript tests pass.
- `cargo clippy -p fps-module-ntp --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- `tools/contracts/validate.py --modules <dir with a link to modules/ntp> --tests --no-openapi` passes.
- `modules/ntp/fixtures/capture.sh` ran as root in the QA-01 harness here: 30 sessions, nDPI 4.2.0 30 of 30 `NTP`, tshark 4.2.2 no parser error and no malformed packet. The capture and its report are committed.
- On port 5123 nDPI 4.2.0 classifies every flow as Unknown, so the fixture uses port 123.

## Remaining

- CI is green on 1345a6b, including the realism job with Zeek 9.0.1.
- The coordinator creates `mk/mod-ntp.mk` and `.github/workflows/mod-ntp.yml` from the shared page notes of the log entry, and the AGT follow up adds the agent registry line.
- Zeek is not installed here. The realism workflow scored the fresh capture with Zeek 9.0.1 and passed.

## Next action

Review and merge PR #447, then the coordinator adds `mk/mod-ntp.mk`, `.github/workflows/mod-ntp.yml`, and the agent registry line.

## Blockers and open questions

none

## Decisions taken in this package

- Functions take `now_ms` as an argument and never read a clock. The scenarios pass `now('epoch_ms')`, so the engine clock seam pins every timestamp in tests.
- The server derives precision, root error, and the phase of its reference time from its reference identifier, so replies of one server agree and the reference timestamp never goes back. A first version drew these per reply and showed a reference time that jumped around.
- `reply` always takes the mode from the request (3 gives 4, 1 gives 2), and clamps the poll exponent to 4 through 17.
- Poll pauses are scenario variables, because a delay takes a template and the language has no arithmetic. The fixture shrinks them with `--poll-ms`.

## Pitfalls

- nDPI 4.2.0 needs port 123 for NTP.
- The worktree guard refuses `source`, `-C`, and long heredocs next to git in one command, so scripts live in the scratchpad.
- The root `.gitignore` allows `modules/*/fixtures/*.pcapng`, so no `-f` is needed.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-MOD-16 cargo test -p fps-module-ntp
sudo modules/ntp/fixtures/capture.sh /tmp/ntp-capture
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11T03:45Z | claude-b74ad8 | claim | none | Claimed #183 and created the handoff file |
| 2026-11-11T04:05Z | claude-b74ad8 | checkpoint | none | Module, tests, fixture, README, wiki, and log entry written |
| 2026-11-11T04:30Z | claude-b74ad8 | finish | 1345a6b | PR #447 opened, CI green, ready for review |
