## [2026-11-17] plan | Batch 37 merged, and the first merges of batch 38

The coordinator merged these pull requests after the batch 36 wrap-up in PR #587. Each passed all of CI, and Jev scored each diff below 0.5 on every rule and below 0.2 on the secret, self hosted runner, and protected host rules, except where noted:

- UI-07 in PR #584: the web UI end to end suite, with the `controller-browser` job against a real controller. Jev scored 0.47 on the secret rule for the fixed tokens of the test controller, and the vetting agent approved the merge.
- CON-32 in PR #590: kernel release, selected backend, and XDP driver mode in the agent capabilities, with its consumers in the same pull request.
- Bug #588 in PR #589: a transport worker takes one turn before it honors `stop`.
- TRN-17 in PR #586: the TCP stack bench harness with a kernel reference stack. The smoltcp adapter moved to TRN-20, because `cargo deny` rejects its `0BSD` license and D13 keeps `deny.toml` with the owner.
- HST-03 in PR #591: ARP and ND for user space backends.
- CTL-51 in PR #599: clock offsets of agents from their heartbeats.
- TRN-14 in PR #593: backend selection at boot. Generic only XDP never wins on its own, which is policy and not a measurement.
- TRN-18 in PR #601: a minimal own TCP stack on the bench harness. Jev scored 0.20 on the secret rule for the field `isn_secret`, a non cryptographic seed of the initial sequence numbers. Vetted by Fable 5.2: approved without change. Renaming it to `isn_seed` is optional and can come with TRN-12.
- CTL-52 in PR #602: runs on two or more agents check the clocks, start at one instant through the coordinator, and report exact totals. A clock refusal answers 409. The coordinator also corrected the doc comment of `internal/multiagent`, which still said that `ClockSource` had no implementation.
- Bug #592 in PR #604: the run feed reads the gap on the first event of every connection, the mock stream sends every tick, and the reconnect test waits for the gap read. The coordinator added the [Testing](../Testing.md) paragraph on streams and history reads.

Open points from these packages:

- A multi agent run turns `running` when the agents accept the start, up to `StartLead` before traffic starts, so its `started_at` precedes the start instant (CTL-52).
- `runs.Run.Totals` is still a map of floats, so a total above 2^53 rounds, as before (CTL-52).
- The TRN-18 host test on fat-nuc waits for the relay account to build Rust, which is an owner action.

CON-34 in PR #603 is open. The coordinator landed the schema copy, the UI types, and the regenerated `agentpb`, and a worker is landing the compiler, store, agent, and UI changes. AGT-22, CTL-53, and HST-04 follow it.

### Shared page notes

Applied by the coordinator in the same pull request:

- [Testing](../Testing.md): the paragraph "Since batch 36".
- [Module Development](../Module-Development.md), "Watch out for", "Native code": the entry "clippy follows the stable toolchain too".
