# Handoff CTL-18 Compiler port pinning, host share rates, seeds, and module settings

| Field | Value |
|---|---|
| Issue | #135 |
| Branch | `wp/CTL-18` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (4 of 4 plan steps, waiting for CI) |
| Current holder | claude-dd3be7, lease until 2026-11-06T16:50:37Z |
| Last verified commit | none |

## Goal

Apply decision D12 items 7, 8, 9, and 22 in the test compiler `controller/internal/compiler/`. Pinned port locations compile to one agent, rates follow host shares, a configuration without a seed gets a random nonzero seed, and module settings are validated against the manifest settings schema and carried in the plan.

## Plan

- [x] 1. Port pinning and per agent ports and pool slices.
- [x] 2. Rates that follow host shares.
- [x] 3. Random nonzero seed and module settings.
- [x] 4. Golden plans, tests, and the Controller wiki page.

## Done and verified

- Steps 1 to 4 are implemented. The Go module proxy does not resolve in the sandbox, so `make test-ctl` cannot run here. We checked the package three ways instead. A throwaway harness outside the repository compiled the package with a JSON config reader and a stub settings validator. It reproduced the four existing golden files byte for byte and wrote the three new ones. `go vet` over the real package and its tests passed against stub yaml and jsonschema modules. `gofmt -l` is clean.
- `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass.

## Remaining

CI results of `make lint-ctl test-ctl`. The settings validation against the real jsonschema library and the new unit tests have not run locally.

## Next action

Open the pull request, poll CI, and fix failures inside `controller/internal/compiler/`.

## Blockers and open questions

none

## Decisions taken in this package

- A mix entry needs hosts of its client pool only. The agent falls back to the whole server slice for a worker without server hosts, so server pools do not gate rates. Rates split equally over the eligible workers, which keeps every existing golden file unchanged.
- Existing test configs `two_agents.yaml` and `uneven_pool.yaml` used `agent-a/eth1` while two agents ran them. Pinning would change their meaning, so they now use bare interfaces and keep their goldens.
- Without a seed anywhere, `Compile` draws one nonzero seed from `Request.Rand` (default `crypto/rand`), because the store dry run compiles without a seed. `Request.Seed` stays as the caller supplied fallback.
- Module settings travel in a new `PlanModule.Settings` field. `fps.agent.v1.ModuleRef` lacks it, which is a contract gap for the coordinator.
- When a port runs on fewer than all agents, mix entries name their default pools and each plan keeps only the entries its agent can start or serve.

## Pitfalls

The Go module proxy does not resolve in the worker sandbox, so the package cannot build locally. CI is the build and test loop.

## Verify the current state

```bash
make lint-ctl test-ctl
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T15:20Z | claude-dd3be7 | claim | 7cdcf49 | Claimed #135 |
| 2026-11-06T15:50Z | claude-dd3be7 | checkpoint | see branch | Implementation, goldens, wiki, log entry |
