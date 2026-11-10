## [2026-11-10] plan | Batch 24 merged

The coordinator merged:
- TRN-15 in PR #382 and ENG-25 in PR #383.
- UI-05 in PR #379 and CAP-04 in PR #381.
- RLM-12 in PR #384, AGT-16 in PR #385, and MOD-11 in PR #386.
- CON-20 in PR #380, which the owner approved on 2026-11-10. On its branch the coordinator also:
  - refreshed the embedded catalog and module manifest copies;
  - set APIVersion to 0.8.0;
  - updated the fps-stats label tables and the contract test;
  - regenerated the UI types and the Python SDK. The Go regen produced no change.
- MOD-06 in PR #371, after merging main into its branch. nDPI 4.14 labels every cleartext HTTP/2 session, and all its checks passed, the image boot included.

Before the batch, RLM-12 stopped depending on MOD-06, which removed a dependency cycle (PR #378). The apt hang also left RLM-12's scope.

The CI hang of 10 November had a single cause:
- On runner image 20261101.360, `apt-get update` stalled after the InRelease files, in 14 jobs across realism, agt, trn, rlm, mod-http1 and mod-smtp.
- Each stall lasted until GitHub's 6-hour limit cancelled the job, because no job set `timeout-minutes`.
- The owner's agent fixes it on branch `ci/apt-hang` with an apt timeout and retry helper and job timeouts.

We applied the shared page notes:
- The module workflows mod-http1, mod-http2, mod-smtp and mod-imap call `python3 -m realism decrypted` instead of the module copies of `decrypted_check.py`.
- The ssh module gets `mk/mod-ssh.mk` and `.github/workflows/mod-ssh.yml`.
- [Testing](../Testing.md) describes the realism tools in CI.
- [Home](../Home.md) has the batch 24 status line.

The catalog gains:
- AGT-17: ssh in the agent, TLS profiles from the TLS module, and a digested manifest.
- AGT-18: the run_control capture flake.
- CON-21: scenario totals, criterion filters and realism results in the REST report. It needs owner approval.
- CTL-39: the REST report filled in.
- RLM-13: classifier mix series.
- CAP-05: provenance fields from the scrub record.
- MOD-22: the shared decrypted check in the module capture scripts, removing the old copies.

### Shared page notes

None.
