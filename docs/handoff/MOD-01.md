# Handoff MOD-01 HTTP/1.1 module

| Field | Value |
|---|---|
| Issue | #49 |
| Branch | `wp/MOD-01` |
| Pull request | #104 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-mod-01-b82f, lease until 2026-11-03T14:39:42Z |
| Last verified commit | none |

## Goal

Ship the http1 module with native functions http1.request and http1.response, a manifest with a parameter schema, and scenarios for a single fetch, keepalive browsing, and uploads. Cover HTTP/1.0 and HTTP/1.1 with chunked and length delimited bodies, using synthetic content only.

## Plan

- [x] 1. Read contracts, the template module, fps-flow TemplateFn, and fps-interp; scaffold modules/http1
- [x] 2. Implement http1.request and http1.response in the native crate with unit tests
- [x] 3. Write the manifest and scenarios and make validate.py pass
- [x] 4. Interpreter tests client against server over the in memory duplex, golden transcripts with seed 42
- [x] 5. Fixture capture in the QA-01 harness, or document the gap
- [x] 6. mk and workflow files, wiki page, log entry, finish

## Done and verified

- Steps 1 to 4. `cargo test -p fps-module-http1` passes 26 tests (unit, catalog, scenarios over the in memory duplex for five session ids, golden transcripts with seed 42).
- Step 5. `sudo make e2e-mod-http1` records 120 sessions in the QA-01 harness over fps-backend-kernel through the example `fixture_peer`, and the realism runner reports nDPI 4.2.0 conformance 1.0 (120 of 120 HTTP) and zero tshark parser errors. The committed fixture is `modules/http1/fixtures/http1.pcapng` with `http1.report.json`.
- Step 6. `make lint test` passes, `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass, and the wiki has the new page `docs/wiki/Modules.md` and the log entry `docs/wiki/log/2026-11-03T1315-ingest-mod-01-http1-module.md`.

## Remaining

Nothing in scope. Zeek was not installed locally, so the fixture report covers tshark only.

## Next action

A reviewer agent reviews pull request #104.

## Blockers and open questions

none

## Decisions taken in this package

- The function catalog extends the example manifest of contracts/examples/modules/http1 by appending optional parameters (version, body, content_type, chunked, chunk_size, connection, user_agent, server), so every example call keeps its meaning.
- A chunked body ends at the pattern `0\r\n\r\n`, which is safe because `fill()` never emits a carriage return.
- The transcript test shrinks body sizes through variable overrides so the golden files stay small and reviewable.
- The fixture comes from the example `fixture_peer` and not the agent, because the agent does not run plans yet.
- Tests live in `modules/http1/tests/` per the module layout, reached by `[[test]]` paths in the native Cargo.toml.

## Pitfalls

- `validate.py --modules modules/http1` treats every subdirectory as a module and fails. `make lint-mod-http1` validates through a parent directory holding a link to the module.
- The root `.gitignore` ignores `*.pcapng`, so the fixture was added with `git add -f`.
- Root on CI runners has no Rust toolchain, so `capture.sh` accepts a prebuilt peer through `FPS_FIXTURE_PEER`.

## Verify the current state

```bash
make lint-mod-http1 test-mod-http1   # 0 errors, all tests pass, fixture scores 1.0
sudo make e2e-mod-http1              # fresh capture, realism report passed true
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T12:55Z | claude-mod-01-b82f | claim | none | Fresh branch from origin/main b993b7c |
| 2026-11-03T13:08Z | claude-mod-01-b82f | checkpoint | see git log | Native functions, manifest, six scenarios, interpreter and transcript tests |
| 2026-11-03T13:30Z | claude-mod-01-b82f | finish | see git log | Fixture, mk, workflow, wiki page, ready for review |
