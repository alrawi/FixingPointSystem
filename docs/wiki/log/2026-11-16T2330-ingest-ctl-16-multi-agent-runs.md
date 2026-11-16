## [2026-11-16] ingest | CTL-16 multi agent runs

CTL-16 adds `controller/internal/multiagent/`. A `Coordinator` checks the clock offset of every agent against a limit of 10 ms before it sends any command, splits the test into one plan per agent, prepares all agents, and starts them at one instant that it converts to the clock of each agent. A `Collector` sums the counter snapshots of the agents exactly in unsigned 64 bit integers. [Controller](../Controller.md) describes the package.

What the package found:
- The agent manager ignores `Heartbeat.agent_time`, so the controller cannot measure a clock offset today. `ClockSource` is an interface with no production implementation. A follow up in the owner of `internal/agents` (CTL-06) should record the offset per agent, and wire it into `multiagent.Config.Clocks` and the controller. The contract needs no change.
- `fakeagent` sends `timestamppb.Now()` as `agent_time` and has no option to skew it, so a test of the real heartbeat path with a skewed agent needs a change in `internal/fakeagent`.
- The estimate `agent_time` minus the receive time includes the one way heartbeat delay. A tighter bound would need `HeartbeatAck` to echo `agent_time`, which is a contract change for the owner to weigh.

### Shared page notes

None.
