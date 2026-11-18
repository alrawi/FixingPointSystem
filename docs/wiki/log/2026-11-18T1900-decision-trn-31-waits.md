## [2026-11-18] decision | The AF_XDP default waits for a kernel fallback, module coverage, and a KVM measurement

AGT-28 merged, and TRN-31, the flip that makes AF_XDP with the own stack the default on every host with native XDP, had no open dependency left. Under D13 the coordinator asked the vetting agent whether to dispatch it. The vetting agent decided not to dispatch it yet. TRN-34 must land first, but it is not the main blocker: after the flip every agent on virtio_net or veth would select AF_XDP, and `check_af_xdp` in `engine/fps-agent/src/backend.rs` refuses every plan outside one shape (app_sim only, both sides in one process, one client and one server pool on two interfaces, one family, one VLAN, no DHCP, and OS profiles the own stack knows). `http_smoke.yaml` and `enterprise_mix_10g.yaml` would fail on every KVM guest, and only http1 has an AF_XDP realism job.

The coordinator changed the plan as the vetting agent asked:

- AGT-29: an unforced agent runs a plan that the datapath refuses on the kernel backend and reports that; a forced agent refuses as today. A pool with both prefixes falls back too.
- CON-35 now carries the backend that ran as well as the bound mode, and CTL-55 stores and shows them, so a benchmark never measures the wrong backend unseen.
- TRN-36: an AF_XDP realism job for every first party module, sip_rtp included, or a written exclusion with its reason.
- AGT-30: datapath lanes that follow the workers of the plan, instead of one lane on queue 0.
- QA-06 measures both backends, each forced, on a KVM guest, and no longer depends on TRN-31. The host is an owner action, and the default switch waits for it.
- AGT-23 (DHCP leases) depends on AGT-26 instead of TRN-31, because a forced AF_XDP run already works; DHCP need not come before the flip, because the kernel backend refuses it too.
- TRN-31 depends on all of the above and on TRN-34, adds a test for each refused shape, and adds the acceptance that every file in `contracts/examples/tests/` runs on an unforced agent on a veth pair after the flip.

### Shared page notes

Applied by the coordinator in the same pull request: the catalog entries TRN-31, QA-06, AGT-23, CON-35, AGT-29, AGT-30, CTL-55, and TRN-36.
