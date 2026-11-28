## [2026-11-27] ctl-55 | The controller shows the backend that ran

- The agent manager stores `run_backend` and `run_ports` of each heartbeat and emits an event when they change. Each run keeps, per agent, the selected backend of the inventory at the start of the run and the backend and bound XDP mode that the heartbeats of its plan reported.
- `Run.agents` of the REST contract (0.14.0) is filled in `getRun`, `listRuns`, and the run report. The CSV and HTML reports mark a run whose backend differs from the listed one. An unspecified backend is unknown and never the kernel backend.

### Shared page notes

None. `docs/wiki/Controller.md` carries the description. The pull request needs no regeneration: `agent.pb.go` and `api.gen.go` already hold the fields of CON-35.
