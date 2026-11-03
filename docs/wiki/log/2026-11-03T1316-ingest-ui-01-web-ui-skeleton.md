## [2026-11-03] ingest | UI-01 Web UI skeleton

We created the web UI project in `ui/` with the stack of decision D11 and added the section UI skeleton at the end of the [Web UI](Web-UI.md) page. The project holds a client generated from the OpenAPI contract with a currency check, a `fetch()` reader for the metrics event stream, token sign in, an accessible layout, a capabilities view, and a mock server built from the contract examples. Every version is pinned and `ui/package-lock.json` locks the tree, and a license check fails on any license outside a GPL-3.0 compatible allow list. The new file `mk/ui.mk` adds the root targets `lint-ui` and `test-ui` and the browser target `e2e-ui`, and the workflow `.github/workflows/ui.yml` runs them. We corrected the claim on the Web UI page that D11 still waits for owner approval. The evidence is `make lint-ui test-ui e2e-ui` and `docs/handoff/UI-01.md` on the branch `wp/UI-01`.

### Shared page notes

- [Testing](Testing.md) could add the rows `lint-ui`, `test-ui`, and `e2e-ui` to the target table. `e2e-ui` needs a Playwright browser and stays out of `make e2e`.
- [Coding Standards](Coding-Standards.md) could name the mock server in `ui/mock/` and the client currency check `npm run check:api` in its TypeScript section.
- [Home](Home.md) and the sidebar could link the [Web UI](Web-UI.md) page if they do not yet.
- Decision D11 names Prism for the mock API. UI-01 uses a local `node:http` server with Ajv contract tests instead, and the owner may want the record to note this.
