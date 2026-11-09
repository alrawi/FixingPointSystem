# Handoff MOD-08 IMAP module

| Field | Value |
|---|---|
| Issue | #175 |
| Branch | `wp/MOD-08` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (5 of 6 plan steps) |
| Current holder | claude-a11910, lease until 2026-11-09T20:01:41Z |
| Last verified commit | ad0f22c |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship the `imap` module under `modules/imap/`: client and server scenarios for CAPABILITY, LOGIN, SELECT and EXAMINE, FETCH of headers and full messages with literals from the content module, SEARCH, STORE, and LOGOUT, with correct tag sequencing and continuation responses. Plain IMAP on port 143. nDPI must classify at least 99% of the fixture sessions as IMAP.

## Plan

- [x] 1. Native crate `fps-module-imap` with the template functions and unit tests
- [x] 2. Manifest, scenarios, catalog test
- [x] 3. Scenario tests and golden transcripts
- [x] 4. Fixture peer, capture.sh, expected.yaml, committed capture
- [x] 5. mk file, workflow, README, Modules page, log entry
- [ ] 6. Checks, pull request, CI

## Done and verified

- `cargo test -p fps-module-imap` passes (43 unit tests, 2 catalog, 6 scenario, 2 transcript).
- `cargo clippy -p fps-module-imap --all-targets -- -D warnings`, `cargo fmt --check`, and shellcheck are clean.
- `make lint-mod-imap` (validate.py), `plan.py check`, `plan.py render --check`, and `plan.py lint` pass.
- tshark 4.2.2 reads `fixtures/imap.pcapng` (40 sessions) with no malformed packet and no warning.
- A Python port of the nDPI 4.2 `mail_imap.c` loop classifies 40 of 40 sessions as IMAP at the fifth payload packet. The sandbox has no ndpiReader, so the realism job in CI gives the real ratio.

## Remaining

Open the pull request, wait for CI, and fix failures inside the module paths.

## Next action

Open the pull request against main and poll the check runs.

## Blockers and open questions

none

## Decisions taken in this package

- Modelled on `modules/smtp` (MOD-07). The crate copies the MIME and name code of smtp, because a module crate must not depend on another module crate.
- The client counts tags with `imap.next` and the server reads them from `last` with `imap.tagged`, because a flow has no counters and cannot branch.
- The client reads each long response up to `secs).\r\n`, the timing suffix of Dovecot, because an `expect` pattern cannot name a tag.
- No `AUTHENTICATE` scenario: nDPI 4.2 sets IMAPS for any flow with an `AUTHENTICATE` command. `imap.auth_plain` stays in the catalog for a later scenario.
- `login_append` skips CAPABILITY, so nDPI sees the `LOGIN` that names the user as the second payload packet and detects at the fifth.
- No Zeek log check, because we could not confirm that Zeek logs IMAP messages.
- The agent dispatch line is left to AGT-13.

## Pitfalls

- The nDPI dissector gives up at the eighth payload packet, and a `LOGIN` must name the user, so keep `LOGIN` early in any new scenario.
- The sandbox worktree guard refuses `source`, `$PWD`, and long chained commands. Run plain commands.

## Verify the current state

```bash
cargo test -p fps-module-imap
make lint-mod-imap
python3 -m realism modules --format names
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T18:13Z | claude-a11910 | claim | none | Claimed #175 and created the branch |
| 2026-11-09T18:25Z | claude-a11910 | checkpoint | 22c23a8 | Crate, manifest, scenarios, and tests |
| 2026-11-09T18:34Z | claude-a11910 | checkpoint | ad0f22c | Fixture peer, capture, makefile, workflow |
| 2026-11-09T18:45Z | claude-a11910 | checkpoint | pending | README, Modules page, log entry |
