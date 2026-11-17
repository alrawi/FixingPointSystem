## [2026-11-17] ingest | AGT-22 The agent selects and reports its backend

AGT-22 (issue #575) wires `select_backend` of TRN-14 into `fps-agent` and fills the capability fields of CON-32. The new module `fps_agent::select` builds the `SelectionInput` from `HostCapabilities` (`af_xdp.kernel_support` from the AF_XDP detection, `driver` from the weakest port, the transport facts from `AfXdpFacts::default()` and `KernelFacts::default()`), and maps `BackendKind` and `XdpSupport` to the proto `Backend` and `XdpMode`. `fps-agent connect` selects after the kernel check and before the first dial, and the registration carries `kernel_release`, `selected_backend`, and `xdp_driver_mode`. [Agent](Agent.md#backend-selection) has the rules.

Decisions.

- The operator forces a backend with `--backend kernel|af_xdp` on `connect`. A forced backend that cannot run ends the boot with exit code 2 and the `SelectError` message, before the agent dials, and never falls back. `fps-agent run` takes no flag, because its plan names the backend.
- `xdp_driver_mode` follows the contract text of CON-32: unspecified unless the selected backend is AF_XDP. Then every port reports its own driver table mode (`ZeroCopy`, `Native`, `Generic` for a named driver outside the native table, unspecified for a port without a driver name), while the selection uses the weakest port. The mode that a bound socket reads back is for the queue that binds it (TRN-12).
- A kernel whose AF_XDP support is `Unknown` does not offer AF_XDP to the selection, as `backends` already did not.
- The selection reads only detected facts: no AF_XDP socket opens and no device is probed.
- `fps-agent` now depends on `fps-backend-afxdp` (the lock file gains that edge, no package).
- `fps-agent capabilities` prints `kernel_release`, `selected_backend`, and `selection_reason`.

Findings.

- `assemble` checks `plan.backend`, not the selection. Today AF_XDP is never selected (`TRANSPORT_AVAILABLE` is false), so the two agree. TRN-12 must make the assembly follow the selection in the change that sets the constant, or an agent would report `BACKEND_AF_XDP` and run kernel sockets.
- A bridge or any port without a driver name makes the weakest port `None`, so a host with one keeps AF_XDP out of the default choice. This is deliberate: a port that no table knows cannot be offered an AF_XDP queue. An operator can still force the backend.
- Tests that depend on `TRANSPORT_AVAILABLE` follow the constant (a branch on it), except those on a host without kernel AF_XDP support, which hold either way. The new fixture `testdata/virtio-guest-no-xdp` is a 6.8 guest without `CONFIG_XDP_SOCKETS`, so the boot failure test passes the kernel check.

### Shared page notes

- Testing: add `engine/fps-agent/tests/select.rs` to the agent test list: backend selection on fake system roots and the binary's boot failure for a forced, unsupported backend. No root, no network card, no AF_XDP socket.
- Implementation Plan: AGT-22 is complete; CTL-53 (#597) can read `selected_backend` and `xdp_driver_mode` from the registration of an agent of this version.
- Module Development: no change, because no module field, scenario action, or template function changed, and there is no "Watch out for" entry.
- TRN-12 (note for its handoff): set `fps_backend_afxdp::TRANSPORT_AVAILABLE` and make `fps_agent::assemble` start the selected backend, so the report and the run agree; then report the bound mode in `PortInfo.xdp_driver_mode`.
- Cargo.lock: the only shared file that changed, by the new path dependency of `fps-agent` on `fps-backend-afxdp`.
