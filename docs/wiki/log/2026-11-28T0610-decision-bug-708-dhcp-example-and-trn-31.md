## [2026-11-28] decision | bug #708: the DHCP example, TRN-31, and two new packages

- The vetting agent decided under D13 (read at main 060ecc5): `contracts/examples/tests/dhcp_clients.yaml` gets fixed, not exempted. New CON-36 gives each arm its own interface and the server pool a gateway. New TRN-37 adds DHCP leases on the datapath (fps-datapath answered DHCP frames with `Unsupported`, and the own stacks read addresses from the pool prefix, a gap no package owned). AGT-23 is rewritten for the datapath and depends on both. DHCP stays a user space feature in v1, and the kernel backend keeps refusing it.
- TRN-31 depends on AGT-23 now. Its acceptance says that an unforced agent runs every example and refuses none. A file whose shape the datapath serves runs on AF_XDP; any other runs on the kernel fallback with the backend and cause reported (CON-35), which counts as run. That covers the five `local/lo` examples and `port_locations.yaml`.
- Paths outside a package's workstream (the `fps-agent` tests that pin `TRANSPORT_AVAILABLE` for TRN-31, the compiler golden for CON-36) are made by the coordinator in the same pull request, like the workflow changes of TRN-31.
- For the owner, not a blocker: after the flip every plan with more than one client or server pool runs on the kernel backend, because the datapath serves one pool per side. Lifting that is a separate, undecided package.
- Contingency: if QA-06 and every other TRN-31 dependency are done while TRN-37 or AGT-23 is still open, the coordinator returns to the vetting agent, and a named exception for dhcp_clients.yaml is then acceptable.

### Shared page notes

None.
