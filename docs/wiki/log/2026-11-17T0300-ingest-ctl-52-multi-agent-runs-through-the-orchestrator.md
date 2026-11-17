## [2026-11-17] ingest | CTL-52 Multi agent runs through the run orchestrator

CTL-52 (issue #596) sends every run on two or more agents through `multiagent.Coordinator` with the clock source of CTL-51. `runs.Config` gains `Clocks`, `MaxClockOffset`, and `StartLead`, and the controller wiring sets `Clocks` to `multiagent.HeartbeatClocks{Agents: mgr}`. `Create` checks the clocks before it creates the run, and a refusal answers 409. The run starts through the coordinator, stops through `Run.Stop`, and takes its series and totals from the coordinator collector.

Decisions.

- A clock refusal maps to 409, next to the agent offline, reserved, and busy conflicts. No contract change is needed, because `startRun` answers problems through `default`.
- Runs on one agent, and an orchestrator without `Config.Clocks`, keep the old path exactly, as the vetting agent asked.
- The coordinator receives the plans that `Create` compiled, through a `CompilerFunc`, so the plan ids that the orchestrator watches stay valid.
- The orchestrator keeps its per agent snapshots for the calibration and for the crash accounting of failed agents, and the coordinator collector supplies the sums. Totals are not added to those of `internal/metrics`.
- The sum is exact in unsigned 64 bit integers and becomes a float once, because `Run.Totals` is a float map. Beyond 2^53 a total therefore rounds, as it did before this package.

Findings.

- `controller/internal/multiagent/doc.go` still says that `ClockSource` has no implementation. The coordinator should fix that sentence, because the package is outside the paths of CTL-52.
- The run enters `running` when the start commands were accepted, which is up to `StartLead` before the agents begin to send traffic, so `started_at` of a multi agent run precedes the start instant by that lead.
- The Go code could not be built locally. CI is the first build.

### Shared page notes

None.
