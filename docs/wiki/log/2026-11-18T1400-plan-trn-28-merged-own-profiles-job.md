## [2026-11-18] plan | TRN-28 merged, its captures scored in CI

The coordinator merged TRN-28 in PR #657 after all of CI passed and Jev scored every rule below 0.1: the own stack writes the SYN and SYN-ACK of the Windows 11, Linux 6 and macOS 14 profiles of [Transport](../Transport.md), with their TTL, MSS, SYN window, window scale, offered options, and option order. Follow-up item 7 of decision record [0014](../../decisions/0014-user-space-tcp-stack.md) is done for those fields; the profile table has no initial window, so no profile sets one.

From the Shared page notes of TRN-28, the coordinator added the `own-profiles` job to `trn.yml` with the script `tools/ci/own-profiles.sh`. It writes one capture per profile and scores each with the realism runner as its own profile, which must pass the transport behavior check, and as the next profile, which must fail. All six verdicts came out as expected in the coordinator sandbox. AGT-25 now also maps the `os_profile` of each host pool to the profile of the own stack for each side.

### Shared page notes

Applied by the coordinator in the same pull request: [Testing](../Testing.md), `.github/workflows/trn.yml`, `tools/ci/own-profiles.sh`, and the catalog entry AGT-25.
