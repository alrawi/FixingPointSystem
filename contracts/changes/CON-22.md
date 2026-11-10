# CON-22 Realism report delivery and reservation conflicts in the REST API

Contract. Controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`. The change takes findings of CON-21 (`contracts/changes/CON-21.md`, the realism array that nothing fills) and UI-06 (`docs/wiki/log/2026-11-10T1500-ingest-ui-06-agents-and-ports-view.md`, the refused reservation). The owner approved it on 2026-11-10. Every change is additive.

Versions.

- OpenAPI moves from 0.9.0 to 0.10.0. The change adds an operation, a schema, and a property that a reader can see in the document, so it takes a minor number, as CON-12, CON-14, CON-15, CON-20, and CON-21 did. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The agent protocol (0.2.3), the scenario contract (v0.6), the test configuration (v0.3), the module manifest schema (v0.4), the metric catalog (0.4.0), and the realism report format stay as they are.

## attachRealismReport

New operation `PUT /v1/runs/{id}/realism`, scope `run`, tag `runs`.

| Question | Rule |
|---|---|
| Body | The realism report document as the runner writes it (`python3 -m realism score`), content type `application/json`, validated against `realism/schema/realism-report.schema.json`. The request body schema is a `$ref` to that file, as `createConfig` refers to `test-config.schema.json`, so the contract holds no second copy of the report format and a new check kind of the runner changes nothing here. The controller validates with an embedded byte copy, as `controller/internal/store` does for the test configuration |
| Result | `200` with `RealismAttachment`: `realism` (the `RealismResult` entries the run now holds, as `RunReport.realism` serves them) and `replaced` (true when an earlier report was replaced) |
| Repeat upload | Replaces, as a whole. A run holds at most one report, the second valid call wins, and a refused report leaves the earlier one. The same document twice gives the same state. Rejecting would force a client that scores again (other labels, a newer runner) to need a delete operation, which the contract does not have |
| Run states | Only `completed`, `failed`, and `aborted`. `pending`, `preparing`, `running`, and `stopping` answer 409, because the capture that the report scored is complete only after the run ended |
| Size | At most 4 MiB (4194304 bytes). A report of the http1 fixture is 53 KB, and the report lists at most 100 mismatches per check, so the limit leaves room for many applications and still bounds a request. A larger body answers 413 |
| Errors | All `application/problem+json` like the rest: 400 (not JSON), 404 (unknown run), 409 (run not ended), 413 (too large), 415 (content type is not `application/json`), 422 (the document breaks the report schema, or two results share `metric` and `labels`; `errors` holds the findings with JSON Pointers into the document), and the `default` problem for 401, 403, and 5xx |
| Digest | The controller does not compare `capture.sha256` with the digests in `CaptureFile`, because the runner may score a merged or scrubbed copy of the agent files. It stores the digest as given |
| Trust | A token with scope `run` may attach, so the results are what the client reported and not a measurement of the controller |

Mapping from the report to `RealismResult`. The controller creates one entry for each metric of each check, in the order of the checks and then of their metrics, so the order is the order of the document. `metric`, `labels`, and `observed` are the name, the labels, and the value of the metric. `property` is wording for an operator and not part of the pairing, which uses `metric` and `labels` (CON-21). The other fields come from the check that holds the metric.

| Metric | `target` | `passed` |
|---|---|---|
| `realism.conformance_ratio` | `>= ` and the `threshold` of the check, for example `>= 0.99` | The `passed` of the entry of `applications` whose `application` equals the label of the same name |
| `realism.ks_p_value` and `realism.ks_statistic` | `p value >= ` and the `alpha` of the comparison of the same application and property, for example `p value >= 0.05` (`request_bytes` and `response_bytes` are the `payload_size` comparisons of direction `client` and `server`) | True when the `status` of that comparison is `pass` |
| A metric of a later check kind | `check passed` | The `passed` of its check |

A KS statistic has no threshold of its own: the comparison passes by its p value, so both metrics of one comparison carry that rule and that outcome.

Finding for the coordinator: of the eight check kinds only `conformance` and `temporal` carry metrics today. `parser_errors`, `encryption`, `content`, `session_structure`, `mix_accuracy`, and `transport_behavior` have an empty `metrics` array until the metric catalog defines metrics for them, so a failing check of those kinds adds no `RealismResult`, and `RunReport.realism` does not show it. The mapping above is exactly CON-21's rule (each metric of a check, with the verdict of the check) and I did not widen it. Two ways to close the gap need an owner decision as a later CON package: a catalog metric per check kind (the report schema already says "empty until the metric catalog defines ..."), or an optional per check verdict array on `RunReport` (`check`, `passed`, and a reason). Until then the full document is the only place where those verdicts are visible, and the controller may keep it for a later read operation.

`RunReport.realism` keeps its shape. Only its text changes: it names `attachRealismReport` as the way the report arrives and no longer says that the contract has no such operation. It stays the empty array for a run without an attached report.

## reserveAgent

`reserveAgent` now documents `404` (unknown agent) and `409`. The 409 is what `controller/internal/agents/manager.go` `Reserve` and `rest.go` `writeError` already answer, so the controller needs no behavior change, only the contract.

- The operation is idempotent. The same holder (`operator`) asking again gets `200` with the agent, even when it is `offline` or `busy` at that moment, because the manager checks the holder first.
- Otherwise the controller refuses with `409` in this order: the agent is reserved by another holder (`ErrAgentReserved`, for example a run that uses it), it has no open control session (`ErrAgentOffline`), or it holds a plan that is not finished (`ErrAgentBusy`). The problem `detail` names the reason. The agent is not changed.
- The contract adds no machine readable reason (no `type` URI and no new Problem member). A client that needs the cause reads `listAgents`: `state` and the new `reserved_by` tell offline, busy, and held.

## Agent.reserved_by

New optional string, the holder of the reservation: `operator` for a reservation through `reserveAgent`, and the identifier of the run for an agent that a run uses (`runs` reserves with the run id as holder). It is present whenever a reservation exists. `state` computes offline before busy before reserved (`record.state` in the manager), so `reserved_by` can be present while `state` is `offline` or `busy`, and a UI that shows who holds an agent reads this field and not the state. It is absent without a reservation, and a controller older than 0.10.0 does not send it. `ToAPI` fills it from `Agent.ReservedBy`, which the manager already carries.

## Compatibility

A client that ignores the new operation, `reserved_by`, and the new status codes sees no difference. No property becomes required in a request or leaves a response, no type changes, and every enum and status code stays. The new request body is on a new operation. `oasdiff breaking` is expected to report nothing: it adds a path, a schema, an optional response property, and non success responses to an existing operation, and `tools/contracts/check_openapi_breaking.py` already replaces a `$ref` to a `../` JSON Schema file with an opaque object, so the reference to the report schema loads. `python3 tools/contracts/validate.py` and `python3 -m pytest -q tools/contracts` pass. I could not run oasdiff in the sandbox (the fixture test skips without it), so the `openapi-breaking` workflow is its first run.

## Consumers

| Consumer | Package | Uses |
|---|---|---|
| Controller | A controller package that follows (the report package CTL-39, or its own) | Serves `attachRealismReport`: validates with the embedded schema copy, applies the size, state, and error rules above, stores the report per run, builds the entries by the mapping, and serves them in `RunReport.realism`. Fills `Agent.reserved_by` in `agents.ToAPI`. A test with a literal report and the expected entries is its acceptance |
| Realism runner | RLM follow up | A way to deliver, for example `python3 -m realism score ... --attach URL --run ID` with a token of scope `run`, which calls `PUT /v1/runs/{id}/realism` after the run ended. No change to the report format |
| CLI | `cli/internal/fpscli` | `fps report attach RUN FILE` (optional) and the refused reservation shown as a conflict. `fps report` already prints `realism` and needs nothing else |
| Web UI | `ui/src/pages/AgentsPage.tsx`, the report views | Shows `reserved_by` next to the state and a readable message for the 409 of reserve (UI-06 finding). The report views need nothing, since `realism` has its shape already |
| Python SDK | `sdk/python` | `attach_realism_report(id, body)`, `RealismAttachment`, and `Agent.reserved_by` from the generator |

Follow up outside the catalog paths (the coordinator applies them on the branch, and this package does not edit them).

- Controller version: `controller/internal/api/capabilities.go` `APIVersion` moves from `0.9.0` to `0.10.0`, which `capabilities_test.go` (`doc.Info.Version != api.APIVersion`) checks.
- Generated code: `go generate ./internal/api` in `controller` rewrites `controller/internal/api/api.gen.go` and `controller/internal/api/routes.gen.go` (`TestGeneratedCodeIsCurrent` fails otherwise). `ServerInterface` gains `AttachRealismReport(w http.ResponseWriter, r *http.Request, id Id)`, the file gains `RealismAttachment`, `Agent.ReservedBy *string`, and `AttachRealismReportJSONBody`/`AttachRealismReportJSONRequestBody` as `map[string]interface{}` (the external schema becomes an open object, as for `createConfig`), and `RunReport.Realism` is unchanged.
- Handler stubs that the Go server needs to compile, because `ServerInterface` now requires the method:
  1. `controller/internal/api/unimplemented.go`: add `func (Unimplemented) AttachRealismReport(w http.ResponseWriter, r *http.Request, _ Id) { notImplemented(w, r, "attachRealismReport") }`, next to `ReserveAgent` and the other stubs. Without it `var _ ServerInterface = Unimplemented{}` and every package that embeds `api.Unimplemented` (`agents.API`, `report.API`, `metrics`, `runs.API`) stop compiling.
  2. `controller/internal/wiring/server.go`: `Server` embeds several handlers that each inherit the method from `api.Unimplemented`, so the selector `AttachRealismReport` is ambiguous until it sits in `pending` (the file says so, and `StopCapture` is the model). Add `func (p pending) AttachRealismReport(w http.ResponseWriter, r *http.Request, id api.Id) { api.Unimplemented{}.AttachRealismReport(w, r, id) }`. The real handler later replaces it by a field and an interface, as `ReportHandler` does for `GetRunReport`.
  3. `controller/internal/agents/rest.go` `ToAPI`: add `if ag.ReservedBy != "" { out.ReservedBy = ptr(ag.ReservedBy) }` before the `c := ag.Capabilities` line, which returns early for an agent without capabilities. Add the assertion to `rest_test.go` (the reserve test at line 147 already reads `State`). The 409 needs no code, since `writeError` maps `ErrAgentOffline`, `ErrAgentReserved`, and `ErrAgentBusy` to it. A REST test that reserves an agent that a run holds and expects 409 is worth adding, because `rest_test.go` checks the manager error and not the status.
- Schema copy for the later handler: `//go:generate cp ../../../realism/schema/realism-report.schema.json schema/realism-report.schema.json` and a `go:embed`, in the package that serves the operation.
- Python SDK: `python3 sdk/python/scripts/generate_client.py` rewrites `sdk/python/fixingpoint/_generated.py` (`API_VERSION`, the digest, `RealismAttachment`, `Agent.reserved_by`, and the method `attach_realism_report`). Check that the generator takes the body of an external reference as a plain mapping, as it does for `create_config`.
- UI: `npm run gen:api` in `ui` regenerates `ui/src/api/schema.d.ts`. The report schema reaches it as `components["schemas"]["realism-report.schema"]`, as `test-config.schema` does. If openapi-typescript cannot read the draft 2020-12 file (`if`/`then`, `const`), say so and I change the body to the opaque object that `check_openapi_breaking.py` already uses for such files. `ui/src/api/client.ts` needs no change for the report. `reserveAgent` already throws the problem, and the 409 handling is a UI package.
- `tools/contracts/test_contracts.py` pins no OpenAPI version, so nothing changes there.
- Wiki: `docs/wiki/Controller.md` (REST report section and agents) and `docs/wiki/Realism.md` (delivery) are for the owners of those pages, with the packages above.
