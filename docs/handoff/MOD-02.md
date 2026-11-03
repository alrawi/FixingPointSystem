# Handoff MOD-02 DNS module

| Field | Value |
|---|---|
| Issue | #50 |
| Branch | `wp/MOD-02` |
| Pull request | none |
| Status | in progress |
| Progress | 88% (7 of 8 plan steps) |
| Current holder | claude-mod-02-c552, lease until 2026-11-03T15:51:23Z |
| Last verified commit | none |

## Goal

Ship the dns module with native functions dns.query and dns.answer_ip, a server that answers from a zone file, and lookup scenarios over UDP with TCP fallback. Cover the record types A, AAAA, CNAME, MX, and TXT. tshark parses every fixture message without errors, and nDPI classifies at least 99% of sessions as DNS.

## Plan

- [x] 1. Read contracts, the http1 module, fps-flow TemplateFn, and the fps-interp datagram API; scaffold modules/dns
- [x] 2. Implement the DNS wire encoder and parser and the zone file in the native crate with unit tests
- [x] 3. Implement dns.query and dns.answer_ip through TemplateFn with unit tests
- [x] 4. Write the manifest and scenarios and make validate.py pass
- [x] 5. Interpreter tests over the in memory duplex and datagram sessions, catalog test
- [x] 6. Golden transcripts with seed 42
- [x] 7. Fixture capture in the QA-01 harness and realism score
- [ ] 8. mk and workflow files, wiki section, log entry, finish

## Done and verified

- Steps 1 to 6. `cargo test -p fps-module-dns` passes 40 tests (unit tests of the codec, zone parser, and functions, catalog, scenarios over the in memory datagram pair and duplex for four session ids, golden transcripts with seed 42). `make lint-mod-dns` passes validate.py on the manifest and the seven scenarios.
- Step 7. `sudo make e2e-mod-dns` records 160 sessions (140 UDP, 20 TCP) in the QA-01 harness through the example `fixture_peer`, and the realism runner reports nDPI 4.2.0 conformance 1.0 (160 of 160 DNS) and zero tshark 4.2.2 parser errors. The committed fixture is `modules/dns/fixtures/dns.pcapng` with `dns.report.json`.

## Remaining

Step 8, the full `make lint test`, the pull request, and CI.

## Next action

Run make lint test, open the pull request, and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- The catalog extends the contract example manifest. `query` gains `tcp`, `response` makes `address` optional and gains `tcp` and `udp_size`, and the new function `tcp_length` reads the TCP length prefix, so a TCP side reads a message with `expect bytes: 2` and `expect bytes` of a bound length. Every example call keeps its meaning.
- The zone comes from the `Dns` value, which serves the bundled `zones/example.test.zone` by default and any parsed zone through `Dns::new`. Scenarios cannot name a zone yet, because no contract field carries module settings.
- The server answers as an authoritative server (AA set, RA clear, RD copied). Names outside the zone get REFUSED.
- TCP fallback is a scenario with a UDP flow and a TCP flow `after` it. The interpreter cannot branch on the TC bit, so the scenario queries a record whose answer always exceeds 512 bytes.
- `fixture_peer` runs UDP flows over Tokio UDP sockets, because TRN-02 has not merged. The server binds its UDP socket and TCP listener at start, so no session races a bind.
- UDP golden transcripts prefix each datagram with its two byte length, so the files keep datagram boundaries. Each flow has its own pair of files.
- Queries carry no EDNS OPT record. This keeps the 512 byte limit and the fallback deterministic.

## Pitfalls

- `Pcg64Mcg::new` forces the two lowest state bits, so seeds 42 and 43 give the same stream. Tests that compare seeds use 7 and 42.
- `validate.py --modules` needs a parent directory that holds only the module, as for http1. `make lint-mod-dns` builds it under the Cargo target directory.
- The root `.gitignore` ignores `*.pcapng`, so the fixture was added with `git add -f`.

## Verify the current state

```bash
cargo test -p fps-module-dns          # 40 tests pass
make lint-mod-dns test-mod-dns        # validate.py and the fixture score pass
sudo make e2e-mod-dns                 # a fresh capture scores 160 of 160 DNS
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T14:22Z | claude-mod-02-c552 | claim | none | card 6606272123 |
