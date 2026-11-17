## [2026-11-17] ingest | CON-34 DHCP configuration of a host pool

CON-34 (issue #598) moves the test configuration from v0.3 to v0.4 and the agent protocol from 0.2.5 to 0.2.6. The vetting agent (Fable 5.2, under D13) specified the change on 2026-11-17. `$defs.host_pool` gains the optional object `dhcp4` (no property yet), allowed only on a client pool without `ipv4`, `ipv6`, and `gateway`. `HostPoolSlice` gains `Dhcp4 dhcp4 = 13`, and plan rule 5 says that host i keeps the MAC address `mac_base` plus i and takes its address from its lease. The example `dhcp_clients.yaml`, negative cases, and the plan checker follow. [Contracts](Contracts.md#dhcp-for-a-host-pool) has the rules, and `contracts/changes/CON-34.md` lists the consumers, HST-04 first.

Decisions.

- `if` and `then` carry the rule that a DHCP pool is a client pool without static addresses, with `false` property schemas, so a validator error names the property that conflicts. The `anyOf` gains a branch, so a pool still needs an address source.
- The plan checker also rejects a `dhcp4` slice that is not a client slice, which the specification of the vetting agent did not name. It mirrors the schema rule, so a plan that no configuration could compile fails early.
- A gateway of a family for which the pool has no prefix is an error that names the family, where it was an "outside" error before.

Findings.

- The validator checked the `gateway` of a pool against both address families, so every dual stack pool with a gateway failed. It now checks the gateway against its own family, as `fps-hosts` matches it. The two new tests fail on the old validator.
- `engine/fps-agent/src/convert.rs` builds `pb::HostPoolSlice` without `..Default::default()`, so adding field 13 stops the whole crate from compiling until the agent consumer lands. `contracts/changes/CON-34.md` lists this and the other failing tests (store example tests, one UI message test, one UI row count, the embedded schema copy test).

### Shared page notes

- Implementation Plan and Work Packages: HST-04 can start once this package merges. The coordinator lands the consumer commit in the pull request (schema copy, `regen-agentpb`, compiler, agent mirrors, UI) as for CON-32.
- Module Development: no change, because no module field, scenario action, or template function changed.
