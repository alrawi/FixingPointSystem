# Handoff ENG-05 Vectored writes for fill bodies

| Field | Value |
|---|---|
| Issue | #25 |
| Branch | `wp/ENG-05` |
| Pull request | #115 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-eng-05-a650, lease until 2026-11-03T16:19:09Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Write `fill()` bodies as vectored writes from the shared filler buffer, so large bodies are not copied per session, which is Gate 0 carry over item 1. Output bytes stay identical for the same seed. A bulk benchmark records a baseline in `bench/baselines/`, and the 1 MiB body workload stays within 25% of the hand written baseline on the same host.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Render parts that point into the filler, with tests that the bytes and draws match `render_into`
- [x] 2. Vectored send path in `fps-interp`, with component tests
- [x] 3. Criterion bulk benchmark, adapter to fps-bench/v1, and the recorded baseline
- [x] 4. Engine page section, log entry, handoff, and pull request

## Done and verified

- Step 1. `Template::render_parts`, `Part`, and `Filler::as_bytes` in `fps-flow`, with three unit tests in `engine/fps-flow/src/template/tests.rs`. `cargo test -p fps-flow --lib template` passes.
- Step 2. `send` in `engine/fps-interp/src/session.rs` writes filler parts with vectored writes, and `VECTORED_FILL_MIN` is public. `cargo test -p fps-interp` passes, with six new tests in `tests/vectored.rs`, one of them a property test.
- Step 3. `engine/fps-interp/benches/bulk.rs` with criterion 0.5 and a paired mode. `FPS_BENCH_OUT=<file> cargo bench -p fps-interp --bench bulk` recorded `bench/baselines/fps-interp-bulk-container.json`, and `bench/compare.py` of a second run against it with `--require-same-host` passes.
- Step 4. Engine page section, log entry `docs/wiki/log/2026-11-03T1450-ingest-eng-05-vectored-fill-writes.md`, and this file. `make lint test`, `cargo deny check`, and `plan.py lint` pass.

Acceptance. The 1 MiB body workload stays within 25% of the hand written baseline on the same host. The paired ratio of interpreter to hand written session rate was 1.09 to 1.17 over seven runs of 300 rounds on the 2 vCPU host, and 1.172 in the baseline, against 0.773 for the copying send path measured the same way.

## Remaining

- A `fill()` inside a module call argument is still copied. The log entry records the follow up.
- Carry over item 3, the bulk run on a 4 vCPU KVM guest with virtio-net, is not part of this package.

## Next action

Review the pull request #115.

## Blockers and open questions

none. The coordinator should accept or move the additive `fps-flow` change, which lies outside the catalog paths, see the log entry.

## Decisions taken in this package

- The parts API lives in `fps-flow`, because template segments are private there. It only adds items, so no caller changes.
- `render_parts` takes a threshold, and `send` copies bodies below 16 KiB, because a copy that small costs less than an extra I/O slice and keeps one write.
- Only top level `fill()` becomes parts. A module function needs the bytes of its arguments.
- The send path batches at most 16 slices per call, far below `IOV_MAX`, on a stack array, so it allocates nothing.
- The benchmark runs each worker on its own current thread runtime, because a `Session` borrows `dyn TemplateFn` and is not `Send`, which matches how the engine owns sessions per worker.
- The acceptance figure comes from a paired measurement that rotates the variants each round, because other agents loaded the shared host and moved absolute rates by up to a factor of two between criterion runs. The bench writes fps-bench/v1 itself, so no adapter was added to `bench/`.

## Pitfalls

- Criterion runs variants one after another, so on a loaded host its per variant figures drift apart. Use the paired mode for ratios.
- Absolute rates on the shared host are not comparable between runs, so the baseline gives them a 60% tolerance.

## Verify the current state

```bash
cargo test -p fps-flow --lib template      # 26 tests pass
cargo test -p fps-interp --test vectored   # 6 tests pass
FPS_BENCH_OUT=/tmp/r.json cargo bench -p fps-interp --bench bulk   # prints a ratio above 0.75
python3 bench/compare.py /tmp/r.json bench/baselines/fps-interp-bulk-container.json
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T14:24Z | claude-eng-05-a650 | claim | none | Claimed with card 6606329427 |
| 2026-11-03T14:55Z | claude-eng-05-a650 | finish | see PR #115 | Ready for review, make lint test passes |
