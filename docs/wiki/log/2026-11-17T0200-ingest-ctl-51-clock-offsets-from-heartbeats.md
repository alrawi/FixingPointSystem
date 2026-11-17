## [2026-11-17] ingest | CTL-51 Clock offsets of agents from heartbeats

CTL-51 (issue #595) gives the multi agent clock check of CTL-16 a production source. `agents.Manager` keeps `agent_time` minus the receive time of each heartbeat per open session and exposes it as `Agent.ClockOffset` and `Agent.HasClockOffset`. `multiagent.HeartbeatClocks` adapts it to `ClockSource`, and fails closed with `ErrNoClockOffset` for an offline, unknown, or silent agent. `fakeagent.Options` gains `ClockSkew` and `Clock`, both neutral by default.

Decisions.

- The adapter sits in `multiagent` because that package imports `agents`.
- The estimate includes the one way heartbeat delay. It is documented on `HeartbeatClocks` and in [Controller](Controller.md).
- The offset resets at session open, so a reconnection never reuses an old value.

Findings.

- CTL-52 must pass `HeartbeatClocks{Agents: mgr}` as `Config.Clocks`.
- The Go code could not be built locally. CI is the first build.

### Shared page notes

None.
