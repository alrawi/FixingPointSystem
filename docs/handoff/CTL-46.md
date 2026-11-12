# Handoff CTL-46 One property wording for realism metrics

| Field | Value |
|---|---|
| Issue | #481 |
| Branch | `wp/CTL-46` |
| Pull request | #0 |
| Status | in progress |
| Progress | 66% (2 of 3 plan steps) |
| Current holder | claude-e834bb, lease until 2026-11-12T04:26:17Z |
| Last verified commit | none |

## Goal

`MapRealism` words the property of a KS metric as the examples of CON-23 do (`KS statistic of inter_arrival of http1`), the CON-23 example test compares property too, and the HTML report test shows the new wording.

## Plan

- [x] 1. Change `ksProperty`, the fixture expectation, the example test, and add an HTML report test.
- [x] 2. Update the Controller page and write the log entry.
- [ ] 3. Open the PR, get CI green, hand over for review.

## Done and verified

- Steps 1 and 2: `gofmt -l controller/internal/report` prints nothing. Go cannot build here (no module proxy), so the tests run in CI only.

## Remaining

CI result of the pull request.

## Next action

Open the pull request and poll its check runs.

## Blockers and open questions

none

## Decisions taken in this package

- The example test compares property only for the KS metrics and the conformance ratio, the metrics whose wording the controller now shares with the examples. The examples word the other metrics differently from `genericProperty` (for example `failures of the temporal check` against `realism.check_failures of check temporal`). Changing them is outside the scope of this package, so a follow up should decide it.
- The value of the label `property` stays unchanged (`inter_arrival`), because the examples keep the underscore.

## Pitfalls

Go modules cannot be downloaded in the sandbox, so nothing compiled locally.

## Verify the current state

```bash
gofmt -l controller/internal/report
cd controller && go test ./internal/report/...
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11 | claude-e834bb | claim and implementation | none | claimed #481 |
