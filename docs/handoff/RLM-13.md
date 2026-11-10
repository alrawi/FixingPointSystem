# Handoff RLM-13 Mix accuracy from the classifier as metric series

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #392 |
| Branch | `wp/RLM-13` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-d73749, lease until 2026-11-10T18:55:18Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The mix accuracy check emits mix.share_pct, mix.abs_error_pp and mix.max_abs_error_pp with source=classifier and the classifier label (catalog 0.4.0), and reads the scenarios table of the REST report beside the file report form.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Metric series in the mix entry, schema, report version 0.10.0
- [x] 2. REST report scenarios table input
- [x] 3. Tests on a fixture with a known mix, catalog 0.4.0 validation
- [ ] 4. README, Realism page, log entry, PR

## Done and verified

- Steps 1 to 3: `python3 -m pytest -q realism` gives 567 passed, 9 skipped; `ruff format --check`, `ruff check`, and `mypy` of `realism` are clean; `plan.py check`, `render --check`, and `lint` pass. A real ndpiReader 4.2.0 run on `realism/tests/fixtures/mix/` gives the literal series.

## Remaining

Step 4: pull request and CI.

## Next action

Open the pull request, wait for CI, then run `fps_swarm.py review 392 --pr N`.

## Blockers and open questions

none

## Decisions taken in this package

- Report format 0.10.0 (additive, per the schema description).
- One basis per report in `metrics_basis`, because the catalog has no basis label; null when it cannot be told.
- The unmatched row has no series. Run counter series (`source` `metrics`) are not emitted, the controller reports them.

## Pitfalls

The root `.gitignore` ignores `*.pcapng`, so the fixture capture needs `git add -f`.

## Verify the current state

```bash
python3 -m pytest -q realism/tests/test_mix.py   # 80 passed
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T18:15Z | claude-d73749 | claim | - | claimed #392 |
| 2026-11-10T18:50Z | claude-d73749 | checkpoint | - | series, REST form, tests, docs done |
