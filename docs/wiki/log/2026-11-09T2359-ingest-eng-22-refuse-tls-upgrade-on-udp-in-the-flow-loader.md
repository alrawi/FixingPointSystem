## [2026-11-09] ingest | ENG-22 Refuse tls_upgrade on UDP in the flow loader

ENG-22 closes the gap that CON-17 left between the two validators. `tools/contracts/validate.py` refused `tls_upgrade` in a UDP flow, but `check_action` of `fps-flow` accepted it and the runtime failed the session with `internal`. `check_action` now returns the new `LoadError::TlsUpgradeOnUdp` with the message "tls_upgrade applies only to TCP flows", at the location of the action. The check runs on every action that `walk` visits, so it holds at any depth of `loop` and `branch`. Three fixtures carry the verdicts for both validators in `engine/fps-flow/tests/fixtures/verdicts.yaml`: `tls_upgrade_tcp` (accept, top level and inside a loop, both sides), `tls_upgrade_udp` (reject), and `tls_upgrade_udp_nested` (reject, a branch arm inside a loop). `tests/fixtures.rs` and `tests/test_fixtures.py` both pass, with the same error pointers. We described the rule in [Engine](../Engine.md#tls-upgrade-of-a-running-session).

### Shared page notes

- Testing: no change. The new fixtures run in the existing `test-eng` target, which runs `cargo test -p fps-flow` and the pytest of `engine/fps-flow/tests`.
- The `internal` reason for a UDP flow in the failure table of ENG-20 stays as a defence for programs that bypass the check, for example a `Program` compiled from a hand built document.
