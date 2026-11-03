# Handoff CTL-05 Test compiler

| Field | Value |
|---|---|
| Issue | #43 |
| Branch | `wp/CTL-05` |
| Pull request | #84 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-ctl-05-1382, lease until 2026-11-03T04:13:46Z |
| Last verified commit | pending |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We compile a test configuration and an agent inventory into one plan per agent. The compiler normalizes the mix by sessions and splits rates and host pools by core count, and it is deterministic from the seed.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Add Go types that mirror the canonical JSON mapping of the Plan message
- [x] 3. Add the compile function with mix normalization and the core based split
- [x] 4. Add golden tests and determinism tests
- [ ] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Each step was verified with the command after it.

- Steps 2 to 4. `make lint-ctl test-ctl` passes with golangci-lint v2.12.2 and reports 0 issues. `go test -cover ./internal/compiler/` reports 88.6%.
- The goldens satisfy the contract. `python3 tools/contracts/check_plans.py --modules contracts/examples/modules controller/internal/compiler/testdata/*/*.json` reports 0 errors and 0 warnings in 6 files.
- Step 5. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The Controller page has a Test compiler section, and the log entry is `docs/wiki/log/2026-11-03T0258-ingest-ctl-05-test-compiler.md`.

## Remaining

CI on #84 and the review. Known gaps follow.

- Byte based mix correction belongs to CTL-12. A `bytes` basis compiles with its weights treated as session shares.
- The plan types are hand written. They move to generated protobuf types when a package adds code generation.
- `params` carries only the overrides of the test, not the scenario defaults, and the compiler does not check params against the module parameter schema.

## Next action

Wait for CI on #84, then run `fps_swarm.py review 43 --pr 84` and remove the `wip` label.

## Blockers and open questions

None block the package. We raise four owner questions.

1. The compiler ignores the agent part of a port location and gives every agent every port with the interface after the slash, because the package splits pools across agents. Should the agent part instead pin a port to one agent?
2. A pool smaller than the worker count leaves some workers without hosts of that pool, while every worker still carries an equal rate. For example `http_smoke.yaml` has one server for two workers. Should the compiler reject this, or should rates follow host shares?
3. A seed of 0 is the proto default, so the canonical form leaves it out, and `check_plans.py` then reports that the seed is missing. Should the schema forbid seed 0, or should the checker accept an absent seed?
4. Throughput objectives need a bytes per session estimate per scenario. Which module metadata field should supply it?

## Decisions taken in this package

- Agents sort by id, and worker i of an agent pins to core i, because `Capabilities` reports a core count and no core ids.
- Pools and session rates divide over all workers in global order, and the first workers get the remainder hosts. An agent therefore owns a contiguous slice, and `ErrPoolTooSmall` reports an agent with no host of a pool.
- Concurrency objectives round to whole sessions, apportion over the mix by largest remainder, and spread leftover sessions over workers with a rotating start. `simulated_users` maps to one session per user.
- A pool without `mac_base` gets `02:xx:yy:00:00:00` from its index in the configuration, so MAC addresses stay unique across agents.
- A hold phase runs at fraction 1. A linear or step phase ramps from the current fraction to 1, or down to 0 from 1. A step phase takes one step per started second.
- The seed does not influence the split. The split is plain arithmetic, so a fixed request gives fixed plans, and the seed passes through to the plan. `Request.Seed` applies only when the configuration sets no seed.
- Mix entry ids are the scenario name, with `_2`, `_3`, and so on for repeats.
- `controller/go.mod` gains `require github.com/alrawi/FixingPointSystem/tests/golden/go v0.0.0` and `replace github.com/alrawi/FixingPointSystem/tests/golden/go => ../tests/golden/go`. `go.sum` does not change.

## Pitfalls

- golangci-lint refuses to run while another instance holds its lock in the temporary directory. Set `TMPDIR` to a private directory when agents share a machine.
- gosec G115 flags integer conversions even after range checks in some places. Each suppression names the rule and the reason.

## Verify the current state

```bash
make lint-ctl test-ctl
python3 tools/contracts/check_plans.py --modules contracts/examples/modules controller/internal/compiler/testdata/*/*.json
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44 | claude-ctl-05-1382 | claim | pending | Claimed with card 6594098855 |
| 2026-11-03T03:02 | claude-ctl-05-1382 | checkpoint | see git log | Compiler, goldens, and wiki section done; waiting for CI |
