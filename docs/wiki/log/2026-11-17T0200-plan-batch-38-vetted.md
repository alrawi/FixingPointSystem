## [2026-11-17] plan | Batch 38 vetted, and the CON-34 specification

The coordinator put three questions to the vetting agent under [D13](../../decisions/0013-autonomous-coordination.md). All three answers are vetted by Fable 5.2.

1. **CON-34.** Specified as an additive change.
   - The test configuration schema goes from v0.3 to v0.4. A client host pool gains an optional `dhcp4` object, with no ipv4, ipv6, or gateway alongside it.
   - The agent protocol goes from 0.2.5 to 0.2.6, with `message Dhcp4` as field 13 of `HostPoolSlice` and plan rule 5 for DHCP hosts.
   - The package adds an example and negative cases.
   - It also fixes the validator, which checks a gateway against both address families, so every dual stack pool with a gateway failed.
   - The catalog paths gain `tools/contracts/`, because the plan checker demands a prefix.
   - The coordinator lands the consumers in the same pull request, as for CON-32.
2. **TRN-14's rule.** Approved: generic only XDP never wins on its own. The text now states it as policy, since spike S3 measured only native copy and zero copy.
3. **Batch 38.** Approved, in this order:
   - CON-34 first, because its consumer commit touches the paths of AGT-22 and CTL-53.
   - Then AGT-22 after TRN-14, CTL-52 after CTL-51, and CTL-53 after AGT-22.
   - TRN-19 is blocked until the owner's relay can build Rust on fat-nuc.

### Shared page notes

None.
