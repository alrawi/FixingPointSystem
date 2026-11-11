# Handoff RLM-10 Host identity check

| Field | Value |
|---|---|
| Issue | #192 |
| Branch | `wp/RLM-10` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-6897c3, lease until 2026-11-11T19:41:48Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Count the distinct hosts of a capture and detect address conflicts. Acceptance: a capture with two MAC addresses for one IP address reports a conflict.

## Plan

- [x] 1. `realism/hosts.py`: host counting and conflict search over Ethernet frames, VLAN tags, and ARP
- [x] 2. `hosts` labels section (`labels.py`, `labels.schema.json`), `host_identity` report entry (`realism-report.schema.json`), `score()` and the `--host-identity` and `--host-count` flags, report format 0.13.0
- [x] 3. `realism/tests/test_hosts.py` (35 tests, no tool needed)
- [x] 4. README, `docs/wiki/Realism.md`, log entry
- [ ] 5. Checks, pull request, CI, review

## Done and verified

- Steps 1 to 3: `python3 -m pytest -q realism` gives 721 passed, 9 skipped (tools missing). `ruff format --check`, `ruff check`, and `mypy` on `realism` are clean.

## Remaining

Pull request and CI.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- A host is an IP address per VLAN that is a source, a destination, or an ARP sender. Multicast, broadcast, reserved, and unspecified addresses are not hosts. An address binds to a MAC only through the sender of a frame, because the destination MAC of a remote host is the router.
- A conflict is one IP address with two or more Ethernet source addresses in one VLAN, in IP packets or ARP senders. `gateway_macs` in the labels leaves router MACs out. One MAC with many IPs is no conflict.
- A capture without any Ethernet source address fails ("not judged"), like the other checks that judge nothing.
- The entry has `metrics: []`, like `messages`: the metric catalog 0.5.0 has no value `host_identity` for the label `check`. A CON follow up is needed for `realism.check_failures`.
- Report format 0.12.0 moves to 0.13.0 (new entry kind). The existing tests that assert the version were updated.

## Pitfalls

- The controller embeds a byte copy of the report schema, and its test fails until the coordinator refreshes it. The controller workflow does not run for `realism/**`, so this pull request is not affected.

## Verify the current state

```bash
python3 -m pytest -q realism/tests/test_hosts.py
python3 -m ruff format --check realism && python3 -m ruff check realism
python3 -m mypy --config-file realism/pyproject.toml realism
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11T18:12:00Z | claude-6897c3 | claim | | Claimed #192, branch from origin/main |
| 2026-11-11T18:20:00Z | claude-6897c3 | checkpoint | | Steps 1 to 4 done, first push |
