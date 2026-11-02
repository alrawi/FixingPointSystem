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

## The helper command

We wrap every step of this protocol in one command, [`tools/swarm/fps_swarm.py`](https://github.com/alrawi/FixingPointSystem/blob/main/tools/swarm/fps_swarm.py). It needs Python 3.10 or newer and the `gh` command line client, and it calls only the repository scoped REST API through `gh api`. Each section below shows the helper command first and the raw calls after it. The raw calls stay authoritative, so an agent without Python can still follow the protocol.

| Command | Protocol step |
|---|---|
| `fps_swarm.py now` | Print GitHub server time |
| `fps_swarm.py list` | List claimable items in selection order |
| `fps_swarm.py claim N --steps S` | Claim steps 1 to 5, including the settle wait |
| `fps_swarm.py takeover N --steps S` | Claim an item whose newest card expired, was released, or was blocked |
| `fps_swarm.py heartbeat N --check` | Confirm that our card holds the lease with 10 minutes left |
| `fps_swarm.py heartbeat N --progress P --commit C --pr R --next X` | Checkpoint step 5, which renews the lease and edits the card |
| `fps_swarm.py release N --next X` | Release steps 3 and 4 |
| `fps_swarm.py block N --reason X --blocked-by M` | Block steps 2 and 3 |
| `fps_swarm.py review N --pr R` | Finish steps 4 and 5 |
| `fps_swarm.py promote`, `stale`, or `sweep` | The promotion sweep, the stale sweep, or both, each with `--dry-run`; the [scanner](#the-scanner) runs `sweep` every 30 minutes |

The command reads the agent identifier from `FPS_AGENT_ID` and the repository from `FPS_REPO`, and the flags `--agent` and `--repo` override them. It exits with 0 on success, with 1 when the protocol refuses the step, for example because another card holds the lease, and with 2 on a usage or API error. Git work stays with the agent, so `claim` prints the card id, key, branch, and handoff path that the next steps need. The tests in `tools/swarm/` replay recorded API responses and never reach the network.

```bash
export FPS_AGENT_ID="claude-$(openssl rand -hex 3)" FPS_REPO=alrawi/FixingPointSystem
swarm() { python3 tools/swarm/fps_swarm.py "$@"; }   # a function also works in scripts
```

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
| Minimum lease left before a push | 10 minutes |
| Stale after | Lease expiry; no grace period |

An agent checkpoints before any command that may run longer than ten minutes, because a crash during that command must not lose earlier work.

Agents compute every lease time from GitHub server time, because local clocks differ between sandboxes. The `Date` header of any API response gives that time.

```bash
now_utc() { gh api -i "repos/$FPS_REPO" 2>/dev/null | sed -n 's/^[Dd]ate: //p' | head -1 \
  | python3 -c 'import sys,email.utils as e; print(e.parsedate_to_datetime(sys.stdin.read()).strftime("%Y-%m-%dT%H:%M:%SZ"))'; }
lease_in() { python3 -c 'import sys,datetime as d; t=d.datetime.strptime(sys.argv[1],"%Y-%m-%dT%H:%M:%SZ"); print((t+d.timedelta(minutes=int(sys.argv[2]))).strftime("%Y-%m-%dT%H:%M:%SZ"))' "$(now_utc)" "$1"; }
LEASE=$(lease_in 90)
```

## Work items and naming

Agents claim two kinds of issues. Both follow the same protocol and differ only in naming.

| Item | Issue labels | Key | Branch | Handoff file | Allowed paths |
|---|---|---|---|---|---|
| Work package | `type:wp` | catalog id, for example `ENG-04` | `wp/ENG-04` | `docs/handoff/ENG-04.md` | The package's `paths` in the catalog |
| Bug | `type:bug` | `bug-<issue number>`, for example `bug-142` | `bug/142` | `docs/handoff/bug-142.md` | The paths of the workstream in the bug's `ws:` label |

Every pull request may also change `docs/wiki/` for the ingest step. Follow up issues are not claimable until triage turns them into work packages or bugs.

## Claimable status

An issue is claimable when it is open, carries `status:ready` or `status:stale`, and carries neither `needs:split` nor `type:followup`. The label `needs:owner` does not prevent a claim. It means that the owner must act before the package can finish, for example by approving a contract or creating a secret, and the agent prepares everything up to that point and then blocks the package.

## Session start

Each session starts with the same sequence of steps.

1. Read [Home](Home.md), the newest entries of [Log](Log.md), and the `AGENTS.md` file at the repository root.
2. Run the promotion sweep below. The [scanner](#the-scanner) also runs it every 30 minutes, so this step usually finds nothing to change.
3. Run the stale sweep. For every `status:in-progress` issue whose holder card has expired, replace the label with `status:stale` and post one stale comment that names the expiry time.
4. Pick an item with the selection rule below.

The helper runs both sweeps and lists claimable items in two commands.

```bash
swarm sweep            # promotion sweep, then stale sweep; add --dry-run to only print changes
swarm list             # claimable items in selection order, tab separated
```

The promotion sweep applies three rules. A label change that returns 404 means that another agent already made it, and the sweep ignores that error.

1. A `status:backlog` work package becomes `status:ready` when every dependency id in its body maps to an issue that closed with `state_reason` equal to `completed`. The sweep skips issues labeled `needs:split` or `type:followup`.
2. A `status:blocked` item becomes `status:ready` when every issue that its blocking comment names has closed as completed. The sweep adds `has-handoff`.
3. A closed item that still carries a status label gets `status:done` in place of that label.

```bash
# Map catalog ids to issue numbers and states once per sweep
gh api --paginate -X GET "repos/$FPS_REPO/issues" -f state=all -f labels=type:wp -f per_page=100 \
  --jq '.[] | [(.title | capture("^\\[(?<id>[A-Z]+-[0-9]+)\\]").id), .number, .state, (.state_reason // "")] | @tsv' > ids.tsv
# Dependencies of one backlog issue, read from its body
gh api "repos/$FPS_REPO/issues/$N" --jq '.body' | sed -n 's/^- \*\*Depends on\.\*\* //p'
# List claimable items
for q in 'type:wp,status:stale' 'type:wp,status:ready' 'type:bug,status:stale' 'type:bug,status:ready'; do
  gh api -X GET "repos/$FPS_REPO/issues" -f state=open -f labels="$q" -f per_page=100 \
    --jq '.[] | select([.labels[].name] | index("needs:split") | not) | [.number, .title, ([.labels[].name] | join(","))] | @tsv'
done
```

The stale sweep posts one comment on every item that it marks stale. The comment starts with a marker that names the expired lease, which is the latest lease among the active cards, or `none` when no card is active. The sweep posts no second comment for a lease that already has one.

```markdown
<!-- fps:stale lease=2026-11-02T09:30:00Z -->
**Stale lease**

The lease of `claude-7f3a2c` on ENG-04 expired at 2026-11-02T09:30:00Z (UTC), so the stale sweep replaced `status:in-progress` with `status:stale`.
Any agent may now resume the item with `fps_swarm.py takeover 23`.
```

## The scanner

The workflow [`.github/workflows/swarm-scan.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/swarm-scan.yml) runs `fps_swarm.py sweep` every 30 minutes, so labels stay current when no agent session is running. It holds no sweep logic of its own, and agents and the scanner therefore apply the same rules. The job uses only the repository token with the permissions `issues: write` and `contents: read`, and `gh` reads that token from `GH_TOKEN`.

The owner runs the scanner by hand from the Actions tab with **Run workflow**. The `dry_run` input defaults to true, and a dry run prints the planned changes, one `<issue>\t<status>` row per change, and changes nothing. Scheduled runs carry no inputs and always apply their changes.

```bash
gh api repos/$FPS_REPO/actions/workflows/swarm-scan.yml/dispatches -f ref=main -f 'inputs[dry_run]=true'
```

Label changes made with the repository token start no other workflow, because GitHub suppresses workflow events from that token. The board sync of OPS-04 therefore does not see the scanner's relabels until the next label event on the same issue.

## Selection rule

An agent picks the first claimable item in this order and skips items outside its competence.

1. Stale items with the `critical-path` label.
2. Ready items with the `critical-path` label.
3. Pull requests waiting for review, when the agent acts as a reviewer. See [Review](#review).
4. Other stale items, then other ready items. Bugs come before work packages within each group.

Within each group, the agent prefers the lowest phase and then the lowest issue number. The agent never holds two leases at once.

## Claim or resume

The agent claims an item in seven steps. The same steps resume an item that has earlier progress, which is the case when the label `has-handoff` is present or the branch already exists.

1. Confirm that the item is claimable and that every dependency issue closed as completed.
2. Read every card on the issue. If an unexpired active card exists, pick another item.
3. Post a status card with state `active`, a lease 90 minutes ahead, and `Takeover of` set to the identifier of the newest earlier card, or `none`.
4. Wait 20 seconds, read every card again, and apply the holder rule. If another card holds the lease, edit the new card to state `superseded` and return to the selection rule.
5. Replace the status label with `status:in-progress`. Add the GitHub login of the agent as an assignee for visibility; the card remains the authority, because agents may share one login.
6. Prepare the branch. When `origin/wp/<ID>` exists, check it out, read the handoff file, run its verification commands, and reuse the open pull request. Otherwise create the branch from `origin/main` and copy the handoff template to `docs/handoff/<ID>.md`.
7. Append a session log row to the handoff file, commit, and push immediately. This first push marks the branch as taken, so an earlier holder that still believes it holds the lease meets a rejected push. Open the pull request when none exists, with the `wip` label and the text `Closes #<issue>`.

The helper performs steps 1 to 5 in one command. It exits with 1 and marks its own card `superseded` when another card wins the settle race. It skips the assignee when `gh api user` fails or when `--no-assign` is given.

```bash
swarm claim 23 --steps 5     # prints card=<id> key=ENG-04 branch=wp/ENG-04 handoff=docs/handoff/ENG-04.md lease=<time>
```

The raw calls for every step follow.

```bash
N=23; ID=ENG-04; BR="wp/$ID"
# Read cards: comment id and marker line
gh api "repos/$FPS_REPO/issues/$N/comments" --paginate \
  --jq '.[] | select(.body | startswith("<!-- fps:card")) | [.id, (.body | split("\n")[0])] | @tsv'
# Post a card (card.md holds the card text) and keep its id
CARD=$(gh api "repos/$FPS_REPO/issues/$N/comments" -F body=@card.md --jq .id)
sleep 20   # then read the cards again and apply the holder rule
# After winning
for l in status%3Aready status%3Astale; do gh api -X DELETE "repos/$FPS_REPO/issues/$N/labels/$l" --silent 2>/dev/null || true; done
gh api "repos/$FPS_REPO/issues/$N/labels" -f 'labels[]=status:in-progress' --silent
gh api "repos/$FPS_REPO/issues/$N/assignees" -f "assignees[]=$(gh api user --jq .login)" --silent
git fetch origin
if git rev-parse -q --verify "origin/$BR" >/dev/null; then
  git switch "$BR" 2>/dev/null || git switch -c "$BR" --track "origin/$BR"
else
  git switch -c "$BR" origin/main && cp docs/handoff/TEMPLATE.md "docs/handoff/$ID.md"
fi
# Append the session log row, then
git add "docs/handoff/$ID.md" && git commit -m "Claim $ID" -m "Work-Package: $ID" -m "Agent: $FPS_AGENT_ID"
git push -u origin "$BR"
PR=$(gh api -X GET "repos/$FPS_REPO/pulls" -f head="alrawi:$BR" -f state=open --jq '.[0].number // empty')
if [ -z "$PR" ]; then
  PR=$(gh api "repos/$FPS_REPO/pulls" -f title="[$ID] Flow program interpreter" -f head="$BR" \
    -f base=main -f body="Closes #$N. Work in progress, see docs/handoff/$ID.md." --jq .number)
fi
gh api "repos/$FPS_REPO/issues/$PR/labels" -f 'labels[]=wip' --silent
```

We open the pull request at claim time, because a visible pull request shows reviewers and successors the work as it grows. The `wip` label tells the owner not to merge it. GitHub's REST API cannot convert a draft pull request to ready, so we do not use drafts.

## Checkpoint

A checkpoint makes the current state recoverable. The agent performs five steps.

1. Run the fast tests of the workstream and record the result in the handoff file.
2. Update the plan checkboxes, progress, next action, and session log in `docs/handoff/<ID>.md`.
3. Read the cards on the issue and confirm that the agent's card still holds the lease with at least 10 minutes left. If the card lost the lease, stop work and push the local commits to `salvage/<ID>-<agent id>` and never to the work branch. Post a comment that names the salvage branch, so the current holder can pick commits from it.
4. Merge `origin/main` into the branch when `main` moved, then commit and push. A checkpoint commit may contain unfinished code when the handoff file says so.
5. Edit the status card with the new lease, progress, last commit, and next action.

```bash
git add -A && git commit -m "Checkpoint $ID: expect modes and timeouts" \
  -m "Work-Package: $ID" -m "Agent: $FPS_AGENT_ID"
swarm heartbeat "$N" --check && git push
swarm heartbeat "$N" --progress "40% (2 of 5 plan steps)" --commit "$(git rev-parse --short HEAD)" \
  --pr 57 --next "Implement expect min_bytes and its timeout test"
# Raw equivalent of the last command
gh api -X PATCH "repos/$FPS_REPO/issues/comments/$CARD" -F body=@card.md
```

The helper finds the card by the agent identifier, so the agent does not need to remember the comment id. Fields that the command does not name keep their values.

An agent never force pushes and never rebases a published branch. A rejected push means that another agent advanced the branch, and the agent must stop, read the cards, and push to a salvage branch. Conflicts in `docs/wiki/Log.md` resolve by keeping both entries in date order.

## Progress percentage

Progress equals the number of checked plan steps divided by the total number of plan steps. Each plan step must take at most 30 minutes of work, so the percentage stays meaningful. An agent that splits a step updates the total and records the change in the session log. Agents do not estimate progress in any other way.

## Release

An agent that must stop on purpose releases the lease. Typical reasons are a low remaining budget, the end of an assigned session, or a package that exceeds the agent's competence.

1. Checkpoint.
2. Set the handoff status to `paused` and write the next action as a single concrete step.
3. Edit the card to state `released`.
4. Replace `status:in-progress` with `status:ready` and add `has-handoff`. Remove the assignee.

The helper performs steps 3 and 4 after the agent checkpoints and updates the handoff file.

```bash
swarm release "$N" --next "Write the timeout test for expect min_bytes"
```

## Oversized packages

An agent that finds a package too large for one session does not grow its scope. It finishes the current plan step, checkpoints, and blocks the package with the labels `status:blocked`, `needs:split`, and `needs:owner`. The blocking comment proposes child packages with titles, paths, and dependencies. A planner agent or the owner then allocates new ids in the same workstream, adds the children to the catalog in a plan pull request, and runs the seeder after it merges. Each child depends on the epic's dependencies, and every dependent of the epic depends on all children. The epic becomes a tracking issue that closes when its children close.

## Block

An agent that cannot continue because of a missing decision, a contract gap, or an open dependency blocks the package.

1. Checkpoint.
2. Post a comment that names the blocker, links the blocking issue or opens one, and states what would unblock the work.
3. Edit the card to state `blocked` and replace the status label with `status:blocked`. Add `needs:owner` when only the owner can resolve the blocker.

The helper performs steps 2 and 3. Its blocking comment starts with a marker that names the blocking issues, and the promotion sweep reads the newest such marker. A marker without issues means that only the owner can unblock the item, so the sweep leaves it alone.

```bash
swarm block "$N" --reason "Needs the counter catalog from CON-02" --blocked-by 18 --needs-owner
# The comment it posts starts with this line
<!-- fps:block agent=claude-7f3a2c issues=18 -->
```

Whoever resolves the blocker relabels the issue `status:ready`. The next agent resumes from the handoff file.

## Takeover

A takeover is a claim of an item whose newest card is expired, released, or blocked. [Claim or resume](#claim-or-resume) covers it, because step 3 names the previous holder and step 6 resumes the branch. The new holder never waits for permission, since the previous agent may have crashed. When the branch fails its verification commands, the new holder fixes it forward from the last commit that the handoff marks as verified and records the failure in the session log.

The helper command `swarm takeover N` follows the claim steps and also accepts an item that still carries `status:in-progress` when its holder card has expired. It refuses an item without an earlier card, because that is a plain claim.

An agent that returns after its lease expired treats its own item like anybody else's. It reads the cards, and when another card holds the lease it edits its old card to `superseded`, pushes its local commits to a salvage branch, and stops.

## Finish

An agent finishes an item when the [Definition of Done](Implementation-Plan.md#definition-of-done) holds. The agent completes every step while its lease is active.

1. Ingest the result into the wiki as [Wiki Maintenance](Wiki-Maintenance.md) describes.
2. Merge `origin/main` into the branch and run the full tests of the workstream.
3. Set the handoff status to `ready for review` and progress to 100%, then push.
4. Remove the `wip` label from the pull request.
5. Replace `status:in-progress` with `status:review` and edit the card to state `done`.

The helper performs steps 4 and 5 and takes the pull request number from the card when `--pr` is absent.

```bash
swarm review "$N" --pr 57
```

A reviewer agent then reviews the pull request. The owner merges it, and the `Closes #<issue>` text closes the issue. The promotion sweep then replaces the status label with `status:done`.

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

The verdict is `approve` or `changes`. A `changes` verdict lists each required change as a checkbox, and the reviewer then replaces `status:review` with `status:ready` and adds `has-handoff`. Any agent may claim the item again and resume it.

Reviewers find work with one query. Items in review without a review comment at the current head commit wait for a reviewer.

```bash
gh api -X GET "repos/$FPS_REPO/issues" -f state=open -f labels=status:review -f per_page=100 \
  --jq '.[] | [.number, .title] | @tsv'
gh api "repos/$FPS_REPO/issues/$PR/comments" --jq '[.[] | select(.body | startswith("<!-- fps:review"))] | length'
```

## Creating tickets

Agents create tickets with the REST API and the templates in `.github/ISSUE_TEMPLATE/`. Every new issue is added to the Project board automatically, as [Project Board Setup](Project-Board-Setup.md) describes.

| Ticket | When | Labels | Ready immediately |
|---|---|---|---|
| Work package from the catalog | A planner seeds a phase with `tools/plan/plan.py seed --phase N --apply` | Set by the seeder | When its dependencies are closed |
| Split of an epic or an oversized package | A package needs more than one session, see [Oversized packages](#oversized-packages) | Set by the seeder after the plan pull request | When its dependencies are closed |
| Bug | A test or a review finds a defect outside the current package | `type:bug`, `ws:<workstream>`, `status:ready` | Yes |
| Follow up | Work that the current package should not absorb | `type:followup`, `ws:<workstream>`, `status:backlog` | After triage by the owner or a planner |
| Contract change | An implementation needs a contract change | `type:wp`, `ws:con`, `needs:owner` | After owner approval |

```bash
gh api "repos/$FPS_REPO/issues" -f title="[bug] Interpreter drops the last byte of a chunked body" \
  -F body=@bug.md -f 'labels[]=type:bug' -f 'labels[]=ws:eng' -f 'labels[]=status:ready' --jq .number
```

An agent fixes a defect inside its own package without a ticket. It creates a ticket for anything outside its paths and leaves that work to another agent.

## Guarantees and limits

The protocol bounds lost work to one checkpoint interval, which is at most 30 minutes, and salvage branches keep even that work reachable. Two agents that claim within the same settle window resolve to one holder through the lowest comment id. A returning agent cannot overwrite a successor for two reasons. It must hold the lease with 10 minutes left before it pushes, and the successor pushes a claim commit right after winning, so Git rejects the stale agent's push as a non fast forward.

The protocol does not prevent an agent from ignoring it. The owner reviews every merge, and the review checklist verifies the handoff file and the card history.
