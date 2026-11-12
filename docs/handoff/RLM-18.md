# Handoff RLM-18 TLS on non standard ports in the decrypted check

| Field | Value |
|---|---|
| Issue | #495 |
| Branch | `wp/RLM-18` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-71aa28, lease until 2026-11-12T17:55:25Z |
| Last verified commit | none |

## Goal

`realism/decrypted.py` lists ClientHellos with plain tshark, which finds TLS on the ports 8443 and 8444 only by its heuristic, so a ClientHello split after its first bytes is missed (MOD-23). `read_sessions` must decode the server ports of the labels as TLS. The http2 wrapper in `modules/http2/` stays, because MOD-24 removes it.

## Plan

- [ ] 1. `Tshark.run` and `read_sessions` pass `-d tcp.port==P,tls` for the server ports of the labels.
- [ ] 2. Tests: a split ClientHello capture on a non standard port lists every session, and the labels give the ports.
- [ ] 3. Runner README and Realism page, log entry, checks, pull request, CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Implement step 1 in `realism/decrypted.py`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 -m pytest realism/tests/test_decrypted.py -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12 | claude-71aa28 | claim | none | claimed #495 |
