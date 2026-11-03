## [2026-11-03] ingest | ENG-06 UDP datagram sessions

We added the `DatagramIo` trait, its async helpers, and the in memory `datagram_pair` to `engine/fps-io/`, and UDP sessions to `engine/fps-interp/` with `DatagramOpener`, `Session::run_datagram_program`, `Session::open_datagram`, `Session::run_datagram`, and `Session::set_max_datagram`. Each `send` writes one datagram, each `expect` reads and consumes exactly one datagram that also becomes `last`, an oversized datagram fails with reason `pattern_mismatch`, and `close` releases the socket. The DNS lookup example of the contract passes over the in memory pair, and the datagram limit and the timeouts have tests. Existing public items are unchanged, and the additions are new items plus two new `FailureKind` variants. We added the section [UDP datagram sessions](Engine.md#udp-datagram-sessions) to the Engine page. The evidence is `docs/handoff/ENG-06.md` and the tests in `engine/fps-interp/tests/datagrams.rs` and `engine/fps-io/tests/datagram.rs`.

## Shared page notes

- [Engine](Engine.md), crate table. The `fps-io` row can add "and the `DatagramIo` seam with the in memory `datagram_pair`", and the `fps-interp` row can drop ENG-06 from its next packages and mention UDP sessions, see [UDP datagram sessions](Engine.md#udp-datagram-sessions).
- [Engine](Engine.md), the SessionIo seam section. It can point to the new section for UDP flows.
- Follow up for the ENG workstream, in the `fps-flow` paths. `Op::Expect` drops the per action `max_datagram` of the contract, so the interpreter applies one limit per side through `Session::set_max_datagram`. Carrying the field in `Op::Expect` would let the interpreter honor each action, and it would change a public enum of `fps-flow`.
- Follow up for ENG-10. The runtime picks `run_program` or `run_datagram_program` from `Flow::l4`, and it calls `set_max_datagram` when a scenario sets a side wide limit.
- `FailureKind` gained `DatagramTooLarge` and `DatagramMismatch`. A downstream `match` over `FailureKind` without a wildcard arm needs the two arms.
