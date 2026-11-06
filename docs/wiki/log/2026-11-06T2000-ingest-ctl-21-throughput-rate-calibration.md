## [2026-11-06] ingest | CTL-21 throughput rate calibration

The new package `controller/internal/runs/calibrate/` (decision D12 item 10) corrects the session rate of a `throughput_bps` objective from measured bytes per session. A pure `State.Update` pools the bytes and sessions of each mix entry with decay, derives the factor that meets the objective, and bounds the move with a step limit of 1.5x, a dead band with a wider reverse band, and a factor range of 1/64 to 64. A closed loop test with estimates of 0.25x to 4x the true bytes per session and noisy measurements stays within 5 percent of the objective and does not oscillate. The package is not wired in. The [Controller](../Controller.md#throughput-rate-calibration) page says what the orchestrator must pass and where to call it.

### Shared page notes

- [Implementation Plan](../Implementation-Plan.md) and the catalog: the orchestrator wiring (a calibration loop in `internal/runs/`) and the agent side are not in any package of CTL-21. The wiring needs `UpdateRates` of CON-05 and its handler in AGT-05, so the coordinator should add it to CTL-12 (`mixcontrol`) or to a follow up issue.
- The rates use the client side counters `bytes.tx` plus `bytes.rx` and `sessions.succeeded` of the scenario view. If the metrics catalog gives a per mix entry bytes per session metric later, the orchestrator can read it directly.
