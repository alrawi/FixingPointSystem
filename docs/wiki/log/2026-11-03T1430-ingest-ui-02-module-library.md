## [2026-11-03] ingest | UI-02 Module library view

We added the module list view and the module detail view to the web UI and a Module library section at the end of the [Web UI](Web-UI.md) page. The list view at `/modules` shows every installed module and links each one to its detail view at `/modules/{name}/{version}`, which shows the manifest fields, the license and provenance, the scenarios, the parameters, and the template functions. The client follows `next_page_token` across pages, the query cache retries only errors that may heal, and the mock server gained paging and fault injection for module lists. The evidence is `make lint-ui test-ui`, `make e2e-ui`, and `docs/handoff/UI-02.md` on the branch `wp/UI-02`.

## Shared page notes

The coordinator may apply two notes.

1. [Home](Home.md) can mark UI-02 as ready for review.
2. [Testing](Testing.md) can note that the UI component tests start one mock server per test when a test needs an empty registry or an injected fault.
