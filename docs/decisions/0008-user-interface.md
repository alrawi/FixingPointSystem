# D8. User interface in v1

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

BreakingPoint users build tests in a graphical interface.

## Options

- No UI with API, CLI, and Grafana (recommended default)
- Minimal web UI
- Full web UI (chosen)

## Decision

v1 includes a full web UI built only on the public REST API. The UI runs as a parallel workstream from Phase 1 and joins the v1 release gate.

## Consequences

The UI adds a large effort class to v1 and exercises the REST API as a real client.
