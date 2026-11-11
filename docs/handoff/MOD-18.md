# Handoff MOD-18 MQTT module

| Field | Value |
|---|---|
| Issue | #185 |
| Branch | `wp/MOD-18` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-4d2e91, lease until 2026-11-11T05:13:59Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship the `mqtt` module under `modules/mqtt/` with scenarios for connect, subscribe, publish at QoS 0 and 1 (and 2), and keepalive. The deliverable is MQTT 3.1.1 and 5.0 encoding functions and a broker scenario. The acceptance is that nDPI classifies at least 99% of the fixture sessions as MQTT.

## Plan

- [ ] 1. Crate `fps-module-mqtt`: variable length integers, packet encoders and readers for 3.1.1 and 5.0, unit tests
- [ ] 2. Template functions, catalog, and the manifest
- [ ] 3. Scenarios and in memory interpreter tests with golden transcripts
- [ ] 4. `fixture_peer`, `capture.sh`, `expected.yaml`, the committed capture scored with nDPI and tshark
- [ ] 5. README, component page, log entry with shared page notes
- [ ] 6. Checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `modules/mqtt/native/src/wire.rs` with the variable length integer and the packet encoders.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-module-mqtt
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11T03:50:00Z | claude-4d2e91 | claim | none | Claimed issue 185 |
