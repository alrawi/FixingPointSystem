# Handoff AGT-01 Standalone agent mode

| Field | Value |
|---|---|
| Issue | #36 |
| Branch | `wp/AGT-01` |
| Pull request | #121 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-agt-01-0bc5, lease until 2026-11-03T16:58:37Z |
| Last verified commit | none |

## Goal

Implement `fps-agent run --plan plan.json --modules DIR`. The command reads a plan in its canonical JSON mapping, loads modules from a directory with digest checks, runs the client and server workers without a controller, prints one counter snapshot per second as JSON lines, and exits with distinct codes for pass, fail, and error.

## Plan

- [x] 1. Read the plan JSON mapping into typed structs
- [x] 2. Load modules from a directory with tree digest checks
- [x] 3. Build a RuntimeConfig and register the native module functions
- [x] 4. Run workers, print snapshots, and map the verdict to exit codes
- [x] 5. Namespace harness test, mk/agt.mk, and .github/workflows/agt.yml
- [x] 6. Wiki page, log entry, and finish

## Done and verified

| Step | Verification | Result |
|---|---|---|
| 1 Plan reader (`src/plan.rs`) | `cargo test -p fps-agent --lib plan` | Unknown fields, missing rate targets, seeds, durations, and NaN comparisons covered |
| 2 Module loading (`src/modules.rs`) | `cargo test -p fps-agent --test standalone the_example_module_trees` | Tree digests of `contracts/examples/modules` equal the example plan digests |
| 3 RuntimeConfig and native functions (`src/assemble.rs`, `src/modules.rs`) | `cargo test -p fps-agent --lib modules` | `modules/http1` and `modules/dns` manifests get native functions, the example manifests get none |
| 4 Workers, snapshots, exit codes (`src/run.rs`, `src/udp.rs`, `src/main.rs`) | `cargo test -p fps-agent --test standalone` | 4 tests pass in about 7 s, three runs in a row |
| 5 Harness test, `mk/agt.mk`, `.github/workflows/agt.yml` | `sudo engine/fps-agent/tests/netns_smoke.sh <target>/debug/fps-agent` | Both sides exit 0, 38,107 sessions succeeded on each side, 34,299 http1 and 3,808 dns |
| 6 Wiki page and log entry | `python3 tools/plan/plan.py lint` | ok |

`make lint-rust lint-shell test-agt` and `cargo deny check licenses` pass.

## Remaining

- Port setup. The agent does not configure addresses or interfaces. `fps-hosts::port::setup` moves an interface into its own namespace, and entering that namespace needs `setns`, which a later package may add.
- Scenarios with several flows, `spawn`, and backends other than `BACKEND_KERNEL` end the run with exit code 2.
- The UDP worker in `src/udp.rs` duplicates the session loop of `fps-runtime`. A later ENG package may move it into the runtime.
- No signal handling. Interrupting the process ends it without a final snapshot.

## Next action

None. A reviewer checks the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- Exit codes 0 pass, 1 fail, 2 error, with usage errors counted as errors.
- Snapshot lines report the client side when the process runs client workers and the server side otherwise, because the catalog has no side label.
- `--side client|server` splits a plan across two processes, which lets the harness test put each side in its own namespace.
- A module gets native functions only when its manifest catalog equals the crate catalog in names, order, types, and required parameters. Otherwise the run notes it on stderr and its calls fail the session.
- Engine worker id is `plan_worker << 8 | mix_index << 1 | side`.
- Client sessions target the server range of their own plan worker.
- A ratio criterion over zero sessions fails, because the value is undefined.
- `sha2` computes SHA-256, and `Cargo.lock` changed.

## Pitfalls

- `CounterSnapshot` session series need a `reason` label value from `fps_interp::Reason`, such as `expect_timeout`, or registration fails.
- The example manifests list older catalogs than the first party crates, so the example modules run without native functions.
- An unprivileged test cannot bind port 53, so the loopback tests rewrite the scenario ports and recompute the scenario digests.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/path/to/scratch/target
cargo test -p fps-agent                      # 34 tests pass
cargo build -p fps-agent
sudo engine/fps-agent/tests/netns_smoke.sh "$CARGO_TARGET_DIR/debug/fps-agent"   # about 50 s, ends with "passed on both arms"
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T15:10Z | claude-agt-01-0bc5 | claim | none | Fresh branch from origin/main |
| 2026-11-03T15:24Z | claude-agt-01-0bc5 | checkpoint | see branch | Plan reader, module loading, workers, run loop |
| 2026-11-03T15:28Z | claude-agt-01-0bc5 | checkpoint | see branch | Loopback component tests and the namespace harness test |
| 2026-11-03T15:35Z | claude-agt-01-0bc5 | finish | see branch | Wiki page Agent.md, log entry, ready for review |
