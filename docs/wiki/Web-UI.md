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

## Reports and run comparison

UI-05 added the report view `/runs/{id}/report` and the comparison view `/compare`. Both read `GET /runs/{id}/report` through `fetchRunReport` and use no other endpoint, apart from `GET /runs` for the run selects. The Runs page links every run to its report and to the comparison, and the live view of a run links to its report. The two views are part of the initial bundle, which grew from 119.6 kB to 123.0 kB gzip against the budget of 250 kB, and no dependency was added.

### Views

| Path | View | Behavior |
|---|---|---|
| `/runs/{id}/report` | `ui/src/pages/ReportPage.tsx` | The verdict as one word (Passed, Failed, or No verdict) with the reason and the degraded notice, the run facts (state, configuration, times, duration, seed, engine version, modules), a table of the criteria with the observed value, the result, and the reason, a table of the realism results with property, labels, observed value, target, and result, a table of the scenarios of the run (UI-08), and the totals of the run. A run that is still active shows a sentence in place of results and reads the report again every few seconds until the run is final |
| `/compare?a={id}&b={id}` | `ui/src/pages/ComparePage.tsx` | Two selects for run A and run B, kept in the query string so the link can be shared, and tables that put the runs side by side: the run facts, the totals with the change from A to B, the scenarios with both runs in each cell, the criteria, and the realism results, each criteria and realism row with a Change column. A summary sentence above the tables counts the results that got worse. The same run on both sides, a run that does not exist, and a report that fails to load each show a message |

Every outcome is a word, so color never carries the meaning alone. The words are Passed, Failed, and No value for a criterion, and Same, Now fails, Now passes, Differs, Only in run A, and Only in run B for a change. The tables carry row and column headers, and both views run axe-core in the component tests.

### What the report holds

The comparison model is in `ui/src/reports/compare.ts`, and its functions are pure.

1. A criterion with `passed` and `observed` shows both. A criterion whose metric has no value carries neither and shows No value with its reason. This follows the `CriterionResult` schema, and the controller reports a missing metric this way, as `controller/internal/report` documents.
2. The criteria of two runs pair by metric, operator, value, and `where` filter (UI-08, since REST 0.10.0 carries `CriterionResult.where`). The label of a criterion ends in `where application=dns`, with the label names in alphabetical order, so two criteria that differ only in their filter are two rows, and a criterion with a filter does not pair with the same criterion without one. Criteria that agree in all four fields, which a configuration can state twice, pair in order of appearance.
3. The realism results pair by `metric` and `labels` when the result has a metric, so a runner that words the `property` differently still pairs, and by property otherwise. The Labels column shows the labels as `application=http1, classifier=ndpi`, and the Target column shows both targets when the runs differ.
4. A change is Now fails when a result passed in A and failed in B, and Now passes for the reverse. A pair with a missing value on one side is Differs, and a row that only one run has is Only in run A or Only in run B.
5. The totals are the `RunSummary` fields plus the failed ratio, which the view derives as failed over attempted. The change column shows the difference with a sign and a percentage of A, and the failed ratio shows percentage points. A change from zero shows no percentage.
6. The scenario table lists `RunReport.scenarios`, one row per application and scenario in the order the controller sends, with the counts and a failed ratio that the view derives. The comparison pairs scenarios by application and scenario name, shows `A / B` in each cell, and ends each row with the change of the failed ratio in percentage points, or Only in run A or Only in run B. A report without the property (a controller older than 0.9.0) shows a sentence instead of the table, and the comparison then compares no scenarios. The scenarios do not count in the summary of results that got worse.

A run with no realism report has `realism: []`, and the realism sections then show a sentence. The report endpoint also serves CSV and HTML, which a browser needs the bearer token to download, so the views do not link them. A later package can fetch them with `fetch()` and save the body.

### Mock server

`mock/reports.ts` evaluates the criteria of the configuration against the summary of a finished run with the wording of the controller (`observed 0.02 == 0 does not hold`), and `mock/runs.ts` serves the report through `runs.report`. The mock knows one metric, `sessions.failed_ratio`, so a criterion on any other metric, such as a mix error, has no value. The realism results are fixed, carry `metric` and `labels` as the controller sends them, and a run with failed sessions also fails `response_bytes.ks_statistic`. The mock serves `scenarios` too: it divides the counts of each application among its scenarios by their weights in the mix, the last scenario taking the remainder, so the rows add up to the run summary, and a run before its first tick has an empty list. A criterion with a `where` of `application` is evaluated over the scenarios of that application, and a filter on any other label has no value. Two options of `MockRunOptions` make a passing and a failing run in one server. `failedEvery(ordinal, configId)` sets every how many sessions one fails, and 0 means none, with 50 as the default that earlier tests expect. `verdictStates` makes a run whose criteria fail end in `failed` with the reason of the controller, as the real controller does, and it is off by default, so the scripts of UI-04 still end in `completed`. The report of an active or aborted run has no verdict and no realism results. The mock answers `format=csv` and `format=html` with 501, as the controller does for HTML when no renderer is set. `mock/runs.test.ts` validates every report against the OpenAPI document.

### Tests

`ui/src/pages/Reports.test.tsx` runs the whole UI against the mock and states every number as a literal. It covers a passing run (verdict, criteria, realism, totals, facts), a failing run (reason, failed criterion, failed realism result, totals), a criterion without a value, a run in progress, an aborted run, a missing run, a server error, a rejected token, the links from the run list and the run view, and the comparison: the totals and the changes, the swapped order, runs of different configurations whose criteria only one run has, the two selects with the query string, a link that preselects run A, a missing run, and a run in progress. UI-08 added a report with the scenario table, two criteria that differ only in their filter, a run before its first tick, and comparisons that pair those criteria and the scenarios and mark a scenario that only one run has. Unit tests in `ui/src/reports/compare.test.ts` cover the outcomes, the pairing (by filter, by metric and labels, by scenario), the change words, and the totals. The development sandbox has no browser, so no Playwright test covers the report views yet.

## Agents and ports

UI-06 added the agents view. It uses `listAgents`, `reserveAgent`, and `releaseAgent` and no other endpoint, and UI-09 added the holder of a reservation (see Reservation holders below).

### View

| Path | View | Behavior |
|---|---|---|
| `/agents` | `ui/src/pages/AgentsPage.tsx` | A sentence that counts the agents by state, a table of the agents, and a table of their ports |

The agent table shows the identity, the state (`online`, `reserved`, `busy`, or `offline`), the backend, the cores, the version, the peer address, the labels, and the reservation action. An agent that is `online` has a Reserve button and an agent that is `reserved` has a Release button, and both name the agent (`Reserve agent-local`) so a screen reader user can tell them apart. A `busy` agent shows `In use by a run` and an `offline` agent shows `Not connected`, with no button, because a run holds the first and no control session exists for the second. A field that the agent did not report reads `not reported`. The port table has one row per port with the agent, the port name, the driver, and the speed, which the view writes as Mbit/s below 1000 and as Gbit/s from there. The API has no field that says which test port an agent port serves, so the view shows the ports that each agent offers and does not claim an assignment. The main navigation gained the entry Agents, and the view refreshes the list every five poll intervals (five seconds by default).

### Reservation conflicts

A reserve or release call runs against the state that the controller holds, which may have changed since the list loaded. The view sends the call whatever the list shows and reports the answer. A success puts the returned agent in the list and says `Agent agent-local reserved. It is now reserved.` in a status region. A problem shows `Agent agent-local was not reserved.` and the `detail` of the problem in an alert, and a 409 or a 404 also reads the list again at once, so the stale button changes to the one that fits the real state. The next action clears the message. A 401 or 403 signs the user out as in the other views.

The controller (`controller/internal/agents/rest.go`) reserves for the operator, so it reserves an agent that the operator holds again without error. It answers 409 when a run holds the agent, when the agent is offline (`reserve agent a: agent offline`), or when it holds a plan (`reserve agent a: plan loaded: agent busy`), and 404 for an unknown agent. A release of an agent that is not reserved succeeds. Since UI-09 the OpenAPI document names 404 and 409 for `reserveAgent` (see Reservation holders below), and the view re-reads the list for both.

### Reservation holders (UI-09)

REST OpenAPI 0.10.0 (CON-22) added `Agent.reserved_by` and documented 404 and 409 for `reserveAgent`, and `ui/src/api/schema.d.ts` already carries both. The agent table has a column Held by, which reads `reserved_by` through `holderOf` in `ui/src/pages/agentFormat.ts`.

| `reserved_by` | Column Held by | Reservation column |
|---|---|---|
| absent, any state but `reserved` | `none` | as before |
| absent, state `reserved` (a controller older than 0.10.0) | `not reported` | Release |
| `operator` | `operator` | Release when the state is `reserved` |
| a run id, such as `run_mock0007` | the link `Run run_mock0007` to `/runs/run_mock0007` | `Held by a run` when the state is `reserved`, with no button |

The field counts in every state, so an agent that is `busy` or `offline` still shows its holder, because the state takes precedence over `reserved` while the reservation exists. The view offers no Release for an agent that a run holds: the controller ends any reservation on release, which would take the agent from the run. A 409 from `reserveAgent` shows `Agent <id> was not reserved because of a conflict.` followed by the `detail` of the problem, which names the reason. The controller checks them in this order: another holder (`reserve agent a for operator: held by run_x: agent reserved by another holder`), no control session (`reserve agent a: agent offline`), and a plan that is not finished (`reserve agent a: plan loaded: agent busy`). The list is read again at once, so the Held by column then shows the run that took the agent. A reservation for the operator that the operator already holds answers 200. Other problems read `Agent <id> was not reserved.` and the detail, as before.

The mock sets `reserved_by` to `operator` on reserve and clears it on release (which, as in the controller, ends any holder), answers 409 in the order above, and leaves a refused agent unchanged. `mock/agents.test.ts` validates the `Agent` bodies, the list, and all five 409 cases against the OpenAPI document with Ajv. `ui/src/pages/Agents.test.tsx` has a group Reservation holders with literal agents for the operator, a run, a busy agent held by a run, and an offline agent held by the operator, a conflict with a run that took the agent after the list loaded, an offline conflict, and axe scans of the listing and both conflicts.

### Mock server

The mock now keeps the agent states per server and follows the controller. `POST /agents/{id}:reserve` moves an `online` agent to `reserved`, leaves a `reserved` agent as it is, and answers 409 for an `offline` or `busy` agent with the detail of the controller. `POST /agents/{id}:release` moves a `reserved` agent to `online`, leaves any other agent as it is, and answers 200. An unknown agent answers 404. `GET /capabilities` shows the current agent states, and `RunningMockServer.agents` gives tests the live list, so a test can take an agent offline or make it busy after the view loaded it. `mock/agents.test.ts` validates every response against the OpenAPI document with Ajv.

### Tests

`ui/src/pages/Agents.test.tsx` runs the whole UI against the mock with four agents in four states and states every value as a literal. It covers the table and the port table, the summary sentence, the empty and error states, the navigation entry, reserve then release, release of a reserved agent, a second reservation by another caller, two reservation conflicts (the agent went offline, the agent started a run) where the alert shows the controller's detail and the list refreshes to the new state, a problem that names a holder (injected as a fault, because the controller has no such case), a 404 that drops the agent from the list, the clearing of an old message, the absence of buttons for busy and offline agents, and a rejected token. Axe-core scans the document in the listing, the empty, the success, and the conflict states, with the contrast rule off because jsdom has no layout. Unit tests in `ui/src/pages/agentFormat.test.ts` cover the speed and label formats. The sandbox has no browser, so no Playwright test covers the view.
