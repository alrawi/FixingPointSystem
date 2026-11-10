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

The API streams metrics as Server Sent Events and authenticates with bearer tokens. The browser `EventSource` interface cannot send an `Authorization` header, so the UI reads the stream with `fetch()` and a local parser. The UI keeps samples outside React state, so a once per second stream does not re-render the page once per event. D11 names uPlot for the drawing, and UI-04 draws with a small SVG chart instead, as the section on the run views explains.

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

## Run launch and live metrics

UI-04 added the run views. `/runs` starts a run and lists the runs, and `/runs/{id}` follows one run live. The main navigation gained the entry Runs. The run views are part of the initial bundle, which grew from 112.2 kB to 119.6 kB gzip against the budget of 250 kB.

### Views

| Path | View | Behavior |
|---|---|---|
| `/runs` | `ui/src/pages/RunsPage.tsx` | A form with a select of the stored configurations (`listConfigs`) and a checkbox for each agent (`listAgents`), and a table of the runs, newest first (`listRuns`). Leaving every agent unchecked sends no `agents` field, and the controller then picks the reserved agents. Offline and busy agents cannot be checked. `/runs?config={id}` preselects a configuration. A refusal of `startRun` shows the detail of the problem. The list reads again on every visit and every few seconds while a run is active |
| `/runs/{id}` | `ui/src/pages/RunPage.tsx` | The state of the run, the states that the view saw in order, the reason and the degraded flag, the start and end times, a Stop run button with a Force box, nine counters, and five charts |

`startRun` opens the view of the new run. The run state comes from `getRun`, which the view polls once per second until the state is final, and from the answer of `stopRun`. The API has no log of state transitions, so the list of transitions holds the states that the view saw, which are all of them when the view opens at the start of a run. Stop run shows while the run is pending, preparing, or running. A stop request moves the run to stopping, and the button gives way to a sentence. A forced stop sends `force: true`.

### Counters and charts

The view reads the `application` view of the metrics with `side=client`. Its samples carry the scenario metrics summed over scenarios, keyed by the application, which is the module name. The counters add each metric over the applications of the newest tick: sessions attempted, succeeded, and failed, the failed ratio (failed over attempted), sessions per second, concurrent sessions, throughput, and bytes sent and received. Until the first sample arrives the counters show the `summary` of the run. The five charts show the response time and the session duration with the statistics p50, p90, and p99, which belong to one application, so a select picks the application when the run has more than one, and the rates, the concurrency, and the throughput, summed over applications. The charts read the histogram statistics as the catalog names them, for example `sessions.response_time.p99`, in microseconds.

Every chart is a small SVG line chart written for the project in `ui/src/runs/LineChart.tsx`, and it shows the newest 300 points. Each line differs by color and by dash pattern. A legend lists the newest value of each line as text, and a disclosure under the chart opens a table with the newest ten samples, so the chart is never the only way to read a value. The picture has a title and a description that name the newest values.

### Choice of the chart

D11 names uPlot, which is not in the lock file. The charts of the run view have at most three lines of a few hundred points, so we wrote the SVG chart instead of adding a dependency. It adds no license question and no weight beyond a few kilobytes, and a test can read its points in jsdom, which has no canvas. The rule of D11 that samples stay outside React state still holds: `ui/src/runs/metricsStore.ts` keeps a ring of the newest 600 ticks and tells the page once per flush. The page renders when a tick completes and not once per event, because the store flushes 100 ms after the last event of a tick. If a later package needs charts with many thousands of points, it can put uPlot behind the same `LineChart` props. We list this deviation from D11 for the owner in the log entry of UI-04.

### Event stream, reconnect, and polling

`ui/src/runs/useRunMetrics.ts` follows the metrics of one run.

1. It reads `getRunMetrics` for the `application` view once, so a view that opens in the middle of a run has history.
2. It reads `GET /runs/{id}/metrics:stream?view=application&side=client` with the fetch based reader of UI-01, which sends the bearer token. The stream carries one event per application and tick, whose `data` is a `MetricSample`, and whose `id` is the tick number of the connection. The event `end` carries the run state in its data and closes the stream. The reader follows the `retry` field.
3. After a dropped connection the reader reconnects with `Last-Event-ID`, and the view then reads `getRunMetrics` from the newest tick that it holds, because the stream sends only the newest tick of each connection, so the ticks of the gap would be lost otherwise. Samples merge by time and application, so overlaps do no harm. When the run reaches a final state while the connection is down, the view reads the metrics once more and stops.
4. A 501 answer to the stream means that the controller has no stream, as the CLI treats it in `fps watch`. The view then polls `getRunMetrics` every second with `from` set to the newest tick until the run is final and one more poll has run. Any other client error ends the feed with a message, and a 401 or 403 signs the user out.

A line above the counters says which mode applies, which are connecting, live, reconnecting, polling, ended, and failed.

### Mock server

The mock serves `listRuns`, `startRun`, `getRun`, `stopRun`, `getRunMetrics`, and `streamRunMetrics` in `ui/mock/runs.ts`. A run follows a fixed script on a clock that a test can replace: pending, preparing, running for a set number of ticks, and then completed. A stop request moves it through stopping to aborted, and a forced stop aborts it at once. The mock fills the `application` and `scenario` views and answers every other view with no samples. Every value is a function of the tick number, and every timestamp is the tick number added to a fixed start time, so the tests state the expected numbers as literals. The option `runs` sets the clock, the length of a tick, and the length of a run, `streamDrops` cuts the first connections of the stream, and the `faults` option can answer the stream with 501 or 401. `mock/runs.test.ts` validates every response against the OpenAPI document, parses the stream with the reader of the UI, and checks every `data` field against the `MetricSample` schema and the event names `message` and `end`.

### Tests

`ui/src/runs/fixtures/run-stream.sse` is a recording of the stream of a mock run of `http_smoke`: the retry field, ten ticks, and the end event. `npm run record:stream` makes it again from the mock server over HTTP. The acceptance test in `ui/src/pages/Runs.test.tsx` replays it into the run view in two chunks per event that cut the events in the middle, checks the counters and the chart points after three ticks and after the end event, and checks that the bearer token went out with the stream request. Further component tests cover the start form with agents, the states from pending to aborted with a stop, a forced stop, a run that opens in the middle, a missing run, an error of the list, a reconnect with `Last-Event-ID` and no missing tick, the polling fallback on 501, and a rejected token on the stream. Unit tests cover the store, the chart geometry, and the number formats. The Playwright test `ui/e2e/runs.spec.ts` starts a run against the mock in Chromium, waits for live values, stops the run, and runs axe-core with the contrast rule. The development sandbox has no browser, so CI runs it.

### Limits

The charts read the `application` view only. The views `port`, `tcp`, `tls`, and `realism` are not drawn yet, and the server side of the sessions (`side=server`) has no switch. A run that outlives the ring shows its newest ten minutes.
