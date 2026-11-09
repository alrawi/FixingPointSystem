# Handoff MOD-07 SMTP module

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #174 |
| Branch | `wp/MOD-07` |
| Pull request | none |
| Status | in progress |
| Progress | 88% (7 of 8 plan steps) |
| Current holder | claude-mod07-a1, lease until 2026-11-09T17:42:06Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship smtp client and server scenarios (EHLO, AUTH PLAIN and LOGIN, MAIL, RCPT, DATA with a MIME attachment, QUIT) with a submission variant. Zeek logs every fixture message without parser errors.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the engine, http1, dns, content, and tls modules and settle the STARTTLS question
- [x] 2. Native crate with smtp functions and unit tests
- [x] 3. Manifest and scenarios, validate.py passes
- [x] 4. Interpreter tests with an SMTP server model
- [x] 5. Golden transcripts at seed 42
- [x] 6. Fixture capture script and expected.yaml
- [x] 7. mk and workflow notes, README, wiki section, log entry
- [ ] 8. Checks, pull request, CI

## Done and verified

- Steps 1 to 5. `cargo test -p fps-module-smtp --offline` passes 32 unit tests, 2 catalog tests, 5 scenario tests (both sides over the in memory duplex, parsed with an RFC 5321 model), and 2 transcript tests. `cargo clippy -p fps-module-smtp --all-targets -- -D warnings`, `cargo fmt --check`, and `make lint-mod-smtp` pass.
- Step 6. `fixture_peer` ran all six scenarios, 8 sessions each, over kernel TCP on loopback while dumpcap recorded. tshark 4.2.2 dissects 64 messages as SMTP with IMF parts and reports no error severity expert info. The capture is committed as `fixtures/smtp.pcapng`.
- Step 7. README, wiki section, log entry, `mk/mod-smtp.mk`, `.github/workflows/mod-smtp.yml`. `plan.py check`, `render --check`, and `lint` pass.

## Remaining

- CI: the realism job runs capture.sh in the namespace harness with Zeek, and runs zeek_check.py, which nobody could run locally (no Zeek here). A wrong field name in zeek_check.py shows up only there.
- No STARTTLS scenario, see decisions.
- The committed capture is a loopback recording, to be replaced by the first harness run.

## Next action

Open the pull request, wait for CI, and fix failures of the module checks inside `modules/smtp/`.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- STARTTLS is not expressible. A flow binds its transport once at connect or accept, and the TLS module wraps a whole `SessionIo`. The submission scenarios run in the clear on port 587.
- The server accepts every credential, because a flow cannot branch on what it reads.
- Credentials come from `smtp.credential`, since the content module has no credential generator. Passwords start with `Tst-`, and users are `first.last@example.test`.
- `message` takes `sender`, `to`, and `cc` as bytes, because the validator types a nested module call as bytes and rejects it for a text parameter.
- The scenarios call `content.email` and `content.binary` through nested calls and a `set` of the text, and the manifest requires the content module.
- The fixture peer binds ports 25 and 587 before the first session, so the client never meets a closed port at the change of port.

## Pitfalls

- A `set` result goes through `Value::parse_override`, so a value that looks like a number or a boolean changes type. Passwords always contain letters and a hyphen for that reason.
- The agent sandbox guard refuses heredocs and `$VAR` paths in commands that run tools, so write scripts with the editor and run them by path.

## Verify the current state

```bash
cargo test -p fps-module-smtp --offline   # 41 tests pass
make lint-mod-smtp                         # contracts: 0 error(s)
python3 -m unittest discover -s modules/smtp/fixtures   # 10 tests pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T16:12Z | claude-mod07-a1 | claim | none | Claimed issue 174 |
| 2026-11-09T16:45Z | claude-mod07-a1 | checkpoint | see branch | Code, scenarios, tests, fixture, docs done, opening the pull request |
