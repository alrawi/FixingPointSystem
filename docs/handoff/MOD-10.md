# Handoff MOD-10 FTP module

| Field | Value |
|---|---|
| Issue | #177 |
| Branch | `wp/MOD-10` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-b7c1e4, lease until 2026-11-09T21:23:51Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship the `ftp` module with control channel scenarios and spawned passive mode data flows for LIST, RETR, and STOR. The acceptance is that the session structure check reproduces one control and one data connection per transfer.

## Plan

- [x] 1. Native crate `fps-module-ftp` with the passive mode replies and parsers, listings, names, and the catalog, with unit tests
- [x] 2. Manifest and five scenarios, validated, with the interpreter tests and golden transcripts over `ScenarioInstance`
- [x] 3. `fixture_peer`, the committed loopback capture, `expected.yaml`, `capture.sh`, `structure_check.py`, and `zeek_check.py` with their tests
- [x] 4. README, Modules page, log entry, `mk/mod-ftp.mk`, and `.github/workflows/mod-ftp.yml`
- [ ] 5. Open the pull request, wait for CI (the Zeek check runs there for the first time), fix failures inside the module paths

## Done and verified

- Steps 1 to 4. `cargo test -p fps-module-ftp` passes (20 unit, 2 catalog, 6 scenario, 2 transcript tests), `cargo clippy -p fps-module-ftp --all-targets -- -D warnings` and `cargo fmt` are clean.
- `python3 -m unittest discover -s modules/ftp/fixtures` passes 19 tests.
- `python3 modules/ftp/fixtures/structure_check.py modules/ftp/fixtures/ftp.pcapng --controls 40 --transfers 80` passes, 80 data connections for 80 transfers.
- `validate.py --modules` reports no error for the manifest and the five scenarios.
- `plan.py check`, `render --check`, and `lint` pass.

## Remaining

- CI proof of the Zeek check and the nDPI label FTP_CONTROL, which the sandbox cannot run (no zeek, no ndpiReader).
- `engine/fps-agent` dispatch, which the module does not edit.

## Next action

Open the pull request and poll its checks.

## Blockers and open questions

none

## Decisions taken in this package

- The server spawns the data flow before it announces the port, with a 1 to 3 ms delay so the listener exists. The first recording showed a refused connection per transfer when the client dialed first.
- Transfer sizes travel in `SIZE`/`213` (RETR), `ALLO` (STOR), and the variable `list_entries` (LIST), because a flow cannot read until the peer closes and cannot wait for a spawned flow.
- The server waits 10 to 40 ms before `226`, for the same reason.
- `content.binary` writes at least `size` bytes, so the server measures the file with `ftp.length` over the same seeded call before it sends it.
- nDPI labels only the control connections. The data connections are covered by `structure_check.py`.
- No active mode and no AUTH TLS (see the README).

## Pitfalls

- A `connect` that precedes the peer's `accept` is refused on kernel sockets and shows up as an extra RST session in the capture. `fixture_peer` retries it, but the retry must never fire in a committed capture.
- On loopback the passive ports must stay below the ephemeral range (the fixture uses 20000 to 20999), or a client socket can hold the port that the server wants.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 cargo test -p fps-module-ftp
python3 -m unittest discover -s modules/ftp/fixtures
python3 modules/ftp/fixtures/structure_check.py modules/ftp/fixtures/ftp.pcapng --controls 40 --transfers 80
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T20:53Z | claude-b7c1e4 | claim | none | Claimed #177 |
| 2026-11-09T21:10Z | claude-b7c1e4 | checkpoint | none | Steps 1 to 4 done, first push |
