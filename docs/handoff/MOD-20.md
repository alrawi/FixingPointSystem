# Handoff MOD-20 STARTTLS scenarios for smtp and imap

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #325 |
| Branch | `wp/MOD-20` |
| Pull request | #0 |
| Status | in progress |
| Progress | 40% (2 of 5 plan steps) |
| Current holder | claude-m20a7c, lease until 2026-11-10T06:15:05Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the STARTTLS variants to the smtp module (submission on 587) and the imap module (143), using the `tls_upgrade` action of ENG-20, with fixtures that embed the TLS keys. The realism job must decrypt the fixtures with the exported keys and Zeek must log the messages inside TLS.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the context, claim, and study the https scenarios and fixtures of MOD-04
- [x] 2. smtp scenario starttls_submission with tests
- [ ] 3. imap scenario starttls with tests
- [ ] 4. Fixtures, labels, decrypted checks, Zeek
- [ ] 5. READMEs, Modules page, log entry, PR, CI

## Done and verified

- Step 2: `cargo test -p fps-module-smtp` passes (starttls.rs, goldens, fixture.rs); `cargo clippy -p fps-module-smtp --all-targets -D warnings` clean; capture.sh recorded smtp.pcapng with 8 STARTTLS sessions on port 5587; realism encryption check 8/8 decrypted and 8/8 JA4; nDPI 4.2.0 says SMTP for the STARTTLS sessions; decrypted_check.py passes (Zeek not installed here).

## Remaining

- imap scenario, fixture, READMEs, Modules page, log entry, PR.
- Zeek on the decrypted view cannot run here (no zeek); CI is the only proof.

## Next action

Write modules/imap/scenarios/starttls_mailbox.yaml (safari-18.5 client, server-tls12), its tests, peer, fixture, and decrypted_check.py (copy modules/smtp/fixtures/decrypted_check.py).

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T04:45Z | claude-m20a7c | claim | none | claimed #325 |
| 2026-11-10T00:10Z | claude-m20a7c | checkpoint | see git log | smtp STARTTLS scenario, tests, fixture |
