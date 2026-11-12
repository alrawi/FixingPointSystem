## [2026-11-12] plan | Batch 32 merged

The owner approved CON-30 on 2026-11-12 and asked for the follow up packages of issue #491. MOD-27, MOD-28 and MOD-29 carry an owner action and are assigned to the owner, and RLM-20 and ENG-29 are swarm work (PR #514).

The coordinator merged the following, and on each branch applied the shared page notes before merge, as the module documentation rule asks.
- QA-12 in PR #515, with no change.
- CON-30 in PR #517, manifest contract v0.5. On its branch the coordinator changed the render hook test to `extension.wasm`, regenerated the UI types, and applied the 15 edits of the guide. The edit on the `raw` action is corrected: the contract check of the engine admits it for strike and fuzzer modules, and the interpreter does not run it until ENG-29.
- RLM-18 in PR #516, with its paragraph on the decrypted view in the guide.
- MOD-25 in PR #518. On its branch the coordinator fixed the Gate 2 staging test and applied its notes to Gates, Testing and Implementation Plan.
- RLM-19 in PR #519, report format 0.18.0. On its branch the coordinator applied the tested Gate 2 patch of the worker, so the suite takes the runner's verdict and has no copy of the rule of its own. The coordinator also refreshed the controller schema copy and the UI types, fixed a type annotation of the patch, and added the paragraph on findings that do not count to the guide.

What the workers found:
- With every estimate in place, the bytes basis of Gate 2 misses by 7.84 points, because the run counts the FTP data connection and the RTP media stream as sessions (CTL-49).
- The registry is looser than the validator for three reserved names and the keys of `expected_bytes` (CTL-50).
- smtp and imap still rely on the TLS heuristic of tshark in their decrypted view (RLM-21).
- The coordinator removed the worktrees of merged packages, because the sandbox disk had filled and workers met ENOSPC.

We applied the shared page notes:
- [Testing](../Testing.md) names the batch 32 tests.
- [Home](../Home.md) has the batch 32 status line.

The catalog gains:
- CTL-49: a spawned flow counts with the session that opened it.
- CTL-50: registry rules as strict as the validator.
- RLM-21: STARTTLS ports in the decrypted view.

### Shared page notes

None.
