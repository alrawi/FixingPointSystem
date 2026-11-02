# Handoff CON-04 Scenario contract v0.2

| Field | Value |
|---|---|
| Issue | #20 |
| Branch | `wp/CON-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-con04-d3df, lease until 2026-11-02T21:34:19Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Extend the scenario contract to v0.2. The template grammar gains module function calls with keyword arguments, such as `dns.query(name=vars.host, qtype='A')`, and the reference `last` for the bytes that the previous expect consumed. The contract states datagram rules for expect on UDP flows, and the validator checks module calls against a function list without running WASM.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Design the v0.2 grammar, `last`, and the UDP datagram rules, and bump the scenario schema
- [ ] 2. Add the template function list to the module manifest schema and the example manifests
- [ ] 3. Implement the template parser and module call checks in `tools/contracts/validate.py`
- [ ] 4. Add validator tests for the grammar, `last`, module calls, and UDP expect
- [ ] 5. Update the examples, `contracts/README.md`, and `contracts/CHANGELOG.md`
- [ ] 6. Ingest into the wiki, append the Log entry, run the checks, and finish

## Done and verified

None yet.

## Remaining

All plan steps.

## Next action

Read the CON-03 template function catalog and write the v0.2 grammar into `contracts/README.md`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest tools/contracts
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:05:07Z | claude-con04-d3df | claim | none | Fresh branch from main, card 6587192454 |
