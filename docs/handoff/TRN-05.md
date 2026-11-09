# Handoff TRN-05 TLS client fingerprint profiles

| Field | Value |
|---|---|
| Issue | #160 |
| Branch | `wp/TRN-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-t05a1c, lease until 2026-11-09T21:23:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Define at least four browser like client profiles (Chrome, Firefox, Safari, Edge) in `modules/tls`, each with documented sources for its field orders and an expected JA4 in the profile definition. Where rustls cannot reproduce a field, document the gap and record both the JA4 that this implementation produces and the JA4 of the real browser. A CI test generates a ClientHello per profile through the real TLS module, computes JA4 with `realism/ja4.py`, and asserts it equals the expected value.

## Plan

- [ ] 1. Claim commit and handoff
- [ ] 2. Profile knobs (signature scheme order, ECH GREASE) and the four profiles with documented sources
- [ ] 3. CI test `modules/tls/tests/ja4.rs` with the stdlib JA4 helper script, and a tshark cross check
- [ ] 4. Transport wiki page, module README, log entry
- [ ] 5. Checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything after the claim.

## Next action

Extend `ClientProfile` and build the profiles.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 cargo test -p fps-module-tls
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T21:14Z | claude-t05a1c | claim | none | lease taken on #160 |
