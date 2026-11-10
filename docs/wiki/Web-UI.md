# Web UI

This page describes the web UI of the UI workstream. Decision D8 puts a full web UI in v1 that uses only the public REST API, and packages UI-00 to UI-07 build it in `ui/`.

## Stack decision

UI-00 proposed the stack as decision record [D11](https://github.com/alrawi/FixingPointSystem/blob/main/docs/decisions/0011-web-ui-stack.md), which the owner accepted on 2026-11-03. The record compares React 19 with Vite, Svelte 5 with SvelteKit 3 in static mode, and Vue 3.5 with Vite. It proposes React 19 with Vite 8 and TypeScript, React Aria Components for accessible widgets, openapi-typescript and openapi-fetch for the generated client, uPlot for live charts, and Vitest with Playwright for tests. Every named package carries MIT, Apache-2.0, or MPL-2.0, which are compatible with GPL-3.0.

We measured the baseline bundle of one small component with Vite 8.3.5.

| Framework | Gzip size |
|---|---|
| React 19.3.1 | 68.7 kB |
| Vue 3.5.44 | 24.9 kB |
| Svelte 5.57.3 | 12.8 kB |

The record accepts the larger React bundle in exchange for the most widely tested accessible primitives, the healthiest ecosystem, and a stable base, and it caps the initial JavaScript at 250 kB gzip.

## Live metrics

The API streams metrics as Server Sent Events and authenticates with bearer tokens. The browser `EventSource` interface cannot send an `Authorization` header, so the UI reads the stream with `fetch()` and a local parser. The UI keeps samples outside React state and pushes them to uPlot directly, so a once per second stream does not re-render the page.

## Where to read more

The [UI stack README](https://github.com/alrawi/FixingPointSystem/blob/main/ui/docs/README.md) lists the stack and the rules that UI packages follow.

## UI skeleton

UI-01 created the project in `ui/` with the stack of D11. The [UI README](https://github.com/alrawi/FixingPointSystem/blob/main/ui/README.md) lists every command, and this section records what exists and why.

### What the skeleton holds

| Part | Where | Behavior |
|---|---|---|
| Generated client | `ui/src/api/schema.d.ts` and `client.ts` | openapi-typescript generates the types from `contracts/openapi/fps-api-v1.yaml`, and openapi-fetch sends every call with the bearer token. `npm run check:api` fails when the committed types differ from a fresh generation |
| Event stream reader | `ui/src/api/eventStream.ts` | Reads `text/event-stream` over `fetch()`, reconnects with `Last-Event-ID` and the `retry` delay, and stops on 401 or 403 |
| Token sign in | `ui/src/auth/` | Checks the pasted token with `GET /capabilities` before it keeps it in `sessionStorage`, and signs out with a notice when a later call answers 401 or 403 |
| Layout | `ui/src/layout/AppLayout.tsx` | A skip link, a banner with the sign out button, a labelled main navigation, and the main region, which collapse to one column on narrow screens |
| Capabilities view | `ui/src/pages/CapabilitiesPage.tsx` | The engine API version, backends, agents, and installed modules |
| Mock server | `ui/mock/` | A `node:http` server that serves the read operations from the contract examples and accepts the token `fps-mock-token` |

### Checks

`make lint-ui` runs Prettier, ESLint with the strict rules of typescript-eslint and eslint-plugin-jsx-a11y and warnings as errors, `tsc` in strict mode, the client currency check, and the license check. `make test-ui` runs Vitest, the production build, and the bundle check. `make e2e-ui` runs two Playwright smoke tests with axe-core in Chromium. The workflow `.github/workflows/ui.yml` runs the first two in one job and the browser tests in another, and it triggers on changes to `ui/`, the OpenAPI contract, the schemas, and the examples.

The tests cover four levels. Unit tests check the event stream parser, the reader, the client, and the two check scripts. Contract tests validate every mock response against the response schema of its operation with Ajv, so the mock cannot drift from the contract. Component tests render the whole UI against the running mock server, sign in, read the capabilities, sign out, and run axe-core on the document. The browser smoke tests repeat the sign in with color contrast checks and test the skip link.

The license check reads every package in `ui/package-lock.json`, compares the license with the installed `package.json`, and evaluates SPDX expressions against an allow list of GPL-3.0 compatible licenses. At UI-01 the tree holds 447 packages under MIT, Apache-2.0, ISC, MPL-2.0, the BSD licenses, BlueOak-1.0.0, MIT-0, CC0-1.0, 0BSD, Python-2.0, and CC-BY-4.0. Only development tools carry the last seven, and the shipped bundle contains MIT, Apache-2.0, and 0BSD code.

The bundle check gzips every script and module preload that `dist/index.html` loads. The initial JavaScript of the skeleton weighs 109 kB gzip against the budget of 250 kB.

### Choices

We serve the mock from a small `node:http` server instead of Prism. The server returns the same data on every call, adds no dependency, and runs inside Vitest, so component tests use real HTTP without MSW. The Ajv contract tests give the guarantee that Prism would give, which is that every mocked body matches the contract.

ESLint stays at version 10 as D11 names it. eslint-plugin-jsx-a11y 6.10.2 declares peer support only up to ESLint 9, so `ui/package.json` overrides its peer range, and we checked that its rules still report violations. openapi-typescript 7.13.0 declares TypeScript 5 as its peer, and a second override lets it run with TypeScript 6.0.3.

The sign in button does not use the `isPending` state of React Aria. That state announces through a live region that keeps a reference to the button after the sign in page unmounts, and axe-core reports the dangling reference as an image without a name. The page shows a status message while it checks the token instead, and a component test guards the fix.

## Module library

UI-02 added the module library, which lists the installed modules and shows the manifest of each one. It uses `GET /modules` and `GET /modules/{name}/{version}` through the generated client.

### Views

| Path | View | Behavior |
|---|---|---|
| `/modules` | `ui/src/pages/ModulesPage.tsx` | A table of every installed module with its version, type, license, scenarios, and summary. Each module name links to its detail view. An empty registry shows a sentence in place of the table |
| `/modules/{name}/{version}` | `ui/src/pages/ModuleDetailPage.tsx` | The manifest fields, the license and provenance, the scenario files, a table of the parameters read from the JSON Schema of the manifest, and a table of the template functions with their call signatures. A module that is not installed shows a notice |

The main navigation gained the entry Modules, which stays current on every detail view. Every view reports an API error in an alert and signs the user out when the controller rejects the token.

### Keyboard navigation

Every module name is a link, so Tab reaches each module and Enter opens it, and the detail view starts with a link back to the list. After a navigation inside the UI, focus moves to the heading of the new view, so keyboard and screen reader users start at the top of the content. The first page of a session keeps the browser focus, which leaves the skip link as the first Tab stop.

### Paging and retries

`fetchModules` in `ui/src/api/client.ts` asks for the largest page that the contract allows, which is 500, and follows `next_page_token` until the last page. A token that repeats ends the listing, so a faulty controller cannot loop the UI. The query cache now retries only errors that may heal, which are network failures, HTTP 408, HTTP 429, and server errors, because a missing module or a rejected token does not change on a second try.

### Mock server

The mock server pages `GET /modules` with `page_size` and `page_token` as the contract describes, and it answers a `page_size` outside 1 to 500 or a bad token with 400. Its options accept faults, which answer matching requests with a problem of a given status, and the helper `withoutModules` builds an empty registry. The contract tests validate the paged lists, the empty registry, and the fault answers against the contract.

### Tests

The component tests in `ui/src/pages/Modules.test.tsx` start a mock server per test and cover an empty registry, the two example modules `dns` and `http1`, an API error of the list, a rejected token, a module that is not installed, an API error of the manifest, and the keyboard path from the navigation to a detail view and back. Each view runs an axe-core scan. Unit tests cover the parameter, signature, and path helpers and the paging of `fetchModules`, and a Playwright test repeats the keyboard path with color contrast checks. The initial JavaScript weighs 110.9 kB gzip against the budget of 250 kB, and the license check passes with no new dependency.

## Test configuration editor

UI-03 added the configuration views. `/configs` lists the stored configurations through `GET /configs`, `/configs/new` opens the editor on a starter configuration, and `/configs/{id}` opens the editor on a stored one through `GET /configs/{id}`. The main navigation gained the entry Configurations. The editor route loads on demand, so the initial JavaScript grew only from 110.9 kB to 112.2 kB gzip against the budget of 250 kB, and the editor chunk adds 75.5 kB when someone opens it.

### Editor

| Part | Where | Behavior |
|---|---|---|
| Text model | `ui/src/config/document.ts` | The YAML text is the only state. Edits go through a `yaml` Document, and the module defines the round trip |
| Browser check | `ui/src/config/schema.ts` | Ajv validates the text against `contracts/schemas/test-config.schema.json`, bundled at build time, and maps every failure to the JSON Pointer of its field |
| Editor state | `ui/src/config/model.tsx` | One reducer parses the text, runs the check, and keeps the last value that parsed, so the forms stay readable while the YAML has a syntax error |
| Forms | `ui/src/config/fields.tsx`, `sections.tsx` | General, Network (ports, host pools, external servers), Profile (basis and mix), Load (objective and phases), Timeline, and Criteria, each field with its own inline errors, and add, remove, and move buttons for every list |
| Page | `ui/src/pages/ConfigEditorPage.tsx` | A list of checks, the buttons Save as new and Validate on controller, the forms, and the YAML view side by side on wide screens |

The forms and the YAML view edit the same text. A form edit changes the text at once, and a YAML edit changes the forms at once. A syntax error in the YAML pauses the forms, shows the line and column, and disables saving, and fixing the text brings the forms back. Fields that the forms do not cover, which are `metadata.labels`, `modules`, `params` of a mix entry, and `where` of a criterion, stay in the YAML and show as read only text with a pointer to the YAML view. A scenario field suggests the scenarios of the installed modules from `GET /modules`.

### Round trip definition

The catalog says that every example test round trips through the editor without changes. We define that as follows.

1. Text that the user loaded or typed stays byte for byte as it is until a form field edits it. Loading never reserializes.
2. A form edit parses the text into a `yaml` Document, changes only the nodes of that edit, and serializes with `lineWidth: 0`. Comments, key order, quoting, and flow or block style of every other node stay. A new list item takes the flow or block style of its siblings. An edit may normalize line endings and the final newline of the whole text.
3. A configuration that the controller returns is JSON and carries no comments. The editor shows it as YAML in the key order of the schema, and the YAML parses back to an equal value. Saving sends the YAML text to `POST /configs` with the content type `application/yaml`, so the controller reads what the editor shows, and the controller then stores and returns it as JSON.

`ui/src/config/roundtrip.test.ts` loads every file of `contracts/examples/tests/` and checks, for each one, that the text parses and passes the schema in the browser, that the Document serializes byte for byte, that an edit which sets every field to its current value changes no byte, that a change of one field changes exactly one line and keeps the comments, and that the canonical YAML of the JSON form reads back equal and is stable. The test needs `lineWidth: 0`, because the default width of 80 characters folds the long flow mappings of the examples and the text would then differ.

### Saving and validation

The contract has no update operation, so Save as new always stores a new configuration with `createConfig`. The controller validates only stored configurations, so Validate on controller validates the stored copy when the editor still shows the text that the controller holds, and otherwise stores the text as a new configuration first and then validates it. The buttons stay disabled while the browser check finds a problem, so the controller sees only schema valid text. The findings of `validateConfig` and the `errors` of a 422 problem show next to the fields by their JSON Pointer and in the list of checks. They disappear when the text changes, because they describe the old text. The capacity of the report shows as a sentence. A 401 or 403 answer signs the user out like every other view.

### Mock server

The mock keeps the configurations in memory per server, starts with the six example tests under the ids `cfg_<file name>`, and serves `listConfigs`, `createConfig` (YAML or JSON, 201 or 422 with `errors`, 415 for another content type), `getConfig`, and `validateConfig`. Its validation checks the schema and that every scenario of the mix comes from an installed example module, so `dns_zone` fails with a finding at `/profile/mix/0/scenario`, because the example `dns` module has no `lookup_a`. `mock/configs.test.ts` validates every response against the contract.

### Tests

Unit tests cover the text model, the schema messages, and the round trip of the corpus. Component tests in `ui/src/pages/ConfigEditor.test.tsx` cover the list, loading a stored configuration, form to YAML and YAML to form sync, inline errors, syntax errors, list editing, scenario suggestions, saving, validation with findings on fields, stale findings, API failures, sign out, and axe scans. The Playwright test `ui/e2e/configs.spec.ts` repeats the main path in Chromium with the color contrast rule. The Playwright run found one real defect that jsdom cannot see: a button whose accessible name did not contain its visible text.

### Choices and limits

We added `ajv` and `yaml` as runtime dependencies, which were development dependencies at UI-02. Both are MIT and ISC, and `npm run check:licenses` passes. Ajv compiles the schema with `new Function`, so a future Content-Security-Policy for the UI that forbids `unsafe-eval` needs a precompiled validator. The controller sets no such policy on the UI today.
