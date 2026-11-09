## [2026-11-09] ingest | CTL-24 throughput calibration in the run loop

The run orchestrator in `controller/internal/runs/` now calls the CTL-21 calibration for `throughput_bps` objectives. It builds `calibrate.State` from the compiled plans and the `expected_bytes` of the catalog, samples the client side `bytes.tx`, `bytes.rx`, and `sessions.succeeded` of each mix entry every window (5 s by default, configurable), and sends `UpdateRates` with each agent's whole total per entry (compiled rates times the factor) when the factor changes. Agents with `engine_api` below 0.2.0 never get the command and are logged once. Each change of the factor is a log line and a `CalibrationStep` in `Run.Calibration`. Other objectives are untouched. An end to end test with simulated agents whose sessions carry four times the expected bytes ends within 5 percent of the objective. The [Controller](../Controller.md#throughput-calibration-in-the-run-loop) page describes the loop.

### Shared page notes

- Follow up for CTL-06 (`controller/internal/agents/`, `controller/internal/fakeagent/`): `agents.Manager` needs `UpdateRates`, and `fakeagent` needs to answer the command, or the calibration stays off on a real controller (the orchestrator logs `throughput calibration is off: the agent manager cannot send UpdateRates`). The manager method is the same as `Start`:

  ```go
  // UpdateRates sends an update rates command to agent id and waits for its result.
  func (m *Manager) UpdateRates(ctx context.Context, id string, cmd *agentpb.UpdateRatesCommand) (*agentpb.UpdateRatesResult, error) {
  	res, err := m.command(ctx, id, &agentpb.SessionResponse{Body: &agentpb.SessionResponse_UpdateRates{UpdateRates: cmd}}, false)
  	if err != nil {
  		return nil, err
  	}
  	return res.GetUpdateRates(), nil
  }
  ```

  The `fakeagent` change is a case in `apply` that accepts `SessionResponse_UpdateRates` in the running state and returns `CommandResult_UpdateRates`. The CTL-24 test then no longer needs its `rateManager` wrapper.
- [Implementation Plan](../Implementation-Plan.md) and the catalog: the metrics catalog has no mix entry label, so the orchestrator matches a mix entry to the `scenario` label of its scenario. Two mix entries of one scenario split the counters by weight. A `mix_entry` label on the scenario view metrics would make the match exact.
- `Run.Calibration` is not in the REST API. A report or the web UI that wants the factor history needs a field in the run schema of the OpenAPI document (a contract change).
- The bytes of sessions that fail or are still open count in `bytes.tx` and `bytes.rx` but not in `sessions.succeeded`, so a high failure ratio biases the bytes per session upward and the rate downward. A per entry bytes per completed session metric in the catalog would remove the bias.
