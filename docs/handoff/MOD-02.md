# Handoff MOD-02 DNS module

| Field | Value |
|---|---|
| Issue | #50 |
| Branch | `wp/MOD-02` |
| Pull request | none |
| Status | in progress |
| Progress | 0% (0 of 8 plan steps) |
| Current holder | claude-mod-02-c552, lease until 2026-11-03T15:51:23Z |
| Last verified commit | none |

## Goal

Ship the dns module with native functions dns.query and dns.answer_ip, a server that answers from a zone file, and lookup scenarios over UDP with TCP fallback. Cover the record types A, AAAA, CNAME, MX, and TXT. tshark parses every fixture message without errors, and nDPI classifies at least 99% of sessions as DNS.

## Plan

- [ ] 1. Read contracts, the http1 module, fps-flow TemplateFn, and the fps-interp datagram API; scaffold modules/dns
- [ ] 2. Implement the DNS wire encoder and parser and the zone file in the native crate with unit tests
- [ ] 3. Implement dns.query and dns.answer_ip through TemplateFn with unit tests
- [ ] 4. Write the manifest and scenarios and make validate.py pass
- [ ] 5. Interpreter tests over the in memory duplex and datagram sessions, catalog test
- [ ] 6. Golden transcripts with seed 42
- [ ] 7. Fixture capture in the QA-01 harness and realism score
- [ ] 8. mk and workflow files, wiki section, log entry, finish

## Done and verified

Nothing yet.

## Remaining

Every plan step.

## Next action

Read contracts/examples/modules/dns and engine/fps-interp datagram support.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-module-dns
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T14:22Z | claude-mod-02-c552 | claim | none | card 6606272123 |
