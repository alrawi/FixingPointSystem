## [2026-11-28] ingest | bug #707: an agent test prepares every file of contracts/examples/tests/

- New test `every_example_test_prepares_on_the_kernel_and_the_unforced_selection` in `engine/fps-agent/tests/fallback.rs`. It iterates `contracts/examples/tests/`, turns each file into a plan (`plan_in` and `modules_of` in `tests/common/mod.rs`, which now read dhcp4 pools, a missing seed, per mix entry pools, module settings, and the first party module tree for `dns/lookup_a` and `http1/single_fetch`), and prepares it on the kernel selection and on the unforced AF_XDP selection of the fake `virtio-guest` host. A file that is not in the `EXPECTED` table must prepare on the kernel and end on AF_XDP without a fallback, so a new example is covered with no change to the test.
- Results: `enterprise_mix_10g` ends on AF_XDP. Five `local/lo` files and `port_locations` run on the kernel fallback with their cause. `dhcp_clients.yaml` is refused on both selections (HST-04 text of `assemble.rs`): TRN-31 blocker, bug #708. No behaviour and no contract changed.

### Shared page notes

- `plan/work-packages.yaml`, TRN-31 (`depends_on`): add AGT-23, and see bug #708 for the decision on `dhcp_clients.yaml`.
