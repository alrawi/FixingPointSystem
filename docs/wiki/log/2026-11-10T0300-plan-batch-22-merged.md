## [2026-11-10] plan | Batch 22 merged

The coordinator merged CON-18 in PR #352, RLM-06 in PR #353, ENG-23 in PR #354, UI-03 in PR #355, CAP-02 in PR #358, MOD-05 in PR #356, and AGT-14 in PR #357. AGT-14 derives the run's TLS authority from the run seed so that client and server agents trust each other without an exchange; the key protects nothing, which is acceptable only because these certificates serve test traffic, and the TLS module documents it.

We applied the shared page notes to [Engine](../Engine.md), [Modules](../Modules.md), [Controller](../Controller.md), [Contracts](../Contracts.md), and [Home](../Home.md), and the `cap` workflow gains the `replay` job of CAP-02, which replays generated scenarios with the real agent in the namespace harness. The catalog gains ENG-24 (per session TLS connectors, TLS inside instance flows, and tls metrics), MOD-21 (parser errors on live http1 captures, seen twice in CI), and CON-19 (contract gaps from CAP-02 and RLM-06, which needs owner approval). AGT-16 now also registers http2 and lifts the one flow refusal per ENG-23, and MOD-06 carries the http2 manifest catalog and the fuzz job.

### Shared page notes

None.
