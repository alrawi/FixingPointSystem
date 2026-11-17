## [2026-11-17] plan | CON-34 merged, and HST-04 is ready

CON-34 merged in PR #603 with its consumers: the store copy of the schema, the UI types, the regenerated `agentpb`, the compiler, store fixtures, the agent, and the UI. The vetting agent accepted it as an additive contract change under [D13](../../decisions/0013-autonomous-coordination.md). Jev scored 0.98 on the contract rule, as every contract change does, and 0.21 on the weakened test rule for `engine/fps-hosts/tests/contract_examples.rs`, which now skips a pool with `dhcp4` because such a pool has no prefix to map. Vetted by Fable 5.2: approve, since every static pool is still checked.

The vetting agent added one condition. Until HST-04 lands, the agent refuses a DHCP pool at Prepare, so the capture filter of a DHCP port, which keeps every session, cannot be reached. HST-04 should give the agent the leased addresses so that the filter can use them. The coordinator therefore:

- removed the owner action of HST-04, which waited for CON-34,
- added that condition to the acceptance of HST-04,
- and returned TRN-10 (#561) to the backlog, because the promotion sweep read a stale list of dependencies in its issue that lacked TRN-20.

The `contracts/README.md` table still names OpenAPI 0.12.0, while the controller reports 0.13.0 since CON-32. The next contract package corrects it.

### Shared page notes

None.
