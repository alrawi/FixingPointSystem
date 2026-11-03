## [2026-11-03] ingest | CTL-05 Test compiler

We added the package `controller/internal/compiler`, a pure function that compiles a test configuration and an agent inventory into one plan per agent, and a Test compiler section at the end of the [Controller](Controller.md) page. The plan types mirror the canonical JSON mapping of `fps.agent.v1.Plan` by hand, because the controller generates no protobuf code yet. The mix normalizes by sessions, and host pools and rates divide over the workers of all agents, so each agent gets a share that follows its core count. Golden tests cover one agent, two agents with unequal cores, a pool that does not divide evenly, and the 10 Gbps enterprise example, and the contract plan checker accepts every golden plan. The module `controller/go.mod` gained a `require` and a `replace` line for the Go golden file helper. The evidence is `make lint-ctl test-ctl` and `docs/handoff/CTL-05.md` on the branch `wp/CTL-05`.

## Shared page notes

The coordinator may apply three notes.

1. The Layout table of [Controller](Controller.md) can gain the row `internal/compiler/` with the purpose "Compiles a test configuration and an agent inventory into per agent plans". We left the table alone so that CTL-02 and CTL-05 do not edit the same lines.
2. [Testing](Testing.md) can list `controller/internal/compiler/testdata/` among the golden file locations and mention that `check_plans.py` accepts the compiler goldens.
3. [Home](Home.md) can mark CTL-05 as ready for review.
