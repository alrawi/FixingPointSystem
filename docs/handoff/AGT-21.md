# Handoff AGT-21 NTP, MQTT, LDAP, and HLS in the agent

| Field | Value |
|---|---|
| Issue | #457 |
| Branch | `wp/AGT-21` |
| Pull request | #484 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-a21c7e, lease until 2026-11-12T04:26:32Z |
| Last verified commit | d579d6e |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Batch 28 added the ntp, mqtt, ldap, and hls modules, and the agent registry has none of them. Register them with the code that their log entries give (ntp stateless, mqtt and ldap stateful like http2, hls with the segment counter and pace of one flow side), and run one scenario of each in `controller_e2e.sh` on unprivileged ports (hls/vod_segments for multi flow TCP). Acceptance: controller-e2e passes with succeeded sessions and a passed verdict for each of the four modules.

## Plan

- [x] 1. Dependencies, registry entries, and the registry list test in `engine/fps-agent/`.
- [x] 2. The four runs in `engine/fps-agent/tests/controller_e2e.sh`.
- [x] 3. Local checks and the log entry and Agent page sentence.
- [x] 4. Pull request and CI.

## Done and verified

`cargo check -p fps-agent --offline` passes. CI on d579d6e is green, and the controller-e2e job printed: ntp/sntp_query 16 succeeded sessions, mqtt/publish_qos0 16, ldap/bind_search 26, hls/vod_segments 91, each with verdict passed.

## Remaining

Nothing.

## Next action

Review and merge PR #484.

## Blockers and open questions

none

## Decisions taken in this package

- The four modules take the order ntp, mqtt, ldap, hls after sip_rtp in `REGISTRY`, and the registry test lists them in that order.
- The e2e runs use one `module_test` and `module_run` helper pair for all four modules instead of four copies of the configuration.
- The e2e copies keep the fixtures of each module, because the manifests name them, and drop `native` and `tests`.

## Pitfalls

The Go and Rust builds are slow here (2 CPUs shared), so CI is the build loop.

## Verify the current state

```bash
cargo test -p fps-agent --lib modules
bash -n engine/fps-agent/tests/controller_e2e.sh
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12 | claude-a21c7e | claim | none | claimed #457 |
| 2026-11-12 | claude-a21c7e | finish | d579d6e | CI green, ready for review |
