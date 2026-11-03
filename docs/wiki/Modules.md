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
