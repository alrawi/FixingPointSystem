## [2026-11-03] ingest | TRN-01 Kernel TCP backend

We created the crate `engine/fps-backend-kernel`, which implements `SessionIo` over kernel TCP sockets, and added the [Transport](Transport.md) page. Clients bind host pool source addresses with `IP_BIND_ADDRESS_NO_PORT` and an optional per socket port range through `IP_LOCAL_PORT_RANGE`, listeners accept on pool addresses with a configurable backlog, and both apply buffer size, Nagle, and linger options. The acceptance test opens 10,000 sessions from 100 source addresses in 127.0.0.0/8 without an address error. Because `socket2` 0.6 exposes neither socket option, the crate denies `unsafe` code and allows it in one private module that calls `setsockopt`, which the Transport page records. We linked the page from [Home](Home.md) and the sidebar with the coordinator's approval. The evidence is `cargo test -p fps-backend-kernel` and `docs/handoff/TRN-01.md` on the branch `wp/TRN-01`.

## Shared page notes

The coordinator may apply the notes below.

1. [Engine](Engine.md) states that every engine crate carries `#![forbid(unsafe_code)]`. The crate `fps-backend-kernel` carries `#![deny(unsafe_code)]` with one allowed module, so the sentence should name it as the exception and link [Transport](Transport.md#unsafe-code).
2. [Implementation Plan](Implementation-Plan.md) may mark TRN-01 as done after the merge. TRN-02 and TRN-03 share the path `engine/fps-backend-kernel/`, so they cannot run in the same batch.
