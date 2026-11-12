## [2026-11-13] plan | Batch 33 merged

The owner gave the go for batch 33 on 2026-11-12. CON-30 had already merged in batch 32.

The `rust` job of CI had failed `cargo fmt --check` on main and on every pull request, because the stable toolchain it installs brought a rustfmt that formats the arguments of the `spec!` macros of `modules/ldap/native/src/catalog.rs` one per line. The job stopped at that step, so clippy and the workspace tests did not run in CI. The coordinator reformatted the file in PR #533, which the older rustfmt 1.9.0 also accepts, and the job passed its format, clippy, and test steps.

The coordinator merged the following. On each branch it first merged main and applied the shared page notes, as the module documentation rule asks.
- RLM-20 in PR #531, report format 0.19.0, with its paragraph on expected findings in [Module Development](../Module-Development.md) and its test in [Testing](../Testing.md).
- ENG-29 in PR #532, with its three edits of the guide on the `raw` action.
- CTL-50 in PR #529. On its branch the coordinator also set `REGISTRY_RESERVED_NAMES` of `validate.py` to the eight names and updated its test, which changes no verdict of the validator, and applied its six edits of the guide.
- MOD-24 in PR #530, with its notes for Testing and [Realism](../Realism.md).

CTL-49 did not start. The worker found that the engine makes a spawned flow a session of its own (`engine/fps-runtime/src/instance.rs`, `worker.rs`), as `contracts/README.md` states. The controller cannot fix the count without guessing, so the fix is a change of contract (CON-31, needs owner approval) and of the engine (ENG-30). CTL-49 stays blocked until ENG-30 merges.

What the workers found:
- `raw` writes on the session socket only. It does not send stateless packets or forge source addresses, which MOD-29 needs for floods and spoofed pools.
- A declared parser finding is met by one finding of any session of its rule, so a rule over many sessions does not prove that every session is malformed (RLM-20).
- The labels cannot express the request cadence of HLS, so `hls_check.py` stays (RLM-22). The sip_rtp labels test no mean gap, so a slow drift inside the tolerance passes.

We applied the shared page notes:
- [Testing](../Testing.md) names the batch 33 tests and the time without clippy and tests in CI.
- [Home](../Home.md) has the batch 33 status line.

The catalog gains:
- CON-31: a session is a scenario session, and its spawned flows add bytes. It needs owner approval.
- ENG-30: the engine counts a scenario session once, after CON-31.
- RLM-22: request cadence and bytes for HLS in the realism labels.

### Shared page notes

None.
