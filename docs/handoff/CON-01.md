# Handoff CON-01 Agent control protocol v0.1

| Field | Value |
|---|---|
| Issue | #17 |
| Branch | `wp/CON-01` |
| Pull request | #65 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-con01-222f, lease until 2026-11-02T19:10:00Z |
| Last verified commit | see the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Define the gRPC service between the controller and the agents, and the compiled plan messages that the controller sends to each agent. The plan also has a canonical JSON mapping, so the standalone agent mode can read a plan file without a controller. `buf lint` passes, and an example plan in `contracts/examples/plans/` validates against the JSON mapping.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Write `contracts/proto/fps/agent/v1/agent.proto` with the service and the plan messages.
- [x] 2. Add `contracts/proto/buf.yaml` with lint and breaking rules and compile the proto locally.
- [x] 3. Write the example plan in `contracts/examples/plans/` in the canonical JSON mapping.
- [x] 4. Add a test that parses the example plan with the protobuf package, plus semantic checks.
- [x] 5. Add the CI job with `bufbuild/buf-action` and the plan test, and update the contracts README and CHANGELOG.
- [x] 6. Ingest into the wiki, append the Log entry, and finish the pull request.

## Done and verified

1. `buf lint contracts/proto` and `buf format --diff --exit-code contracts/proto` pass with buf 1.74.0.
2. `buf breaking contracts/proto --against ".git#ref=<sha>,subdir=contracts/proto"` passes against the branch itself.
3. `python3 tools/contracts/check_plans.py` reports 0 errors for `contracts/examples/plans/http_dns_smoke.json`.
4. `python3 -m pytest tools/contracts` passes 62 tests with the proto packages installed, and the plan tests skip cleanly without them.
5. `python3 tools/contracts/validate.py`, `tools/plan/plan.py check`, `render --check`, and `lint` pass.

## Remaining

Owner approval. The open questions below may change the contract before merge.

## Next action

Wait for the owner review of pull request #65 and apply any requested changes.

## Blockers and open questions

The package carries `needs:owner`. The questions for the owner are listed in the pull request description.

## Decisions taken in this package

1. The agent serves `AgentService` and the controller is the only client, because the OpenAPI `registerAgent` operation takes the gRPC address of the agent. AGT-02 mentions a client loop that registers with a controller, which this design does not need.
2. A missed heartbeat timeout aborts the run on the agent, so no traffic runs without a controller.
3. Module digests cover the module tree and not an archive, because the archive format is not defined yet. The example plan can therefore carry real digests of the example modules.
4. Scenario sources travel as YAML text with a digest over the exact bytes.
5. Counters and histograms are cumulative from the run start, and histograms use the HdrHistogram V2 encoding, so snapshots from several agents merge and a lost snapshot loses no data.
6. Worker rates are steady state targets, and each load phase carries explicit start and end fractions, so the agent does not interpret phase names.
7. Fields named `final` and `interface` became `final_snapshot`, `is_final`, and `interface_name`, because they are keywords in Rust or Go.
8. The plan checker lives in new files under `tools/contracts/` and does not edit `validate.py`, because CON-02 changes the validator in parallel. It imports `validate.py` read only.
9. The canonical form check compares the parsed file with the protobuf serialization as JSON values, so integral numbers may appear as `4096` or `4096.0`.

## Pitfalls

1. buf, protoc, and the protobuf packages are not installed on the sandbox image. `npm install @bufbuild/buf@1.74.0` and `pip install -r tools/contracts/requirements-proto.txt` in a virtual environment work.
2. Any change to a file under `contracts/examples/modules/` changes the module digests in the example plan. Run `python3 tools/contracts/check_plans.py --print-digests` and update the plan.
3. The CI breaking check skips while `main` has no `contracts/proto/buf.yaml`, so it starts to compare only after this package merges.

## Verify the current state

```bash
python3 -m pip install -r tools/contracts/requirements-proto.txt
python3 tools/contracts/check_plans.py          # plans: 0 error(s)
python3 -m pytest -q tools/contracts            # all pass
buf lint contracts/proto                        # no output
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:30:24Z | claude-con01-222f | claim | 7f5b453 | Card 6583736116, no earlier cards |
| 2026-11-02T17:36:00Z | claude-con01-222f | checkpoint | 5af24c4 | Proto, buf.yaml, example plan, checker, and tests |
| 2026-11-02T17:40:00Z | claude-con01-222f | finish | this commit | CI job, README, CHANGELOG, wiki ingest |
