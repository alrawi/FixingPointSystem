## [2026-11-05] ingest | AGT-06 Kernel requirement and redial policy

We applied decision D12 items 3 and 11 to the agent. `fps-agent connect` and `fps-agent run` read the kernel release through the injectable `SystemRoot` and refuse a kernel before Linux 6.3 with an error that names `IP_LOCAL_PORT_RANGE`. The redial backoff now carries jitter, restarts at 200 ms after an abort or after a session that opened, and a new flag `--redial-limit SECONDS`, which defaults to 300, ends the agent with exit code 3 after that much failed redialing, where 0 means no limit. We added the sections Kernel requirement and Redial policy to [Agent](Agent.md), a crate README, and the target `test-agt-redial` in `mk/agt.mk`. The evidence is `docs/handoff/AGT-06.md`, `engine/fps-agent/tests/detect.rs`, `tests/redial.rs`, and `tests/exit_codes.rs`.

### Shared page notes

- [Testing](Testing.md): the row `test-agt` could add `tests/redial.rs` and `tests/exit_codes.rs`, and a row could list the new target `test-agt-redial`.
- [Implementation Plan](Implementation-Plan.md): any seam table that names the kernel version should say that the agent now requires Linux 6.3 or newer.
