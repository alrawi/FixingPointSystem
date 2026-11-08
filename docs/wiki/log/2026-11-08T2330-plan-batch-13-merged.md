## [2026-11-08] plan | Batch 13 merged, image passes on the owner host

The owner host built the qcow2 image of IMG-01 and passed all 26 boot checks, after IMG-01 switched to a static musl agent and a static controller and the coordinator fixed the Packer output directory that the host test found. The coordinator merged IMG-01 in PR #224, CTL-25 in PR #243, CLI-01 in PR #245, ENG-18 in PR #246, CTL-13 in PR #244, CTL-15 in PR #247, and CON-11 in PR #248, whose Go bindings the `regen-agentpb` workflow regenerated. CON-11 moved the three pending example tests into `contracts/examples/tests/` and set the agent protocol to 0.2.1.

We added the `cli-sdk` workflow, linked [Image](../Image.md) and [SDK](../SDK.md) from Home and the sidebar, added the SDK row to [Testing](../Testing.md), and updated the Home status. The catalog gains CTL-26 (serve the HTML report), CTL-27 (mutual TLS in the controller), and AGT-08 (mutual TLS and enrollment in the agent).

### Shared page notes

None.
