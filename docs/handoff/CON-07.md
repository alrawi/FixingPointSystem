# Handoff CON-07 API contract fields for failure reasons and cleanup

| Field | Value |
|---|---|
| Issue | #132 |
| Branch | `wp/CON-07` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-con-07-4105, lease until 2026-11-03T20:34:22Z |
| Last verified commit | see the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 19 and 20 to `contracts/openapi/`. The API says why a run failed and why a criterion passed, failed, or is missing, and the contract needs no generator workaround.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Add `Run.reason`, the named schemas `RunVerdict` and `CriterionResult` with `reason`, `type: string` on every enum, and `MetricSample.values` with format double.
- [ ] 2. Document `GET /metrics` outside `/v1` with an optional bearer token.
- [ ] 3. Regenerate `controller/internal/api` and keep the controller build and tests green.
- [ ] 4. Add `contracts/changes/CON-07.md`, the Contracts page section, and the log entry.
- [ ] 5. Run the checks, push, wait for CI, and finish the pull request.

## Done and verified

Nothing yet.

## Remaining

Every plan step.

## Next action

Step 1.

## Blockers and open questions

The package carries `needs:owner`, so the owner approves the pull request.
