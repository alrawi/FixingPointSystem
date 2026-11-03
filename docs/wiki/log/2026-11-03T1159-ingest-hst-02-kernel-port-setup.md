## [2026-11-03] ingest | HST-02 Kernel port setup

We added kernel port setup to the crate `fps-hosts` and a section on it at the end of the [Hosts](Hosts.md) page. Setup moves a test interface into the namespace `fps-port-<name>`, applies a per namespace sysctl profile, adds an optional VLAN subinterface and interface address, adds `local` routes that cover exactly the addresses of the slice, and adds the default route through the gateway. It refuses any interface that looks like a management interface before it changes anything. We drive iproute2 through a narrow command wrapper instead of the `rtnetlink` crate, because namespace work needs unsafe code that the crate forbids, and the page records the reasons. An integration test in the QA-01 harness opens 20,000 TCP sessions from 20,000 pool addresses, and every test checks that teardown leaves no namespace, route, or link behind. The new file `mk/hst.mk` adds the root target `e2e-hst`, and the workflow `.github/workflows/hst.yml` runs it as root. The evidence is `make e2e-hst` and `docs/handoff/HST-02.md` on the branch `wp/HST-02`.

### Shared page notes

- [Testing](Testing.md) could add the row `e2e-hst` to the target table, which runs the kernel port integration tests in the namespace harness and needs root.
- [Home](Home.md) and the sidebar already link the Hosts page, so no link change is needed.
- AGT packages that start the kernel backend can call `fps_hosts::port::setup` once per port and run their sockets in the returned namespace, and `FakeIp` serves their component tests.
