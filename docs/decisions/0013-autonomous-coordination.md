# D13. Autonomous coordination with delegated approval

- Status: Accepted (owner, 2026-11-16)
- Date: 2026-11-16
- Deciders: repository owner

## Context

Through Phase 2 the owner approved every batch, every contract (CON) change, and every `needs:owner` decision. On 2026-11-16 the owner wrote to the coordinator:

> can you run autonmously to move forward to completing the project. My goal is to approve all your future batches and deligate desicions to Fable 5.2 with xhigh reasoning to vet your choices. You can also Use Jev (key configured on my system under ~/.config/typesafe/.env. Instead of pausing and telling me what was done and what you want approved, you should move forward autonmously complete the project, write test cases, and automatically fix identified bugs. I will keep my computer on and i want you to run. You should do all testing on my fat nuc not on my laptop.

The same day the owner set up a relay for host tests: a Claude Code session on the owner's laptop that reads job files from a bridge folder, runs each job on fat-nuc over SSH, and writes the results back. The owner confirmed that passwordless sudo works on fat-nuc for these jobs.

This record is accepted on the basis of that instruction. Records that the vetting agent accepts later say "Accepted (vetting agent under D13, <date>)", never "owner".

## Decision

1. **Batches.** The coordinator selects each batch with `plan.py batch` and launches it once the vetting agent approves it. The owner no longer approves batches.
2. **Delegated decisions.** The vetting agent decides the following in place of the owner:
   - additive contract changes,
   - `needs:owner` packages,
   - plan pull requests,
   - technical decision records,
   - and the sign-off of a gate whose suite passes in CI with no waiver and unchanged criteria.

   The vetting agent is Claude Fable 5.2 with extra high reasoning, in a separate read only session that did not write the change. It reads the repository itself and answers in writing, and it may answer "defer to owner". A rejected proposal returns only with changed content. The coordinator records every question and answer in the log entry of the batch, rejections included, with "vetted by Fable 5.2" in place of the owner's name.
3. **Jev.** Before merging, the coordinator runs every pull request diff through an advisory Jev gate. The gate asks for the probability that the diff breaks a fixed rule:
   - a secret on argv or in a log,
   - a self hosted runner on a pull request,
   - an unbounded workflow,
   - a change to the protected host services,
   - raw apt,
   - a weakened test,
   - or a contract change.

   A probability of 0.5 or more on any rule stops the merge until the flagged part is read. On the secret, self hosted runner, and protected host service rules the threshold is 0.2, and the vetting agent reads the flagged part, not the coordinator alone. The gate runs only in the coordinator session. The key stays in that environment and never enters the repository, a workflow, a command line, a log, or fat-nuc. When Jev is unreachable, the reviewer checks the seven rules by hand, and the log says so. Jev never approves anything, and a low probability never replaces CI or review.
4. **Host tests on fat-nuc.** Tests that need a real host, such as KVM, the image boot, AF_XDP, and the benchmarks, run on fat-nuc and never on the owner's laptop. They run through the owner's relay:
   - The coordinator writes a job that names a commit on GitHub and a command.
   - The relay checks the job against its rules, runs it in a separate clone on fat-nuc, and snapshots the protected host services before and after.
   - If a job changed anything other than the network namespaces it created, the relay stops all jobs and alerts the owner.
   - The coordinator sends only commits that passed CI and the Jev gate, and starts no job between 00:50 and 03:30 UTC.

   No workflow that names the self hosted `fps-bench` runner may run on a pull request, a push, a comment, `workflow_run`, or `workflow_call`.

## What stays with the owner

The owner keeps the following, and the coordinator prepares any of them, sets out the decision, and stops:

- D13 itself and any widening of it.
- The specification, the scope, the license, the tier targets, and the gate exit criteria.
- The Gate 3 sign-off, which is the v1 release, and publishing any release or tag.
- Breaking contract changes, and any loosening of `deny.toml`.
- The hard rules of `AGENTS.md`, `CODEOWNERS`, branch protection, and the repository settings and secrets.
- Every change to fat-nuc itself: the Actions runner, Tailscale, the `inet fps_notrack` table, `fps-bench-tune.service`, `nftables.service`, packages, and users. Nothing pushes from the host.
- Approval of workflow runs from forks, and merging pull requests of anyone but the swarm agents.
- The strike module (MOD-27, MOD-28, MOD-29, MOD-32) and the work of the owner's developers.
- Spending money, and hardware the project does not have, such as a host that gives one KVM guest 16 vCPUs for the tier T1 target of Gate 3.

The coordinator also stops and reports to the owner in four cases:

- `main` is red twice in a row.
- A security flag is confirmed.
- The vetting agent answers "defer to owner".
- A test would need a change to fat-nuc itself.

## Consequences

- Work no longer waits for the owner between batches, and the log of each batch is the record of every delegated decision.
- GitHub cannot tell the owner from an agent, because the pull requests are authored and merged under the owner's account and `main` has no branch protection. The log is the only audit trail until the owner adds branch protection and a worker token without the Workflows permission.
- The owner can withdraw the delegation at any time, and can review any delegated decision in the log.
