## [2026-11-16] decision | D13, autonomous coordination, and the Phase 3 plan

On 2026-11-16 the owner asked the coordinator to run on its own until the project is complete, approving its own batches and delegating the owner's decisions to a vetting agent. [D13](../../decisions/0013-autonomous-coordination.md) records that delegation and its limits. The coordinator put five questions to the vetting agent, and all five answers below are vetted by Fable 5.2.

The questions and answers:

1. **The D13 draft.** Approved with changes. The vetting agent replaced "any decision the protocol gives to the owner" with a closed list, required a separate read only vetting session that may defer to the owner, kept the Jev key out of the repository, CI, and fat-nuc, and lowered the Jev stop to 0.2 for the secret, self hosted runner, and protected host rules. It also widened what stays with the owner: D13 itself, the specification and gate criteria, the Gate 3 sign-off, breaking contract changes, `deny.toml`, the hard rules, every change to fat-nuc, fork approvals, and spending. The same pull request amends `AGENTS.md`, `CONTRIBUTING.md`, the [Swarm Protocol](../Swarm-Protocol.md), and the [Implementation Plan](../Implementation-Plan.md).
2. **Host tests on fat-nuc through a self hosted workflow.** The vetting agent approved a dispatch only workflow without sudo. The owner then chose a relay instead, a Claude Code session on the owner's laptop that runs jobs on fat-nuc over SSH with passwordless sudo, and the owner's choice stands. [Operations](../Operations.md#owner-host) describes the relay and keeps the vetting agent's rules for any workflow that names the runner. OPS-08 keeps the CI check that the vetting agent asked for, so a self hosted label can never meet an untrusted trigger.
3. **Phase 3 readiness.** Approved with changes before seeding:
   - TRN-10 becomes size S, writes decision record 0014 (0012 is D12), and depends on two new measurement packages, TRN-17 for smoltcp and TRN-18 for an own stack.
   - TRN-11 no longer depends on TRN-10 and tests AF_XDP on a veth pair in CI. Its rate test moves to TRN-19 with a 20% tolerance, the noise S3 recorded.
   - DHCP moves from HST-03 to HST-04, which waits for a schema field.
   - TRN-13 waits for an SR-IOV or bare metal host.
   - AGT-22 wires TRN-14 into the agent.
   - QA-06 is scoped to KVM.
   - QA-07 waits for the owner to name the reference KVM host.

   fat-nuc has 8 vCPUs, so it cannot host the 16 vCPU guest of tier T1, and that blocker goes to the owner.
4. **Open issues.**
   - #482 becomes CON-33, a fix inside the examples with no schema change.
   - #89 closes, because CON-23 already added `realism.parser_errors`.
   - #74 becomes CON-32 in part (kernel release, selected backend, XDP driver mode), and the NUMA fields stay open.
5. **Limits and risks.**
   - The pull requests are authored and merged under the owner's account and `main` has no branch protection, so the log is the audit trail.
   - A pushed workflow that names the runner on `push` would run before review.
   - Fork runs are never approved.
   - The coordinator stops on a red `main` twice in a row, a confirmed security flag, a deferral, or any need to change fat-nuc.

The coordinator also added RLM-24 (a reused TCP client port starts a new session), the cause of the doubled HTTP/2 message counts of batch 35.

### Shared page notes

None.
