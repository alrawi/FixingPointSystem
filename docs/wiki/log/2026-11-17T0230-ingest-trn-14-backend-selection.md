## [2026-11-17] ingest | TRN-14 Backend selection at boot

TRN-14 (issue #566) adds `select_backend` to `fps-backend-afxdp`: a function over its own input type, `SelectionInput`, that picks the kernel or the AF_XDP backend from detected facts, with a forced override and a typed error for a forced backend that cannot run. `HostCapabilities` lives in `fps-agent`, which depends on the backends, so the input is a type of the backend crate and AGT-22 builds it from the detection. [Transport](Transport.md#backend-selection) has the rules.

Decisions.

- The default stays the kernel backend. AF_XDP wins only with kernel support, a transport that carries TCP over AF_XDP, and a driver that runs XDP natively (copy mode or zero copy). Generic XDP is the slowest mode and never wins by itself, but an operator can force it.
- Each backend crate has a `TRANSPORT_AVAILABLE` constant. The kernel one is true. The AF_XDP one is false until TRN-12, because nothing carries TCP over AF_XDP before it. The result today is the kernel backend on every host, and a forced AF_XDP run fails with `Unavailable::NoTransport`.
- A forced backend never falls back to another one, so a benchmark never measures the wrong backend.
- `XdpSupport` (`None`, `Generic`, `Native`, `ZeroCopy`) mirrors the contract `XdpMode` that CON-32 added. The selection result carries the expected mode, and AGT-22 must still report the mode that the socket reads back at run time in `PortInfo.xdp_driver_mode`.

Findings.

- The selection puts an operator's `forced` value and a measured `driver` mode in one input. The contract says a reader of `XdpMode` must not select a backend from it, which holds: the agent that measured the mode is the one that selects, and the controller does not.
- virtio-net offers native XDP with copy mode sockets on current guest kernels and zero copy only on kernels that support it (TRN-19 measures it on KVM), so the test inputs for a virtio-net guest use `Native` for "without zero copy support" and `ZeroCopy` for "with".

### Shared page notes

AGT-22 (the agent wiring): build `SelectionInput` from `HostCapabilities` (`af_xdp.kernel_support` from the AF_XDP detection, `driver` from the port probe, the weakest port deciding), take the transport facts from `AfXdpFacts::default()` and `KernelFacts::default()`, map `BackendKind` to `Backend` for `Capabilities.selected_backend`, and fail the boot on `SelectError` when a backend is forced. TRN-12 must set `fps_backend_afxdp::TRANSPORT_AVAILABLE` to `true` in the same change that lets a TCP stack run over `FrameIo`, and then change the test `the_transport_constant_decides_whether_a_capable_host_selects_af_xdp` only if its meaning changed.
