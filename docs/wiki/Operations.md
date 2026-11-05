# Operations

This page describes the repository automation that the OPS workstream runs on GitHub, beyond the build and test workflows. It grows with each OPS package. The [Swarm Protocol](Swarm-Protocol.md) describes the scanner, and [Project Board Setup](Project-Board-Setup.md) describes the board itself.

## Project board status sync

The workflow `.github/workflows/project-sync.yml` copies the status label of each work item into the Status field of the owner's user project, so the Board views that group by Status follow the labels. It calls `tools/swarm/project_sync.py`, which needs only the Python standard library and talks to the GraphQL API.

| Trigger | What the job syncs |
|---|---|
| An issue is opened, reopened, labeled, or unlabeled | The one issue of the event, read fresh from the API |
| The schedule at minutes 7 and 37 of every hour | Every open and closed issue, which covers the relabels of the [scanner](Swarm-Protocol.md#the-scanner) a few minutes after each scanner run |
| A manual run of **Run workflow** | Every issue, and with `dry_run` set, which is the default, the job prints the planned changes and changes nothing |

The scanner relabels with the repository token, and GitHub starts no workflow for events from that token. The schedule therefore moves a card that the scanner relabeled within about 7 minutes, while a label change made by a person or an agent with its own token moves the card within one minute.

For each issue with exactly one status label, the script looks up the Status option with the same name and changes the item only when its current option differs. It adds an issue that is not yet on the board, so the sync works even before **Auto-add to project** catches the issue. The script maps the labels as follows.

| Label | Status option |
|---|---|
| `status:backlog` | Backlog |
| `status:ready` | Ready |
| `status:in-progress` | In progress |
| `status:blocked` | Blocked |
| `status:stale` | Stale |
| `status:review` | Review |
| `status:done` | Done |

The script exits with 0 when the sync succeeds and also when the secret `FPS_PROJECT_TOKEN` is absent, in which case it prints a notice and makes no request. It exits with 1 when an issue carries more than one status label or the board lacks the matching option, after it has synced every other issue. It exits with 2 when it finds no project, more than one candidate project, no Status field, or a token without the needed access.

### Owner setup

The owner completes four steps once, after the board exists as [Project Board Setup](Project-Board-Setup.md) describes.

1. Create a fine grained personal access token at https://github.com/settings/personal-access-tokens/new with the resource owner `alrawi`. Under **Repository access**, select only `alrawi/FixingPointSystem` and grant **Issues** read only access. Under **Account permissions**, grant **Projects** read and write access. A classic token with the scopes `project` and `repo` also works.
2. Store the token as the repository secret `FPS_PROJECT_TOKEN` under **Settings**, **Secrets and variables**, **Actions**.
3. Optionally store the project number as the repository variable `FPS_PROJECT_NUMBER` on the **Variables** tab of the same page. The number is the last part of the project URL, for example `1` in `https://github.com/users/alrawi/projects/1`. Without the variable the script uses the one project of `alrawi` that is linked to the repository, and it stops with an error when it finds more than one.
4. Open **Actions**, select **project-sync**, and select **Run workflow** with `dry_run` set. The log names the project and lists each planned change. Run it again with `dry_run` cleared to apply the changes, then relabel a test issue and check that its card moves within one minute.

### Running it by hand

An agent or the owner can run the same command locally with a token, for example to check one issue.

```bash
export FPS_PROJECT_TOKEN=...            # never commit or paste the token
python3 tools/swarm/project_sync.py --issue 13 --dry-run
python3 tools/swarm/project_sync.py     # sync every issue
```

Agents in sandboxed sessions cannot reach the GraphQL API, so they never run it. The tests in `tools/swarm/test_project_sync.py` replay GraphQL responses from `tools/swarm/testdata/project_sync/` and never reach the network.

## Benchmark runner

The nightly benchmark of QA-08 runs on a self hosted runner on the owner's Linux server with the label `fps-bench`. The owner registers it with the steps in [`bench/README.md`, section Registering the owner host](https://github.com/alrawi/FixingPointSystem/blob/main/bench/README.md#registering-the-owner-host), and [Benchmarks](Benchmarks.md) explains the comparison.
