## [2026-11-03] ingest | TRN-03 Operating system profiles for the kernel backend

We added the operating system profiles windows11, linux6, and macos14 to the kernel backend in `engine/fps-backend-kernel` and documented them in the section [Operating system profiles](Transport.md#operating-system-profiles) of the Transport page. The profiles are YAML files under `profiles/` that `OsProfile` loads and validates. The backend applies the TTL, MSS, and window clamp of a profile per socket through three new `SocketOptions` fields, and each profile lists the SACK, timestamp, window scaling, and receive buffer sysctls that Linux offers only per network namespace. The limits table names every option that the backend cannot set per socket, among them the TCP option order and the exact SYN window. The capture test `tests/netns/profiles.sh` runs on the QA-01 harness with an MTU of 9000 and shows the TTL, MSS, and window scale of each profile in the SYN and the SYN-ACK, and a baseline session without a profile shows the kernel defaults. The new workflow `.github/workflows/trn.yml` runs it with sudo, and the new area file `mk/trn.mk` adds the targets `lint-trn`, `test-trn`, `build-trn-probe`, and `e2e-trn`. The crate now depends on `serde` and `serde_norway`, which the workspace already used. The evidence is `docs/handoff/TRN-03.md` on the branch `wp/TRN-03`.

## Shared page notes

The coordinator may apply the notes below.

1. [Testing](Testing.md) lists the make targets per area. It may add `lint-trn`, `test-trn`, and the root only `e2e-trn`, which needs `make build-trn-probe` first, and name `trn.yml` among the workstream workflows.
2. [Implementation Plan](Implementation-Plan.md) may mark TRN-03 as done after the merge. RLM-09 can read the `reference` section of each profile file for its transport behavior check.
