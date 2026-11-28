## [2026-11-28] ingest | bug 727 The opener may report holes when the link drops frames

Issue #727: the row "kernel client to own server, 1% loss and 20 ms, loss switches on" of `interop_kernel` failed once with "the opener has no holes to report".

Cause. The own acceptor answers with 8 bytes and a FIN, and the FIN always leaves in a segment of its own, so two segments take up sequence space. At 1 percent loss the answer can be lost while the FIN arrives (a hole) or arrive twice after a retransmission (a duplicate report), and the kernel opener then sends SACK blocks. The own server sent nothing it should not. Shown from the code and the send path; the sandbox has no netem, so there is no capture.

Fix. `wiretap::violations` keeps `opener.sack_segments == 0` on a link without loss and, with loss, bounds it by the acceptor's data segments plus two. The check is not dropped.

Evidence. The unit test of the bound. The 50 run loop could not run in the sandbox (`tc qdisc add ... netem`: unknown qdisc kind); the PR's CI runs the row.

### Shared page notes

- Testing: none. Module Development: none.
