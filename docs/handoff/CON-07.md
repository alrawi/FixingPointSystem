# Handoff CON-07 API contract fields for failure reasons and cleanup

| Field | Value |
|---|---|
| Issue | #132 |
| Branch | `wp/CON-07` |
| Pull request | #146 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-con-07-4105, lease until 2026-11-03T20:34:22Z |
| Last verified commit | see the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 19 and 20 to `contracts/openapi/`. The API says why a run failed and why a criterion passed, failed, or is missing, and the contract needs no generator workaround.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Add `Run.reason`, the named schemas `RunVerdict` and `CriterionResult` with `reason`, `type: string` on every enum, and `MetricSample.values` with format double.
- [x] 2. Document `GET /metrics` outside `/v1` with an optional bearer token.
- [x] 3. Regenerate `controller/internal/api` and keep the controller build and tests green.
- [x] 4. Add `contracts/changes/CON-07.md`, the Contracts page section, and the log entry.
- [ ] 5. Run the checks, push, wait for CI, and finish the pull request.

## Done and verified

1. `python3 tools/contracts/validate.py` reports 0 errors, `python3 -m pytest -q tools/contracts` passes 153 tests, and `python3 tools/contracts/check_plans.py` reports 0 errors.
2. `openapi-spec-validator` accepts `contracts/openapi/fps-metrics.yaml`.
3. `go generate` in `controller/internal/api` regenerated `api.gen.go`, and `routes.gen.go` is unchanged. `runs.ToAPI` builds `api.RunVerdict` and `api.CriterionResult`, and `api.APIVersion` is 0.3.0. `go test ./...` in `controller/` passes. `TestEndToEnd` failed once in a full parallel run with the fake agent still in PREPARED, and passed in three reruns on this branch and two on `main`, so we treat it as a timing flake that this change does not touch.
4. `npm run gen:api` regenerated `ui/src/api/schema.d.ts`, and `make lint-ui test-ui` passes.
5. `plan.py check`, `render --check`, and `lint` pass.
6. The proto files are unchanged, so `buf lint` has nothing new to check.

## Remaining

Pull request CI and owner approval.

## Next action

Open the pull request, wait for CI, and run `fps_swarm.py review`.

## Blockers and open questions

The package carries `needs:owner`, so the owner approves the pull request.
