# Handoff MOD-18 MQTT module

| Field | Value |
|---|---|
| Issue | #185 |
| Branch | `wp/MOD-18` |
| Pull request | #448 |
| Status | in progress |
| Progress | 90% (5 of 6 plan steps) |
| Current holder | claude-4d2e91, lease until 2026-11-11T05:13:59Z |
| Last verified commit | 42b2249 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship the `mqtt` module under `modules/mqtt/` with scenarios for connect, subscribe, publish at QoS 0 and 1 (and 2), and keepalive. The deliverable is MQTT 3.1.1 and 5.0 encoding functions and a broker scenario. The acceptance is that nDPI classifies at least 99% of the fixture sessions as MQTT.

## Plan

- [x] 1. Crate `fps-module-mqtt`: variable length integers, packet encoders and readers for 3.1.1 and 5.0, unit tests
- [x] 2. Template functions, catalog, and the manifest
- [x] 3. Scenarios and in memory interpreter tests with golden transcripts
- [x] 4. `fixture_peer`, `capture.sh`, `expected.yaml`, the committed capture scored with nDPI and tshark
- [x] 5. README, component page, log entry with shared page notes
- [ ] 6. Checks, pull request, CI

## Done and verified

- Steps 1 to 5: crate (20 unit tests), 23 functions and the manifest (validator 0 errors), five scenarios with in memory and golden transcript tests, `fixture_peer`, `capture.sh`, and the capture `mqtt.pcapng` (40 sessions, nDPI 4.2.0 40 of 40 MQTT, tshark 4.2.2 no errors), README, Modules page, log entry.
- `cargo fmt --check`, `clippy -D warnings`, and `cargo test -p fps-module-mqtt` pass. `plan.py check`, `render --check`, `lint`, and `build_log.py --check` pass.

## Remaining

Step 6: open the pull request, wait for CI, fix failures inside the paths of the package, and request review.

## Next action

Push, open the pull request against main, and watch CI.

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
