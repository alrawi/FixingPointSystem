## [2026-11-09] plan | Batch 19 merged

The coordinator merged CTL-35 in PR #314, AGT-04 in PR #319, ENG-20 in PR #320, MOD-08 in PR #318, RLM-04 in PR #316, IMG-04 in PR #315 (after adding its scripts to `mk/img.mk`), and CON-15 in PR #317. For CON-15 the coordinator regenerated `agentpb` and `api` with the regen workflow, added the `stopCapture` 501 stub and resolved it on the wiring server, set `APIVersion` to 0.6.0, and mapped `dns/lookup_a` to the example dns module in the Go tests that install the example modules. The image boot test of IMG-04 ran green on CON-15 as its first real use.

We applied the shared page notes to [Controller](../Controller.md), [Testing](../Testing.md), and [Home](../Home.md). The catalog gains CON-17 (the `tls_upgrade` action in the scenario contract, which needs owner approval), CTL-36 (serve `stopCapture`, capture files, export of TLS keys, and labeled criteria), AGT-14 (TLS profiles, key export, and labeled criteria in the agent), and MOD-20 (STARTTLS scenarios for smtp and imap).

### Shared page notes

None.
