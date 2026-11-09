## [2026-11-09] plan | Batch 18 merged

The coordinator merged CTL-33 in PR #302, CTL-34 in PR #304, MOD-07 in PR #308, AGT-10 in PR #307, CON-14 in PR #303 (after the coordinator applied the regenerated Go API code), CAP-01 in PR #305, and CLI-02 in PR #306. The coordinator added the workflows `cap` and `cli-go`; the first `cli-go` run caught a lint finding and the first real run of the CLI end to end test, which the worker fixed.

We applied the shared page notes to [Controller](../Controller.md), [Testing](../Testing.md), [Home](../Home.md), and the sidebar, which now link [CLI](../CLI.md) and [Capture](../Capture.md). The catalog gains AGT-13 (first party native modules in the agent, so smtp plans run), ENG-20 (TLS upgrade of a running flow for STARTTLS), CTL-35 (list runs), and CON-16 (controller version, configuration listing, and an OpenAPI breaking check, which needs owner approval).

### Shared page notes

None.
