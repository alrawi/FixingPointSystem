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
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-m20a7c, lease until 2026-11-10T06:15:05Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the STARTTLS variants to the smtp module (submission on 587) and the imap module (143), using the `tls_upgrade` action of ENG-20, with fixtures that embed the TLS keys. The realism job must decrypt the fixtures with the exported keys and Zeek must log the messages inside TLS.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read the context, claim, and study the https scenarios and fixtures of MOD-04
- [ ] 2. smtp scenario starttls_submission with tests
- [ ] 3. imap scenario starttls with tests
- [ ] 4. Fixtures, labels, decrypted checks, Zeek
- [ ] 5. READMEs, Modules page, log entry, PR, CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Study modules/http1 https scenarios and write modules/smtp/scenarios/starttls_submission.yaml.

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
