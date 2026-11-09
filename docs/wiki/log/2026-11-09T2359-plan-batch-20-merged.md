## [2026-11-09] plan | Batch 20 merged

The coordinator merged CTL-36 in PR #327, RLM-05 in PR #328, TRN-05 in PR #329, AGT-11 in PR #331, MOD-09 in PR #333, MOD-10 in PR #332 (after resolving a Modules.md conflict), IMG-05 in PR #330 (after adding its scripts to `mk/img.mk`), CON-16 in PR #326 (after regenerating `api` and `agentpb` and resolving the wiring merge with CTL-36), and CON-17 in PR #334. Two tests failed once and passed on re-run, the CLI end to end test and the agent enrollment test; CLI-03 and AGT-15 track them.

We applied the shared page notes to [Testing](../Testing.md) and [Home](../Home.md), and the `mod-tls` workflow now runs the JA4 test of the browser profiles with tshark required. The catalog gains CLI-03 and AGT-15 (flaky tests), ENG-21 (extension seams), CTL-37 (serve listConfigs and the controller version), and ENG-22 (refuse tls_upgrade on UDP in the flow loader).

TRN-05 found that rustls cannot produce a browser ClientHello: all four profiles produce a JA4 that differs from the real browser's, and three of them share one value. A browser accurate fingerprint needs another TLS stack or a patched rustls, which is an owner decision.

### Shared page notes

None.
