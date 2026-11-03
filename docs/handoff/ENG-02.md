# Handoff ENG-02 Scenario loader and semantic checks

| Field | Value |
|---|---|
| Issue | #22 |
| Branch | `wp/ENG-02` |
| Pull request | #94 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-eng-02-ac55, lease until 2026-11-03T13:24:51Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Load scenario YAML into the program model with typed errors that name the file, flow, side, and action index, and apply the semantic rules of `tools/contracts/validate.py` for flows, `after`, `spawn`, `branch`, `last`, and module calls against the manifest `functions` catalog. One shared fixture directory feeds both validators, and every fixture gets the same verdict from Rust and Python.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Typed error location (`Location` with file, flow, side, action path) in place of the string `at`
- [x] 2. Semantic rules for flows, after, spawn, branch, delay, expect datagram limits, and `last` placement
- [x] 3. Module call check against the manifest `functions` catalog
- [x] 4. Shared fixtures under `engine/fps-flow/tests/fixtures/` with a Rust test and a pytest that runs validate.py on them
- [x] 5. Fix the bug #90 test helper, Engine page section, log entry

## Done and verified

- Step 1. `src/location.rs` holds `Location`, `Side`, `Step`, and `FlowRef`. `src/error.rs` holds `LoadError`, now `#[non_exhaustive]`, with a boxed `Location` in every variant except `Read`, plus `LoadError::kind` and `LoadError::location`, and `Warning`. Verified with `cargo test -p fps-flow --lib`.
- Step 2. `src/check.rs` mirrors `check_scenario` and `_check_side_templates` of `validate.py` over the YAML document. `Scenario::parse_with` runs it before the compiler.
- Step 3. `src/manifest.rs` reads `Manifest`, `ModuleSet`, and `ModuleContext`. `src/template/check.rs` adds `Template::check` with `CheckScope`, typed variables, `last` placement, and module call binding.
- Step 4. `tests/fixtures/modules/{fx,aux,other,st}` with 43 scenarios and `tests/fixtures/verdicts.yaml`. `cargo test -p fps-flow --test fixtures` passes 5 tests, and `python3 -m pytest -q engine/fps-flow/tests` passes 46 tests. Every verdict, and every listed pointer, holds in both.
- Step 5. `tests/common/mod.rs` builds the generator as the Coding Standards example does, `tests/templates.rs` has the regression property `neighboring_session_ids_draw_distinct_streams`, and `tests/testdata/grammar.seed42.txt` was regenerated because its random draws changed. Engine page section "The scenario loader", log entry, and `.github/workflows/eng.yml` are written.
- `cargo clippy -p fps-flow --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Remaining

- The Rust loader has no JSON Schema step. Identifier patterns, `set` values that are not strings, and similar schema rules still pass in Rust, so the fixtures cover only schema valid scenarios.
- `mk/eng.mk` does not run the pytest half yet. The log entry asks the coordinator to add it, and `eng.yml` runs it in CI meanwhile.
- The Rust loader stops at the first error, while the Python validator reports every finding.
- Overrides arrive after load, so `Scenario::override_vars` does not repeat the type checks of variables.

## Next action

The coordinator merges PR #94 and applies the shared page notes of the log entry.

## Blockers and open questions

none

## Decisions taken in this package

- `Scenario::check` returns the contract verdict and accepts `spawn`, `raw`, spawned flows, and transport modules, while `Scenario::load` keeps rejecting them with `Unsupported`. Adding `Op` variants for them would break the exhaustive matches of ENG-04, so the compiler leaves them out of the programs in check mode.
- The semantic pass walks the YAML document instead of the compiled programs, so it tolerates malformed structure as the Python checks do, and it runs before the compiler so that a semantic error wins over `Unsupported`.
- `Location` is boxed inside `LoadError`, because clippy rejects a large `Err` variant and errors are a cold path.
- Template rules use the declared variable types as `validate.py` does, so `fill(vars.n)` with a text value now fails at load time. The render time leniency of ENG-03 for text that parses as an integer stays for overrides.
- `RenderCtx`, `Op`, `Program`, `Flow`, and `Scenario` keep their public shape, so ENG-04 is unaffected.
- `ModuleContext` lists the owner and every required module. A required module without a manifest yields a warning and unchecked calls, as in Python.

## Pitfalls

- The Python test attributes findings by repository relative path, so fixture modules must keep valid manifests, or every scenario of the module disappears from the scenario checks.
- `serde_norway::Mapping` has no `get_key_value`.

## Verify the current state

```bash
export CARGO_TARGET_DIR=$PWD/../scratch-eng-02/target
cargo test -p fps-flow                       # all suites pass
python3 -m pytest -q engine/fps-flow/tests   # 46 passed
python3 tools/contracts/validate.py          # contracts unchanged and clean
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:39:02Z | claude-eng-02-ac55 | claim | none | Started from origin/main f4b9637 |
| 2026-11-03T11:57:00Z | claude-eng-02-ac55 | finish | see PR | Ready for review |
