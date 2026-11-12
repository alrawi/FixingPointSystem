## [2026-11-12] plan | Batch 31 merged

The owner approved CON-28 and CON-29 on 2026-11-12, and asked that every change to module development be documented so that third parties can develop their own modules.

Before the batch, the coordinator added MOD-26 and a rule in [Coding Standards](../Coding-Standards.md) in PR #501. A pull request that changes how a module is written updates [Module Development](../Module-Development.md) in the same pull request. The worker brief carries the same rule.

The coordinator merged:
- RLM-16 in PR #505, CTL-48 in PR #503 (after it merged main with CON-28), and CON-28 in PR #507 (catalog 0.8.0, report format 0.17.0).
- MOD-26 in PR #508. On its branch the coordinator added the `mod-template` workflow from its shared page notes and linked the guide from Home and the sidebar.
- QA-11 in PR #506. On its branch the coordinator set `overwrite: true` on every artifact upload of qa-gate2.yml, so a rerun never judges an earlier attempt's files.
- CTL-47 in PR #504. On its branch the coordinator applied its shared page notes before merge: the `expected_bytes` subsection of the guide, the removal of the guide's statements that the controller does not read the field, and CTL-47 as a reader in [Contracts](../Contracts.md).

CON-29 was closed without a change. `Run.calibration`, with its multipliers, has been in the REST contract since CON-14. The coordinator had written the package from a QA-05 finding without checking the contract. The real gap is the Gate 2 report (QA-12).

What the workers found:
- The guide lists gaps between the contract and the code that third party authors meet (CON-30). Native crates are first party only under D10, which is still Proposed. The SDK cannot upload a signed module, and settings work only for modules with native functions.
- Only hls and sip_rtp of the 15 manifests carry `expected_bytes`, so the Gate 2 byte basis waits for MOD-25.
- tshark port collisions explain the Gate 2 parser findings that QA-11 could reproduce. The original 10 findings of PR #486 are not proven to be the same.
- The `score` command no longer accepts abbreviated options, so a token can never be read as a token file (RLM-16).

We applied the shared page notes:
- [Testing](../Testing.md) names the batch 31 tests.
- [Home](../Home.md) has the batch 31 status line.

The catalog gains:
- CON-30: module manifest rules that match the registry and the agent. It needs owner approval.
- QA-12: calibration in the Gate 2 report.
- RLM-19: the port collision rule in the realism runner.

### Shared page notes

None.
