# Handoff RLM-24 A reused TCP client port starts a new session

| Field | Value |
|---|---|
| Issue | #578 |
| Branch | `wp/RLM-24` |
| Pull request | #0 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-9deca0, lease until 2026-11-17T00:23:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Two TCP connections that reuse a client port must be two sessions in the session table of `realism/capture.py` and in the decrypted view of `realism/decrypted.py`. A SYN without ACK on a key whose connection has seen a FIN or an RST starts a new session, a retransmitted SYN of an open connection stays in its session, and the view gives each session distinct initial sequence numbers. Every committed fixture must still pass.

## Plan

- [x] 1. Session table split, `Session.connection`, and `SessionResolver` for the readers that walk the frames again.
- [x] 2. Distinct initial sequence numbers in the decrypted view.
- [x] 3. Tests on synthetic captures, fixture comparison, README, Realism page, log entry with the Shared page notes.
- [ ] 4. Pull request, CI, review.

## Done and verified

- `python3 -m pytest -q realism/tests/test_capture.py realism/tests/test_decrypted.py`: passed (the tshark view test ran with the installed tshark).
- `ruff format --check`, `ruff check`, and `mypy --config-file realism/pyproject.toml realism`: clean.
- Session tables of the 16 committed captures before (origin/main) and after the change are identical (ids, endpoints, packets, bytes).

## Remaining

Full `pytest realism` run, push, pull request, CI.

## Next action

Run `python3 -m pytest -q realism`, then push and open the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- The split rule reads flags and not sequence numbers, so the synthetic captures that give every session the numbers 1000 and 5000 still work.
- `Session.connection` and `SessionResolver` carry the rule to `content`, `exchanges`, `structure`, and `temporal`, which looked a session up by its key.
- Results keyed by the 5-tuple (nDPI, tshark findings) still apply to every session of a key; the log entry says so.
