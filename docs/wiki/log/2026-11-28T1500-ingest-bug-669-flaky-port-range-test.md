## [2026-11-28] ingest | bug 669 Flaky port range test in the kernel backend

Issue #669: `port_range_limits_sessions_per_destination_and_shares_ports_across_destinations` (fps-backend-kernel, loopback.rs) failed once in CI with EADDRNOTAVAIL on local port 41003.

Cause. A test bug. The test needs four exact local ports and used 41000 to 41003, which lie inside the kernel ephemeral range (32768 to 60999). Any other socket on the machine (a parallel test thread, another test binary, an outgoing connection) can hold one of those ports, and then `connect` from that port fails with EADDRNOTAVAIL. Reproduced deterministically by holding a wildcard listener on 41003: the test fails the same way.

Fix. The kernel ignores an `IP_LOCAL_PORT_RANGE` outside `ip_local_port_range`, so the block must stay inside the ephemeral range (a first attempt below it failed 92 of 92 runs). The test now reads the range at run time, picks a time-based block away from its edges, and uses the first block whose four ports all bind (`free_port_block`). A port can still be taken between the probe and the connect, so the scenario reruns with a new block (up to 5 attempts) when a connect that must succeed gets EADDRNOTAVAIL; all assertions must hold in the final run. The assertions are unchanged.

Not changed: `loopback.rs` still has two ranges inside the ephemeral range (40000-44999 and 42000-42099). They are wide and only need some free ports, so they do not fail on a single taken port; a successor can move them if they ever flake.

### Shared page notes

- Module Development, "Watch out for": a test that needs exact local ports in a `IP_LOCAL_PORT_RANGE` must pick a free block inside `ip_local_port_range` at run time (the kernel ignores a range outside it); a fixed block can be held by any socket and gives EADDRNOTAVAIL (bug 669).
- Testing page: same advice for test authors.
