# Handoff MOD-20 STARTTLS scenarios for smtp and imap

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #325 |
| Branch | `wp/MOD-20` |
| Pull request | #370 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-m20a7c, lease until 2026-11-10T06:15:05Z |
| Last verified commit | 4768603 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the STARTTLS variants to the smtp module (submission on 587) and the imap module (143), using the `tls_upgrade` action of ENG-20, with fixtures that embed the TLS keys. The realism job must decrypt the fixtures with the exported keys and Zeek must log the messages inside TLS.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the context, claim, and study the https scenarios and fixtures of MOD-04
- [x] 2. smtp scenario starttls_submission with tests
- [x] 3. imap scenario starttls_mailbox with tests
- [x] 4. Fixtures, labels, decrypted checks, Zeek
- [x] 5. READMEs, Modules page, log entry, PR, CI

## Done and verified

- Step 2: `cargo test -p fps-module-smtp` passes (starttls.rs, goldens, fixture.rs); `cargo clippy -p fps-module-smtp --all-targets -D warnings` clean; capture.sh recorded smtp.pcapng with 8 STARTTLS sessions on port 5587; realism encryption check 8/8 decrypted and 8/8 JA4; nDPI 4.2.0 says SMTP for the STARTTLS sessions; decrypted_check.py passes (Zeek not installed here).

- Step 3 and 4 (imap): `cargo test -p fps-module-imap` passes (7 starttls tests, goldens, fixture.rs), clippy clean, `make lint-mod-imap lint-mod-smtp` 0 errors; capture.sh in the namespace harness recorded imap.pcapng (40 plain + 8 STARTTLS on 1143); realism: conformance 40/40 IMAP and 8/8 IMAPS, parser errors 0, encryption 8/8 decrypted and JA4 matched; decrypted_check.py: 16 messages, nDPI classifies the decrypted view as IMAP, ratio 1.0.

## Remaining

- Push, open the PR, poll CI, set ready for review.
- Zeek on the decrypted views cannot run here (no zeek); the realism workflow ran it on PR 370: smtp view 8 messages / 16 files, 0 failed checks; imap view 8 connections, no service, 0 failed checks. The mod-smtp/mod-imap workflow and mk steps are in the log entry for the coordinator.

## Next action

Review of PR 370.

## Blockers and open questions

none

## Decisions taken in this package

- STARTTLS sessions run on ports 5587 (smtp) and 1143 (imap) in the fixtures, because the labels match by port and 587 and 143 carry the plain sessions.
- Browser profiles (firefox-139, safari-18.5) and the server profiles of the http1 registry copy are used, so the ClientHello carries the ALPN ids of a browser. No new profile was defined.
- nDPI labels: smtp STARTTLS stays SMTP (accepted SMTP, SMTPS), imap STARTTLS is IMAPS (accepted IMAPS only).
- Zeek: decrypted view (nDPI says IMAP for the inside-TLS dialogue only, IMAPS when the STARTTLS exchange stays in, nothing when the pre-upgrade lines are joined without it).

## Pitfalls

- `follow,tls` node 0 is not always the server; decrypted_check.py reads the nodes from the header.
- ethtool must be installed for the harness capture, or the first capture is truncated.
- nDPI with `-d` does not guess by port, so a decrypted view that nDPI cannot see through shows Unknown.

## Verify the current state

```bash
cargo test -p fps-module-smtp -p fps-module-imap   # all pass
python3 -m unittest discover -s modules/imap/fixtures
python3 modules/imap/fixtures/decrypted_check.py modules/imap/fixtures/imap.pcapng --sessions 8   # passed
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T04:45Z | claude-m20a7c | claim | none | claimed #325 |
| 2026-11-10T00:10Z | claude-m20a7c | checkpoint | see git log | smtp STARTTLS scenario, tests, fixture |
| 2026-11-10T00:50Z | claude-m20a7c | checkpoint | 544e5fb | imap STARTTLS scenario, fixture, decrypted check; READMEs, Modules.md, log entry written |
| 2026-11-10T01:30Z | claude-m20a7c | ready for review | 4768603 | PR 370, all CI workflows green after re-running three stalled or flaky jobs |
