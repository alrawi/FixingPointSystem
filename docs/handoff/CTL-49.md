# Handoff CTL-49 A spawned flow counts with the session that opened it

| Field | Value |
|---|---|
| Issue | #527 |
| Branch | `wp/CTL-49` |
| Pull request | none |
| Status | blocked |
| Progress | 20% (1 of 5 plan steps) |
| Current holder | claude-c49e7a, blocked |
| Last verified commit | none |

## Goal

The run counts the FTP data connection and the RTP media stream as sessions of their own, so one ftp or sip_rtp scenario session reads as two, the bytes per session halve, and the calibration doubles their shares. Find where the split happens, make sessions and bytes per scenario session include the spawned flows (run metrics, calibration, run report), and remove FPS_GATE2_BASIS from qa-gate2.yml once the bytes basis passes.

## Plan

- [x] 1. Find where a spawned flow becomes a session of its own
- [ ] 2. Fix it inside controller/internal/runs/ and controller/internal/report/ (not possible, see Blockers)
- [ ] 3. Tests, wiki entry, shared page notes
- [ ] 4. Remove FPS_GATE2_BASIS from qa-gate2.yml, open PR
- [ ] 5. CI green, review

## Done and verified

Step 1, by reading the code. The split is in the engine, outside the paths of CTL-49 (`controller/internal/runs/`, `controller/internal/report/`).

- `engine/fps-runtime/src/instance.rs`, `start side` (about line 446): `w.metrics_of(index).started(...)` runs for every flow side of an instance, the spawned flows included.
- `engine/fps-runtime/src/instance.rs`, `finish` (about lines 528 to 570): `worker.metrics_of(log.flow).ended_with(...)` records `sessions.established`, `sessions.succeeded` or `sessions.failed`, the bytes, and the duration for every `SideRecord`, the spawned flows included.
- `engine/fps-runtime/src/worker.rs`, `Worker::new` (lines 137 to 157) and `metrics_of` (line 210): the flows of one module share one `SessionMetrics`, labelled `application`, `scenario`, `side`. The FTP data flow and the RTP media flow therefore add to the same `sessions.succeeded` series as the entry flow, and nothing in the labels tells them apart.
- `docs/wiki/Engine.md` line 467 (Series row) documents this on purpose: "Every flow side counts as a session ... `mixed_session` of the ftp module counts four sessions per instance". `contracts/README.md` line 22 says "Each flow runs one session", and `contracts/metrics` describes `sessions.succeeded` as "Sessions whose flow programs finished every action on both sides".
- The controller reads these series as they are: `controller/internal/runs/calibration.go` `clientTotals` sums `sessions.succeeded` and `bytes.tx/rx` of the client side series per scenario. It cannot tell a spawned flow from an entry flow, so the only controller side fix would divide by a guessed flow count, which is the workaround the brief forbids.

## Remaining

An engine change (below), then CTL-49 can verify the controller side and remove `FPS_GATE2_BASIS`.

## Next action

The coordinator decides who changes the engine counters. After that change merges, resume CTL-49 with `fps_swarm.py takeover 527`, run qa-gate2 on the bytes basis without the variable, and fix the controller or report only if a gap remains (the run report reads the same series).

## Blockers and open questions

Blocked on an engine change in `engine/fps-runtime/`, and probably on an owner decision, because it changes documented semantics (Engine.md Series row, contracts/README.md line 22, the `sessions.*` descriptions in `contracts/metrics`).

Change needed in `engine/fps-runtime/src/instance.rs` (and `worker.rs`):
- In the instance, call `started`, and in `finish` call `established`, `succeeded`/`failed` and `ended`, only for the entry flow of the instance (`flow == worker.flow.flow`). For every other flow side that is spawned, add only the bytes (`SessionCounters::add_bytes`) to the series of the entry flow, so the bytes of one scenario session include its data connection or media stream.
- The outcome needs no new record: a failed spawned flow already fails its parent and the whole instance (`Engine.md`, Spawned failure), so the entry side's outcome covers it.
- Open design question: a flow that is not spawned but ordered with `after` (a later stage of the scenario) is also a flow side today. The owner or the ENG owner decides whether it counts as a session of its own. The `mix.yaml` scenarios `ftp/retr_epsv` and `sip_rtp/call_pcmu` use spawned flows only.
- Tests that assert the old counts and need updating with the change: `engine/fps-runtime/tests/instances.rs` (lines 288, 417, 470, 609, 738), `engine/fps-runtime/tests/datagrams.rs` (line 196, "eight succeeded sessions"), and the capture and agent tests that count sessions of multi flow scenarios.
- Wiki: the Series row of `docs/wiki/Engine.md` (line 467), the `contracts/README.md` sentence "Each flow runs one session", and the `sessions.*` descriptions in `contracts/metrics` (a CON package or owner approved change).

## Decisions taken in this package

none

## Pitfalls

Go modules cannot be downloaded here, so Go code cannot build locally; CI is the build loop. No build directory was created.

## Verify the current state

```bash
sed -n 440,450p engine/fps-runtime/src/instance.rs   # started() for every flow side
sed -n 546,570p engine/fps-runtime/src/instance.rs   # ended_with() for every record
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T19:31Z | claude-c49e7a | claim | c28ea54 | Claimed #527 |
| 2026-11-12T19:50Z | claude-c49e7a | block | | Split found in engine/fps-runtime, outside the paths; blocked as instructed |
