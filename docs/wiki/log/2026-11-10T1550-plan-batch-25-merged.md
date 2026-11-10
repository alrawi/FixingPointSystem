## [2026-11-10] plan | Batch 25 merged

The coordinator merged:
- CAP-05 in PR #395 and CTL-38 in PR #396.
- RLM-09 in PR #399 and UI-06 in PR #400.
- AGT-17 in PR #401 and MOD-12 in PR #402.
- CON-21 in PR #398, which the owner approved on 2026-11-10. On its branch the coordinator also:
  - set APIVersion to 0.9.0;
  - regenerated `controller/internal/api`, the UI types and the Python SDK;
  - adapted `Report.API` to the named `RealismResult` type.
- MOD-22 in PR #397, after merging main into its branch and updating the workflows:
  - mod-http2 installs its tools with `realism/ci/install-tools.sh` and no longer names `ndpi_reader_compat.py`;
  - mod-imap drops the unittest step of the removed script.
- The owner's CI fix in PR #403 (branch `ci/apt-hang`), opened by the coordinator because the owner's agent could not open pull requests:
  - `tools/ci/apt-install.sh` bounds every apt call with a timeout and retries;
  - every hosted job has a `timeout-minutes`;
  - the coordinator fixed one realism test that still looked for the old `apt-get install` line.

Until CTL-39 the controller leaves `RunReport.scenarios` out, as an older controller does, and `RunReport.realism` stays empty. No operation delivers a realism report to the controller yet.

We applied the shared page notes:
- The smb2 functions get `mk/mod-smb2.mk` and `.github/workflows/mod-smb2.yml`, whose tshark job cannot skip.
- [Testing](../Testing.md) names the pattern for a protocol without a capture.
- [Home](../Home.md) has the batch 25 status line.

The catalog gains:
- TRN-16: per profile captures scored with the transport check.
- CON-22: realism report delivery and reservation conflicts in the REST API. It needs owner approval.
- UI-08: scenario table, criterion filters and realism results in the report views.
- AGT-19: SMB2 in the agent, after MOD-13.
- RLM-14: message counts in the decrypted view, which the shared command dropped.
- IMG-07: the heartbeat timeout in the image boot test, which failed twice and passed on rerun.

Two existing packages grew:
- AGT-18 also covers the ftp sessions lost when the first_party tests ran in parallel.
- RLM-13 also reads the scenarios table of the REST report.

### Shared page notes

None.
