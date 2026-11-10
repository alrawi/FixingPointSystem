# Handoff ENG-24 TLS per session connectors, instance TLS, and tls metrics

| Field | Value |
|---|---|
| Issue | #360 |
| Branch | `wp/ENG-24` |
| Pull request | #366 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-e24a7c, lease until 2026-11-10T06:24:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`TlsProfiles` builds one client connector per session (so a client against a ticket issuing server keeps one ClientHello JA4), every flow of a `ScenarioInstance` gets a `TlsUpgrader`, and the runtime records the `tls.*` handshake metrics of the catalog from `TlsSession` reports.

## Plan

- [x] 1. `TlsProfiles::with_client_factory`, `SessionTls` upgrader with a handshake event sink, `FlowEnv::tls_upgrader`.
- [x] 2. `tls.*` metrics in the worker shard (`TlsMetrics`), `FlowRef::port` for the `port` label.
- [x] 3. Test `tests/tls_sessions.rs`: three sessions, same JA4 (computed with `realism/ja4.py`), a shared connector control, a spawned flow with `tls_upgrade`, the metrics in a snapshot.
- [x] 4. Update `docs/wiki/Engine.md`, the log entry, and run lint and tests.
- [x] 5. Pull request and CI.

## Done and verified

- `cargo test -p fps-runtime -p fps-interp` passes, including `tests/tls_sessions.rs` (5 tests).
- `cargo clippy -p fps-runtime -p fps-interp --all-targets -- -D warnings` and `cargo fmt --check` are clean; `plan.py check`, `render --check` pass.

## Remaining

Nothing in the package. CI on the PR: `rust` job and every other job green except `module (http1)` (realism parser errors on the live capture, the known MOD-21 flake: `parser_errors | http1 | fail | 2 parser errors`, https group 0) and `module (ftp)`, which sat in the step that installs the harness (nDPI, tshark, Zeek) for 25 minutes.

## Next action

Review PR #366.

## Blockers and open questions

none

## Decisions taken in this package

- The factory keeps the profile's resumption setting and only gives each session a fresh store. `ClientResumption::Disabled` would drop the TLS 1.2 `session_ticket` extension and change the JA4.
- `with_client(connector)` stays and stays shared (the `TlsConnector` API cannot clone a store). `with_client_factory` is the new per session form.
- The `port` label of the tls series comes from `FlowRef::port` (the catalog says test port name), default `default`.
- `fps-interp` gets `FlowEnv::tls_upgrader` with a default of `None`, so other `FlowEnv` implementations keep compiling.

## Pitfalls

A cold build of `fps-runtime` takes about 6 minutes with one job.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-ENG-24 CARGO_BUILD_JOBS=1 cargo test -p fps-runtime --test tls_sessions
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-e24a7c | claim | none | claimed #360 |
| 2026-11-10 | claude-e24a7c | checkpoint | HEAD | tests, docs, log entry written |
| 2026-11-10 | claude-e24a7c | ready for review | HEAD | PR #366 opened, rust job green |
