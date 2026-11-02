# Swarm Protocol

This page defines how independent coding agents find, claim, checkpoint, hand off, and finish work packages. An agent may stop at any moment because it runs out of credits, crashes, or loses its session. The protocol therefore keeps every piece of state on GitHub, and any other agent can resume the work from that state.

## Principles

We build the protocol on five rules.

1. GitHub holds all coordination state. An agent keeps nothing that matters only in its own memory.
2. The issue is the ticket. Labels carry the status, and one status card comment per claim carries the lease.
3. The branch carries the work. Each checkpoint pushes code and the handoff file together.
4. A lease expires on its own. An expired lease lets any agent take over without asking.
5. Every command uses the repository scoped REST API through `gh api`. Agents that run in sandboxes without GraphQL or Projects access can follow the whole protocol.

## Roles

| Role | Who | Responsibilities |
|---|---|---|
| Owner | The human repository owner | Approves contracts, decisions, gates, and catalog changes. Merges pull requests. |
| Worker agent | Any coding agent | Claims one work package at a time, implements it, and hands it off or finishes it. |
| Reviewer agent | Any coding agent that did not write the change | Reviews a pull request against the Definition of Done and posts a structured review comment. |
| Planner agent | Any coding agent asked by the owner | Splits epics, seeds issues from the catalog, and keeps the wiki current. |

## Identity

Each agent session creates an agent identifier at start and uses it in every card, commit, and handoff entry. Several agents may share one GitHub account, so the identifier, and not the GitHub login, names the holder of a lease.

```bash
export FPS_AGENT_ID="claude-$(openssl rand -hex 3)"   # for example claude-7f3a2c
export FPS_REPO=alrawi/FixingPointSystem
```

An agent that resumes its own work after a restart creates a new identifier and follows the takeover procedure. This rule keeps the record simple.

## Labels

Every work package issue carries exactly one status label. The `tools/swarm/bootstrap-labels.sh` script creates the label set.

| Label | Meaning | Next states |
|---|---|---|
| `status:backlog` | Dependencies are open, or the package is an epic | ready |
| `status:ready` | Dependencies are closed, nobody holds a lease | in-progress |
| `status:in-progress` | One agent holds an active lease | review, blocked, ready, stale |
| `status:blocked` | Work cannot continue until a named issue or owner action resolves | ready |
| `status:stale` | The lease expired without a release | in-progress |
| `status:review` | A pull request meets the Definition of Done and waits for review | in-progress, done |
| `status:done` | The pull request merged and the issue closed | none |

Other labels classify the package. These include `type:wp`, `type:bug`, `type:followup`, `ws:<workstream>`, `phase:<n>`, `size:S|M|L`, `critical-path`, `needs:owner`, `needs:split`, `has-handoff`, and `wip` on pull requests.

## The status card

The status card is one issue comment per claim. Its first line is a machine readable marker, and the holder edits the card in place at every checkpoint.

```markdown
<!-- fps:card agent=claude-7f3a2c state=active lease=2026-11-02T09:30:00Z -->
**Status card** for ENG-04, held by `claude-7f3a2c`

| Field | Value |
|---|---|
| State | active |
| Lease expires (UTC) | 2026-11-02T09:30:00Z |
| Progress | 40% (2 of 5 plan steps) |
| Branch | `wp/ENG-04` |
| Last checkpoint | 3f2a9c1 |
| Pull request | #57 |
| Handoff | `docs/handoff/ENG-04.md` |
| Next action | Implement expect min_bytes and its timeout test |
| Takeover of | none |
```

The `state` field takes one of five values. These values are `active`, `released`, `blocked`, `superseded`, and `done`.

The holder of a work package is decided by one rule. Among cards whose state is `active` and whose lease has not expired, the card with the lowest comment id holds the lease. An expired card never holds a lease, and its author may not renew it.

## Lease timing

| Parameter | Value |
|---|---|
| Lease length | 90 minutes from the latest checkpoint |
| Maximum time between checkpoints | 30 minutes of work |
| Claim settle wait | 20 seconds |
| Stale after | Lease expiry; no grace period |

An agent checkpoints before any command that may run longer than ten minutes, because a crash during that command must not lose earlier work.

## Session start

Each session starts with the same sequence of steps.

1. Read [Home](Home.md), the newest entries of [Log](Log.md), and the `AGENTS.md` file at the repository root.
2. Run the promotion sweep. For every `status:backlog` issue whose listed dependencies have all closed, replace the label with `status:ready`. The OPS-03 scanner automates this step once it exists.
3. Run the stale sweep. For every `status:in-progress` issue whose holder card has expired, replace the label with `status:stale`.
4. Pick a work package with the selection rule below.

```bash
# List claimable work packages
gh api -X GET "repos/$FPS_REPO/issues" -f state=open -f labels='type:wp,status:ready' -f per_page=100 \
  --jq '.[] | [.number, .title, ([.labels[].name] | join(","))] | @tsv'
gh api -X GET "repos/$FPS_REPO/issues" -f state=open -f labels='type:wp,status:stale' -f per_page=100 \
  --jq '.[] | [.number, .title] | @tsv'
```

## Selection rule

An agent picks the first package in this order and skips packages outside its competence.

1. `status:stale` packages with the `critical-path` label.
2. `status:ready` packages with the `critical-path` label.
3. Other `status:stale` packages.
4. Other `status:ready` packages.

Within each group, the agent prefers the lowest phase and then the lowest issue number. The agent never claims a package labeled `needs:split` and never holds two leases at once.

## Claim

The agent claims a package in six steps.

1. Confirm that every dependency issue is closed.
2. Post a status card with state `active`, a lease 90 minutes ahead, and `Takeover of` set to `none`.
3. Wait 20 seconds and read every card on the issue.
4. Apply the holder rule. If another card holds the lease, edit the new card to state `superseded` and return to the selection rule.
5. Replace the status label with `status:in-progress` and add the agent's GitHub login as an assignee.
6. Create the branch `wp/<ID>` from `main`, copy the handoff template to `docs/handoff/<ID>.md`, fill in the goal and plan, commit, push, and open a pull request that carries the `wip` label and the text `Closes #<issue>`.

```bash
N=23; ID=ENG-04
LEASE=$(date -u -d '+90 min' +%Y-%m-%dT%H:%M:%SZ)
# Post the card (card.md holds the card text) and keep its id
CARD=$(gh api "repos/$FPS_REPO/issues/$N/comments" -F body=@card.md --jq .id)
sleep 20
gh api "repos/$FPS_REPO/issues/$N/comments" --paginate \
  --jq '.[] | select(.body | startswith("<!-- fps:card")) | [.id, (.body | split("\n")[0])] | @tsv'
# After winning
gh api -X DELETE "repos/$FPS_REPO/issues/$N/labels/status%3Aready"
gh api "repos/$FPS_REPO/issues/$N/labels" -f 'labels[]=status:in-progress'
gh api "repos/$FPS_REPO/issues/$N/assignees" -f "assignees[]=$(gh api user --jq .login)"
git switch -c "wp/$ID" origin/main
cp docs/handoff/TEMPLATE.md "docs/handoff/$ID.md"   # then fill it in
git add "docs/handoff/$ID.md" && git commit -m "Start $ID handoff" && git push -u origin "wp/$ID"
gh api "repos/$FPS_REPO/pulls" -f title="[$ID] Flow program interpreter" -f head="wp/$ID" \
  -f base=main -f body="Closes #$N. Work in progress; see docs/handoff/$ID.md." --jq .number
gh api "repos/$FPS_REPO/issues/<pr-number>/labels" -f 'labels[]=wip'
```

We open the pull request at claim time, because a visible pull request shows reviewers and successors the work as it grows. The `wip` label tells the owner not to merge it. GitHub's REST API cannot convert a draft pull request to ready, so we do not use drafts.

## Checkpoint

A checkpoint makes the current state recoverable. The agent performs five steps.

1. Run the fast tests of the workstream and record the result in the handoff file.
2. Update the plan checkboxes, progress, next action, and session log in `docs/handoff/<ID>.md`.
3. Read the cards on the issue and confirm that the agent's card still holds the lease. If it does not, stop work, push nothing, and post a comment that names the unpushed state.
4. Commit and push. A checkpoint commit may contain unfinished code when the handoff file says so.
5. Edit the status card with the new lease, progress, last commit, and next action.

```bash
git add -A && git commit -m "Checkpoint $ID: expect modes and timeouts" \
  -m "Work-Package: $ID" -m "Agent: $FPS_AGENT_ID"
git push
gh api -X PATCH "repos/$FPS_REPO/issues/comments/$CARD" -F body=@card.md
```

An agent never force pushes a `wp/` branch. A rejected push means that another agent advanced the branch, and the agent must stop and read the cards.

## Progress percentage

Progress equals the number of checked plan steps divided by the total number of plan steps. Each plan step must take at most 30 minutes of work, so the percentage stays meaningful. An agent that splits a step updates the total and records the change in the session log. Agents do not estimate progress in any other way.

## Release

An agent that must stop on purpose releases the lease. Typical reasons are a low remaining budget, the end of an assigned session, or a package that exceeds the agent's competence.

1. Checkpoint.
2. Set the handoff status to `paused` and write the next action as a single concrete step.
3. Edit the card to state `released`.
4. Replace `status:in-progress` with `status:ready` and add `has-handoff`. Remove the assignee.

## Block

An agent that cannot continue because of a missing decision, a contract gap, or an open dependency blocks the package.

1. Checkpoint.
2. Post a comment that names the blocker, links the blocking issue or opens one, and states what would unblock the work.
3. Edit the card to state `blocked` and replace the status label with `status:blocked`. Add `needs:owner` when only the owner can resolve the blocker.

Whoever resolves the blocker relabels the issue `status:ready`. The next agent resumes from the handoff file.

## Takeover

Any agent may take over a package whose holder card has expired. The previous agent may have crashed, so the takeover never waits for permission.

1. Post a new card with state `active` and `Takeover of` set to the previous agent identifier.
2. Wait 20 seconds and apply the holder rule.
3. Replace `status:stale` or `status:ready` with `status:in-progress`.
4. Fetch `wp/<ID>`, read `docs/handoff/<ID>.md`, and run its verification commands.
5. Append a session log entry that records the takeover, the commit it started from, and the verification result.
6. When verification fails, fix the branch forward from the last commit that the handoff marks as verified. Do not rewrite history.

An agent that returns after its lease expired must treat its own package like anybody else's. It reads the cards, and when another card holds the lease it edits its old card to `superseded` and pushes nothing.

## Finish

An agent finishes a package when the [Definition of Done](Implementation-Plan.md#definition-of-done) holds.

1. Set the handoff status to `ready for review` and progress to 100%.
2. Push, then remove the `wip` label from the pull request.
3. Replace `status:in-progress` with `status:review` and edit the card to state `done`.
4. Ingest the result into the wiki as [Wiki Maintenance](Wiki-Maintenance.md) describes, in the same pull request.

A reviewer agent then reviews the pull request. The owner merges it, and the `Closes #<issue>` text closes the issue. When review requests changes, any agent may claim the package again from `status:review` with a new card.

## Review

A reviewer agent never reviews its own change. It checks the pull request against the Definition of Done and posts one comment with this structure.

```markdown
<!-- fps:review agent=claude-91bd04 verdict=approve -->
**Review** of ENG-04 at 3f2a9c1

| Check | Result | Note |
|---|---|---|
| Acceptance criteria | pass | All four boxes verified locally |
| Tests added or updated | pass | 14 new unit tests, 1 regression test |
| Coding standards | pass | |
| Contracts untouched | pass | |
| Handoff complete | pass | |

Verdict: approve
```

The verdict is `approve` or `changes`. A `changes` verdict lists each required change as a checkbox.

## Creating tickets

Agents create tickets with the REST API and the templates in `.github/ISSUE_TEMPLATE/`. Every new issue is added to the Project board automatically, as [Project Board Setup](Project-Board-Setup.md) describes.

| Ticket | When | Labels | Ready immediately |
|---|---|---|---|
| Work package from the catalog | A planner seeds a phase with `tools/plan/plan.py seed --phase N --apply` | Set by the seeder | When its dependencies are closed |
| Split of an epic or an oversized package | A package needs more than one session | `type:wp`, `needs:owner` | After a plan pull request adds the children to the catalog |
| Bug | A test or a review finds a defect outside the current package | `type:bug`, `ws:<workstream>`, `status:ready` | Yes |
| Follow up | Work that the current package should not absorb | `type:followup`, `ws:<workstream>`, `status:backlog` | After triage by the owner or a planner |
| Contract change | An implementation needs a contract change | `type:wp`, `ws:con`, `needs:owner` | After owner approval |

```bash
gh api "repos/$FPS_REPO/issues" -f title="[bug] Interpreter drops the last byte of a chunked body" \
  -F body=@bug.md -f 'labels[]=type:bug' -f 'labels[]=ws:eng' -f 'labels[]=status:ready' --jq .number
```

An agent fixes a defect inside its own package without a ticket. It creates a ticket for anything outside its paths and leaves that work to another agent.

## Guarantees and limits

The protocol bounds lost work to one checkpoint interval, which is at most 30 minutes. Two agents that claim within the same settle window resolve to one holder through the lowest comment id. A returning agent cannot overwrite a successor, because it must hold the lease before it pushes, and Git rejects a push onto a branch that moved.

The protocol does not prevent an agent from ignoring it. The owner reviews every merge, and the review checklist verifies the handoff file and the card history.
