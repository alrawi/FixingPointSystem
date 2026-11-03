# Handoff ENG-03 Template compiler with module functions

| Field | Value |
|---|---|
| Issue | #23 |
| Branch | `wp/ENG-03` |
| Pull request | #85 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-eng-03-4b2d, lease until 2026-11-03T04:29:02Z |
| Last verified commit | none |

## Goal

Compile templates of contract v0.2 into byte segments and call slots, dispatch module function calls through a `TemplateFn` trait, fold templates without session dependent parts into constants, and print a human readable program dump for golden tests.

## Plan

- [x] 1. Parser for the v0.2 grammar with nested calls, keyword arguments, float, boolean, and `last`
- [x] 2. Segment compiler with core functions, module call slots, and constant folding
- [x] 3. The `TemplateFn` trait and dispatch at render time
- [x] 4. Program dump and golden tests for every grammar rule
- [x] 5. Property test, Engine page, log entry

## Done and verified

- Steps 1 to 3. `src/template/parse.rs` holds the tokenizer and parser, `src/template.rs` the segment compiler, folding, and renderer, and `src/function.rs` the `TemplateFn` trait with `ArgValue`, `CallArgs`, `FnCall`, `FnError`, and `NoFunctions`. Verified with `cargo test -p fps-flow --lib`, 26 tests pass.
- Step 4. `Template::dump`, `Template::write_dump`, and `Scenario::dump`. `tests/grammar.rs` covers every grammar rule against `tests/testdata/grammar.seed42.txt`, and `tests/contract_examples.rs` checks every example scenario before and after `Scenario::fold_constants` against `tests/testdata/examples/*.dump`.
- Step 5. `tests/templates.rs` holds the determinism property and the folding property. Engine page Templates section and the log entry are written. `cargo clippy -p fps-flow --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Remaining

- Calls are not checked against a module function catalog in Rust. `Template::module_calls` exposes the call sites, and the check belongs to the package that loads module manifests (ENG-02 or ENG-11).
- `last` precedence is not checked in Rust. The validator checks it, and `Template::uses_last` exposes the use.
- Module names resolve by string at every call. A later package may intern them into module table indices when it profiles the interpreter.
- Argument evaluation for a module call allocates two small vectors per call, and a nested call or `fill` argument allocates its bytes.

## Next action

The coordinator merges PR #85. ENG-04 implements `TemplateFn` over the session module table.

## Blockers and open questions

none

## Decisions taken in this package

- The dispatch trait `TemplateFn` lives in `fps-flow`, because `contracts/extension/README.md` names no such trait and no `fps-extension` crate exists yet. Its argument types mirror the WIT `value` and `call-args` items, so the adapter to `ExtensionSession::render` is a field copy.
- `TemplateFn::render` receives the session generator as `&mut dyn RngCore`, so templates and module code draw from one stream and replays stay exact. `RenderCtx` therefore takes a sized `R: RngCore` instead of `R: Rng + ?Sized`.
- `Template::compile` folds only parts that need neither variables nor the generator. Variables fold in `Scenario::fold_constants`, which runs after overrides and skips every name that a `set` action binds, because overrides arrive after load.
- Core functions bind their arguments at compile time with static type checks. Variables have type `any`, so `fill(vars.x)` checks at render time as before, including the v0.1 leniency that parses a text variable as an integer.
- `session.id` as an argument must fit `int`, which is s64. Session ids above `i64::MAX` fail with `NotInteger`, while `{{ session.id }}` at the top level still renders any u64.
- The test module stand in `Echo` prints long text and bytes arguments as their length, so nested calls keep the golden file small.

## Pitfalls

- `fill(fill(1))` is a type error, because `fill` returns text. Nesting tests use module calls.
- Proptest wrote a regressions file next to `tests/templates.rs` while the session range was wrong. Delete such files when the cause was a test bug.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/tmp/fps-target
cargo test -p fps-flow                       # all tests pass
cargo clippy -p fps-flow --all-targets -- -D warnings
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-eng-03-4b2d | claim | none | card 6594102239 |
| 2026-11-03T03:05Z | claude-eng-03-4b2d | finish | pending | ready for review |
