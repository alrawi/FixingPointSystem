# Modules

This page describes the protocol modules under `modules/`, which the MOD workstream owns. A module ships a manifest, scenarios, an optional native crate, fixtures, and tests, with the layout that the Modules section of [Coding Standards](Coding-Standards.md#modules) defines and the four proofs that [Testing](Testing.md#testing-a-new-module) requires.

## How a module plugs in

A module joins the build without any shared file edit. The workspace glob `modules/*/native` in the root `Cargo.toml` picks up its native crate, the rust job of `ci.yml` runs its Rust tests through `cargo test --workspace`, and a file `mk/mod-<name>.mk` adds its lint, test, and end to end targets. Decision D10 lets a first party module implement its template functions in Rust. The crate implements the `TemplateFn` trait of `fps-flow`, and the manifest lists the same functions under `functions`, as contract v0.2 requires. A catalog test in the module compares the two lists, so the manifest stays the single catalog that the validator and the controller read.

The table lists the modules that exist today.

| Module | Native crate | Scenarios | Fixture |
|---|---|---|---|
| [`http1`](https://github.com/alrawi/FixingPointSystem/tree/main/modules/http1) | `fps-module-http1` with `http1.request` and `http1.response` | `single_fetch`, `keepalive_browse`, `upload`, `upload_chunked`, `http10_fetch`, `http10_keepalive` | 120 sessions, nDPI ratio 1.0 for HTTP |

## The http1 module

MOD-01 added the `http1` module. Its two functions render complete HTTP/1.0 and HTTP/1.1 messages with a length delimited body or, in HTTP/1.1, a chunked body. They check every text argument before they write a byte, so a header value with a line break fails the session with `extension_invalid_argument` instead of producing a broken frame. The module README lists the parameters, their defaults, and the header order.

The six scenarios cover a single fetch, keepalive browsing with a chunked page and several embedded objects, length delimited and chunked uploads, and HTTP/1.0 with and without keepalive. A receiving side finds a header block with the pattern `\r\n\r\n`, reads a length delimited body with `expect bytes`, and reads a chunked body with the pattern `0\r\n\r\n`. The chunk pattern is safe because `fill()` never produces a carriage return.

We test the module at four levels.

1. `make lint-mod-http1` validates the manifest and every scenario with `tools/contracts/validate.py`.
2. The scenario tests run both sides of every scenario over the in memory duplex and parse both directions with the framing rules of RFC 9112.
3. The transcript test compares the bytes of every scenario at seed 42 with golden files under `modules/http1/tests/testdata/`.
4. `fixtures/capture.sh` records 120 sessions in the QA-01 harness over the kernel TCP backend, and the realism runner scores the capture. nDPI 4.2.0 classifies every session as HTTP, and tshark reports no parser errors.

The example `fixture_peer` drives the interpreter over kernel sockets without the agent. Both sides walk the scenarios in manifest order and run the sessions one after another, so the n-th accepted connection belongs to the n-th client session and both sides draw from the same session generators. The workflow `mod-http1` scores the committed fixture and records and scores a fresh capture as root on every change.

## Known gaps

We record three gaps that later packages close.

1. `validate.py --modules modules/http1` treats every subdirectory of the module as a module and fails, so `lint-mod-http1` validates through a parent directory that links to the module.
2. The root `.gitignore` ignores `*.pcapng` outside `contracts/examples`, so module fixtures need `git add -f` until the ignore list allows `modules/*/fixtures/*.pcapng`.
3. The fixture comes from `fixture_peer` and not from the agent, because the agent does not run plans yet. Gate 1 records its capture through the agent.

## The dns module

MOD-02 added the `dns` module under [`modules/dns`](https://github.com/alrawi/FixingPointSystem/tree/main/modules/dns). Its native crate `fps-module-dns` holds an RFC 1035 codec with name compression, a parser for zone files in master file format, and four template functions. `dns.query` renders a query with a transaction id from the session generator, `dns.response` answers it from the zone or from one given address, `dns.answer_ip` returns the first address of an answer for a `set` action, and `dns.tcp_length` reads the two byte length prefix of a TCP message. The catalog extends the contract example by appending optional parameters and the function `tcp_length`, so every example call keeps its meaning. The module README lists every parameter and the answer rules.

The server answers as an authoritative server of `zones/example.test.zone`, which holds A, AAAA, CNAME, MX, TXT, NS, and SOA records with names under `.test` and addresses in the documentation ranges. It follows CNAME chains, answers a missing name with NXDOMAIN and the SOA record, and adds glue addresses to MX answers. A UDP answer longer than 512 bytes keeps only the question and sets TC, and the scenario `tcp_fallback` then repeats the question over TCP, where each side reads the length prefix with `expect bytes: 2` and the message with `expect bytes` of the bound length.

The seven scenarios are `lookup_a`, `lookup_aaaa`, `lookup_cname`, `lookup_mx`, `lookup_txt`, `nxdomain`, and `tcp_fallback`. We test the module at the same four levels as http1. The parser also survives 20,000 random inputs without a panic. The transcript files carry the flow id in their names, as in `tcp_fallback.udp.seed42.c2s.bin`, and a UDP transcript prefixes each datagram with its length so the file keeps the boundaries.

`fixtures/capture.sh` records 160 sessions in the QA-01 harness, 140 over UDP and 20 over TCP. TCP flows run over the kernel TCP backend, and UDP flows run over Tokio UDP sockets inside `fixture_peer` until the kernel UDP backend of TRN-02 merges. nDPI 4.2.0 classifies 160 of 160 sessions as DNS, and tshark 4.2.2 reports no parser errors. The workflow `mod-dns` scores the committed fixture and records and scores a fresh capture as root on every change.

The dns module adds two gaps to the list above. The zone of a run is the bundled zone, because neither the manifest nor the test configuration can name a zone file yet. Queries carry no EDNS OPT record, so the server applies the 512 byte limit of RFC 1035 to every UDP answer.
