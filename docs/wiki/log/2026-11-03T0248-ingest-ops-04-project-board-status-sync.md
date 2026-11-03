## [2026-11-03] ingest | OPS-04 Project board status sync

We added the workflow `.github/workflows/project-sync.yml` and the command `tools/swarm/project_sync.py`, which copy the status label of each work item into the Status field of the owner's user project over GraphQL. The workflow runs on issue label events for the one issue of the event, on a schedule at minutes 7 and 37 to catch the relabels of the scanner, and on manual dispatch with a `dry_run` input. It uses the secret `FPS_PROJECT_TOKEN` and the optional repository variable `FPS_PROJECT_NUMBER`, and it exits with 0 and changes nothing when the secret is absent. We created the component page [Operations](Operations.md) with the owner setup steps, and with the coordinator's approval we added one line to [Project Board Setup](Project-Board-Setup.md) that links it. The evidence is `docs/handoff/OPS-04.md` and `tools/swarm/test_project_sync.py`, which replays GraphQL responses and never reaches the network. Nobody has run the sync against the live API yet, because sandboxed agents cannot reach it.

### Shared page notes

- [Project Board Setup](Project-Board-Setup.md), section Keeping the Status field current. Our pull request already appends one line that links [Operations](Operations.md), so `plan.py lint` passes. We suggest that the coordinator also replace the last two paragraphs of the section with the following text, which states that the workflow now exists.

  > The [scanner](Swarm-Protocol.md#the-scanner) changes labels every 30 minutes with the repository token, and GitHub starts no workflow for events from that token. The OPS-04 workflow `project-sync` therefore runs on label events and also on a schedule a few minutes after each scanner run.
  >
  > The workflow needs a token with project access, because the default workflow token cannot write to a user owned project. The owner creates a fine grained token with read and write access to projects and read access to the issues of this repository, and stores it as the repository secret `FPS_PROJECT_TOKEN`. Without the secret the workflow succeeds and changes nothing.

- `_Sidebar.md` and [Home](Home.md). Add a link to [Operations](Operations.md) next to the other component pages.
