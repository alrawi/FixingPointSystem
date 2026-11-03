# D11. Web UI stack

- Status: Accepted (owner, 2026-11-03)
- Date: 2026-11-03
- Deciders: repository owner

## Context

Decision D8 puts a full web UI in v1, built only on the public REST API in `contracts/openapi/fps-api-v1.yaml`. That contract uses OpenAPI 3.1, authenticates every call with a scoped bearer token, and streams live metrics from `GET /runs/{id}/metrics:stream` as Server Sent Events with one `MetricSample` per second per view. Packages UI-01 to UI-07 need one language, framework, build tool, and set of test tools before any code exists. The Coding Standards already fix four rules for the UI. The compiler runs in strict mode, a formatter and a linter run in CI with warnings as errors, components call the API only through the generated client, and tests run against a mock server built from the contract examples.

We compare three stacks. All three use TypeScript, Vite, the same generated client, the same chart library, and Playwright, so the comparison isolates the framework and its ecosystem. We checked versions, licenses, and release dates against the npm registry on 2026-11-03. We measured bundle sizes ourselves by building the same small component with Vite 8.3.5 for each framework. The component holds a counter, opens an event stream, and renders a keyed list.

## Options

- React 19 with Vite and TypeScript (proposed)
- Svelte 5 with SvelteKit 3 in static mode and TypeScript
- Vue 3.5 with Vite and TypeScript

### Comparison

| Criterion | React 19 with Vite | Svelte 5 with SvelteKit static | Vue 3.5 with Vite |
|---|---|---|---|
| Current version | react 19.3.1, released 2026-10-14 [1] | svelte 5.57.3 and @sveltejs/kit 3.1.1, kit 3 stable since October 2026 [1][2] | vue 3.5.44, with 3.6 still at release candidate [1][3] |
| Core license | MIT | MIT | MIT |
| Measured baseline bundle, gzip | 68.7 kB | 12.8 kB | 24.9 kB |
| Accessible component primitives | React Aria Components 1.22.1, Apache-2.0, from Adobe [1][4] | Bits UI 2.20.3, MIT [1][5] | Reka UI 2.11.0, MIT [1] |
| Accessibility linting | eslint-plugin-jsx-a11y, MIT | Compiler `a11y_*` warnings on every build [6] | eslint-plugin-vuejs-accessibility, MIT |
| OpenAPI 3.1 client | openapi-typescript and openapi-fetch, plus openapi-react-query [7] | openapi-typescript and openapi-fetch | openapi-typescript and openapi-fetch |
| Query cache | @tanstack/react-query 5, MIT | @tanstack/svelte-query 6, MIT | @tanstack/vue-query 5, MIT |
| Live chart | uPlot, imperative updates through a ref | uPlot through an action, or LayerChart 2, MIT | uPlot through a template ref |
| Unit tests | Vitest 5 and @testing-library/react | Vitest 5 and @testing-library/svelte | Vitest 5 and @testing-library/vue |
| Browser end to end | Playwright 1.64, Apache-2.0, with @axe-core/playwright | the same | the same |
| Weekly npm downloads of the core package | 244.6 million | 9.1 million | 24.7 million |
| Weekly downloads of the primitive library | 8.2 million | 1.6 million | 2.9 million |

### Criteria common to all three

**License compatibility with GPL-3.0.** Every package we name carries MIT, Apache-2.0, or MPL-2.0, and the FSF lists all three as compatible with GPL-3.0 [8]. TypeScript, React Aria, ECharts, Playwright, and Prism use Apache-2.0. axe-core and @axe-core/playwright use MPL-2.0, which we run only as a test tool and never ship. No option forces a license exception.

**OpenAPI 3.1 client generation.** openapi-typescript 7.13.0 states support for OpenAPI 3.0 and 3.1 [7]. We ran it on `contracts/openapi/fps-api-v1.yaml`, and it produced 1261 lines of types in 0.4 s without warnings. openapi-fetch 0.17.0 adds 2.8 kB gzip and calls the API through those types, so the generator does not depend on the framework. The UI therefore keeps no hand written request types, and a currency check in CI regenerates the types and fails on a diff.

**Live metrics over Server Sent Events.** The browser `EventSource` interface cannot send an `Authorization` header, and the W3C closed the request for custom headers as WONTFIX in favor of `fetch()` with streams [9]. Every option therefore reads the stream with `fetch()` and a small parser of the `text/event-stream` format, which keeps the bearer token out of the URL and out of server logs.

**Charts for live metrics.** uPlot 1.6.32 is MIT licensed, adds 22.5 kB gzip in our build, and targets fast time series with release notes on streaming performance [10]. It draws to a canvas and takes new data through an imperative call, so the framework never re-renders the chart once per second. Its last release dates from 2025-03-14, which reflects a mature library more than an abandoned one at 0.96 million weekly downloads, and we can fall back to Apache ECharts 6.1.0 under Apache-2.0 with the same integration pattern.

**Build tool.** Vite 8 shipped on 2026-03-12 with Rolldown as its single bundler and needs Node.js 20.19 or 22.12 and later [11]. The current release is 8.3.5. SvelteKit 3 builds on Vite as well and raises the Node.js minimum to 22 and the TypeScript minimum to 6 [2].

## Decision

We propose React 19 with Vite 8 and TypeScript in strict mode, built as a static single page application that the controller serves. The stack has these parts.

| Concern | Choice | License |
|---|---|---|
| Language | TypeScript 6.0, pinned until typescript-eslint supports 7 | Apache-2.0 |
| Framework | React 19 and react-dom 19 | MIT |
| Build | Vite 8 with @vitejs/plugin-react | MIT |
| Routing | React Router in declarative mode | MIT |
| Accessible primitives | React Aria Components | Apache-2.0 |
| API client | openapi-typescript types and openapi-fetch | MIT |
| Server state | @tanstack/react-query | MIT |
| Live charts | uPlot | MIT |
| Event stream | A local `fetch()` reader for `text/event-stream`, no dependency | GPL-3.0 |
| Lint and format | ESLint 10, typescript-eslint, eslint-plugin-jsx-a11y, Prettier | MIT |
| Unit and component tests | Vitest and @testing-library/react | MIT |
| Mock API | Prism from the contract examples, or MSW in unit tests | Apache-2.0 and MIT |
| Browser end to end | Playwright with @axe-core/playwright | Apache-2.0 and MPL-2.0 |

We give five reasons.

1. Accessibility has the strongest support. React Aria Components implement the WAI-ARIA patterns for tables, dialogs, menus, comboboxes, and date fields, and Adobe tests them across devices and assistive technology [4]. A traffic generator UI consists mostly of forms, tables, and dialogs, which is exactly the area where hand built widgets fail.
2. Maintenance health is the best of the three. React has about seven times the weekly downloads of Vue and Svelte combined, React Aria has five times the downloads of Bits UI, and React, React Aria, Vite, and TanStack Query all released within the last month [1]. A library that loses its maintainer has a React alternative more often than a Svelte or Vue one.
3. The stack is stable now. React 19 has shipped patch releases since December 2024 [12]. SvelteKit 3 reached stable in October 2026 with breaking changes to adapters, and Vue 3.6 with Vapor mode is still a release candidate, so either alternative would start on a moving base.
4. Agents write most of this code. React and its testing tools have the largest body of public examples, which lowers the error rate of generated components and the review load at the gates.
5. The bundle cost is acceptable for this product. The UI loads once per session on a lab network, and 69 kB gzip for the framework is small next to the time a test run takes. We keep the budget explicit and fail the build when the initial JavaScript passes 250 kB gzip.

## Consequences

The UI starts 56 kB gzip heavier than the Svelte option and 44 kB heavier than the Vue option. We accept this cost and guard it with the bundle budget above.

React re-renders from state, so a once per second stream could drive needless renders. UI-04 therefore keeps samples in a ring buffer outside React state and pushes them to uPlot through a ref, and React renders only the summary values.

We lose the compile time accessibility warnings that Svelte gives on every build. eslint-plugin-jsx-a11y in the linter and axe-core checks in every Playwright test replace them, and React Aria covers the widget behavior that no linter checks.

The UI cannot use `EventSource`, because the API uses bearer tokens. UI-01 adds the local stream reader with unit tests for the event format, reconnection with `Last-Event-ID`, and token expiry.

TypeScript stays on 6.0 until typescript-eslint supports TypeScript 7, whose current peer range stops below 6.1 [1]. UI-01 records the pinned versions in `ui/package.json` and a lock file.

Contributors need Node.js 22 to build the UI. The controller and the agent do not need it at run time, because the controller serves the built files.

The rejected options carry these costs. Svelte gives the smallest bundle and built in accessibility warnings, but SvelteKit 3 is weeks old, its primitive library has a fifth of the users of React Aria, and static mode adds a framework layer that a single page application does not need. Vue sits between the two on size and ecosystem, but its next minor release changes the rendering model, and its accessible primitives have a smaller user base than React Aria.

## Sources

1. npm registry metadata and the npm downloads API, queried on 2026-11-03, for every package named in this record. <https://registry.npmjs.org/> and <https://api.npmjs.org/downloads/point/last-week/react>
2. SvelteKit release notes, releases.sh. <https://releases.sh/svelte/sveltekit.md> and <https://releases.sh/svelte/overview.md>
3. Vue Vapor mode and Vue 3.6 status, DEV Community, 2026-10-06. <https://dev.to/codigoaoponto/vue-vapor-mode-what-it-is-and-what-changes-in-vue-36-2cmc>
4. React Aria Components package page. <https://www.npmjs.com/package/react-aria-components>
5. Bits UI, Getting Started. <https://bits-ui.com/docs/getting-started>
6. Svelte, Accessibility warnings. <https://www.svelte.dev/docs/accessibility-warnings>
7. openapi-typescript, Introduction. <https://openapi-ts.dev/introduction>
8. GPL compatible licenses approved by the FSF, as listed by the Gentoo wiki. <https://wiki.gentoo.org/wiki/License_groups/GPL-COMPATIBLE>
9. W3C bug 25984, EventSource and custom request headers. <https://www.w3.org/Bugs/Public/show_bug.cgi?id=25984>
10. uPlot releases. <https://github.com/leeoniya/uPlot/releases?page=3>
11. Vite 8 announcement. <https://vite.dev/blog/announcing-vite8.html>
12. React release and support status, endoflife.ai. <https://endoflife.ai/article-react-eol.html>

The bundle measurement builds one component per framework with Vite 8.3.5 and the official Vite plugin of each framework, with react 19.3.1, svelte 5.57.3, and vue 3.5.44. The figures are the gzip sizes that `vite build` reports for the single JavaScript chunk.
