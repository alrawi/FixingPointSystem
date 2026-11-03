# Handoff MOD-01 HTTP/1.1 module

| Field | Value |
|---|---|
| Issue | #49 |
| Branch | `wp/MOD-01` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-mod-01-b82f, lease until 2026-11-03T14:25:49Z |
| Last verified commit | none |

## Goal

Ship the http1 module with native functions http1.request and http1.response, a manifest with a parameter schema, and scenarios for a single fetch, keepalive browsing, and uploads. Cover HTTP/1.0 and HTTP/1.1 with chunked and length delimited bodies, using synthetic content only.

## Plan

- [ ] 1. Read contracts, the template module, fps-flow TemplateFn, and fps-interp; scaffold modules/http1
- [ ] 2. Implement http1.request and http1.response in the native crate with unit tests
- [ ] 3. Write the manifest and scenarios and make validate.py pass
- [ ] 4. Interpreter tests client against server over the in memory duplex, golden transcripts with seed 42
- [ ] 5. Fixture capture in the QA-01 harness, or document the gap
- [ ] 6. mk and workflow files, wiki page, log entry, finish

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Read contracts/README.md and engine/fps-flow to learn the TemplateFn trait.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 tools/contracts/validate.py --modules modules/http1
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T12:55Z | claude-mod-01-b82f | claim | none | Fresh branch from origin/main b993b7c |
