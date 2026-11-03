## [2026-11-03] ingest | CTL-07 Run orchestrator

We added the run orchestrator `controller/internal/runs` and a Run orchestrator section at the end of the [Controller](Controller.md) page. The package holds the run state machine with its transition table, the orchestrator that reserves agents, compiles plans, and sends the prepare, start, and stop commands, and the REST type `runs.API` that serves `POST /v1/runs`, `GET /v1/runs/{id}`, and `POST /v1/runs/{id}:stop` and embeds `api.Unimplemented`. An agent that fails, crashes, or misses heartbeats during a run marks the run degraded, and the session failure ratio against the criteria decides whether the run completes or fails. The orchestrator reaches the store, the compiler, the scenario catalog, and the agent manager through interfaces. The evidence is `make lint-ctl test-ctl` and `docs/handoff/CTL-07.md` on the branch `wp/CTL-07`.

## Shared page notes

The coordinator may apply four notes.

1. The Layout table of [Controller](Controller.md) can gain the row `internal/runs/` with the purpose "Run state machine, orchestrator, and the run operations of /v1/runs".
2. The controller binary does not build the orchestrator yet. A wiring package can create it with the store, the agent manager, and a registry backed `runs.Catalog`, set `agents.Config.Observer` to `Orchestrator.Observe`, and mount `runs.API` next to the other operations.
3. CTL-08 and CTL-09 can read the totals and the verdict of `runs.Run` and replace `Config.Evaluator` with the full criteria evaluation.
4. [Home](Home.md) can mark CTL-07 as ready for review.
