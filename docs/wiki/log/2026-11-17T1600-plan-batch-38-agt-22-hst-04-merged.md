## [2026-11-17] plan | AGT-22 and HST-04 merged, AGT-23 added

The owner paused development for the night on 2026-11-16 and resumed it on 2026-11-17. The coordinator merged two pull requests of batch 38, each after all of CI passed and Jev scored every rule below 0.2:

- AGT-22 in PR #607: the agent builds the selection input of TRN-14 from its host capabilities, where the weakest port decides. It reports `kernel_release`, `selected_backend`, and `xdp_driver_mode`, and a forced backend that the host cannot run stops boot with exit 2. CTL-53 (#597) now runs.
- HST-04 in PR #608: a DHCPv4 client per host for user space backends, with a lease table that the agent can read. The harness test ran as root in CI.

The coordinator applied the notes of both packages to the [Testing](../Testing.md) page and added two items to the plan:

- AGT-23: the agent leases the addresses of a DHCP pool. It follows the steps that the HST-04 log entry lists, and meets the condition the vetting agent set on CON-34: a DHCP port is filtered by its leased addresses. It depends on HST-04 and on TRN-12, which opens the port queues.
- A deliverable of TRN-12, from AGT-22: the change that makes the AF_XDP transport available also makes `assemble` start the selected backend and report the bound XDP mode, so that the report and the run agree.

From this day the coordinator paces its work by usage, at the owner's request, so that no Claude Code instance runs out of usage in the middle of a task. Before it launches an agent it checks a usage ledger. After a usage limit it waits until the reset time and then resumes the stopped agents, whose work is saved. The ledger and its rules live in the coordinator session and not in the repository.

### Shared page notes

Applied by the coordinator in the same pull request: [Testing](../Testing.md), and the catalog entries AGT-23 and TRN-12.
