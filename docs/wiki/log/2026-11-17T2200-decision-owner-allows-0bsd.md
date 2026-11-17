## [2026-11-17] decision | The owner allows the license 0BSD for Rust dependencies

On 2026-11-17 the owner approved the license `0BSD` for Rust dependencies. D13 keeps every change to `deny.toml` with the owner. `0BSD` is a permissive license with no conditions and is compatible with GPL-3.0. The license policy for modules and Go dependencies in [Module Development](../Module-Development.md) and [Controller](../Controller.md) already lists it, and `deny.toml` now lists it too.

This unblocks TRN-20, the smoltcp adapter on the TCP stack bench harness. Its working adapter waits on the branch `wp/TRN-20`. TRN-20 leads to the stack decision TRN-10, and then to TRN-12 and AGT-23.

In the same message the owner named fat-nuc as the 16 vCPU host for Gate 3, and asked for the specification of the strike module as an open issue for the owner's developer. The log entries that follow record what the coordinator did with those.

### Shared page notes

None.
