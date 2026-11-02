---
name: Work package
about: A unit of work that one agent can finish in one session
title: "[XXX-NN] Title"
labels: ["type:wp", "status:backlog", "needs:owner"]
# Prefer adding the package to plan/work-packages.yaml and running the seeder.
---

<!-- fps:wp id=XXX-NN -->
Phase N · size S|M · workstream XXX

- **Depends on.** IDs or none
- **Paths.** `path/one/`, `path/two/`

Summary in two or three sentences.

Deliverables.

- Deliverable

Acceptance.

- [ ] A criterion that names a command or an observation

## Coordination

Follow the [Swarm Protocol](https://github.com/alrawi/FixingPointSystem/wiki/Swarm-Protocol). Branch `wp/XXX-NN`, handoff file `docs/handoff/XXX-NN.md`. A new package becomes ready after a plan pull request adds it to `plan/work-packages.yaml`.
