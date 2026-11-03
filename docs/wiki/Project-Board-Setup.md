# Project Board Setup

The GitHub Project gives the owner one board over every work package issue. Agents never need the board, because issue labels carry the authoritative status, and agents in sandboxed sessions cannot reach the Projects API. The owner sets the board up once in the browser.

## One time setup by the owner

The owner completes seven steps in about ten minutes.

1. Run `tools/swarm/bootstrap-labels.sh` once, or ask an agent to run it. The script creates the label set of the [Swarm Protocol](Swarm-Protocol.md) and changes nothing that already exists.
2. Open https://github.com/users/alrawi/projects, select **New project**, choose the **Board** template, and name it `FixingPointSystem v1`.
3. In the project settings, under **Manage access** or the repository's **Projects** tab, link the project to `alrawi/FixingPointSystem`.
4. Open **Workflows** in the project menu and enable **Auto-add to project** with the filter `is:issue,pr label:type:wp,type:bug,type:followup`. New tickets then appear on the board without any agent action.
5. In the same menu, enable **Item closed** and **Pull request merged** with the target status **Done**.
6. Edit the **Status** field options to match the status labels, which are Backlog, Ready, In progress, Blocked, Stale, Review, and Done.
7. Add the saved views in the table below.

| View | Layout | Filter | Group or sort |
|---|---|---|---|
| Board | Board | `label:type:wp` | Status |
| Ready now | Table | `label:status:ready,status:stale` | Sort by labels, then number |
| In flight | Table | `label:status:in-progress,status:review` | Group by assignee |
| Needs owner | Table | `label:needs:owner` | Sort by updated |
| Phase 1 | Board | `label:phase:1` | Status |
| Critical path | Table | `label:critical-path is:open` | Sort by number |

## Keeping the Status field current

Labels change through the REST API, and the Status field does not follow labels by itself. The views that filter by label stay correct without any automation. The Board views group by the Status field, and they stay correct once the owner stores the token below.

The [scanner](Swarm-Protocol.md#the-scanner) changes labels every 30 minutes with the repository token, and GitHub starts no workflow for events from that token. The OPS-04 workflow `project-sync` therefore runs on label events and also on a schedule a few minutes after each scanner run.

The workflow needs a token with project access, because the default workflow token cannot write to a user owned project. The owner creates a fine grained token with read and write access to projects and read access to the issues of this repository, and stores it as the repository secret `FPS_PROJECT_TOKEN`. Without the secret the workflow succeeds and changes nothing.

[Operations](Operations.md#project-board-status-sync) describes the `project-sync` workflow, its schedule, and the owner setup steps.

## Seeding the board

A planner agent seeds one phase at a time. The command prints the planned issues without `--apply`.

```bash
python3 tools/plan/plan.py seed --phase 1          # dry run
python3 tools/plan/plan.py seed --phase 1 --apply  # create the issues
```

The seeder skips work packages whose issue already exists, so a second run is safe. Auto add then places every new issue on the board.
