# Web UI stack

This directory documents the stack of the web UI. The owner accepted the stack in decision record [D11](../../docs/decisions/0011-web-ui-stack.md) on 2026-11-03, and every UI package follows it. The [UI README](../README.md) lists the commands.

## Stack

The UI is a static single page application in TypeScript that the controller serves. It talks to the controller only through the public REST API in `contracts/openapi/fps-api-v1.yaml`, as decision D8 requires.

| Concern | Choice | License |
|---|---|---|
| Language | TypeScript 6.0 in strict mode | Apache-2.0 |
| Framework | React 19 | MIT |
| Build | Vite 8 with @vitejs/plugin-react | MIT |
| Routing | React Router in declarative mode | MIT |
| Accessible primitives | React Aria Components | Apache-2.0 |
| API client | Types from openapi-typescript, calls through openapi-fetch | MIT |
| Server state | @tanstack/react-query | MIT |
| Live charts | A small SVG line chart in `src/runs/LineChart.tsx`, written for the project. D11 names uPlot, but UI-04 found that its charts of a few hundred points need no library and that jsdom has no canvas to test one | GPL-3.0 |
| Event stream | A local `fetch()` reader for `text/event-stream` | GPL-3.0 |
| Lint and format | ESLint 10, typescript-eslint, eslint-plugin-jsx-a11y, Prettier | MIT |
| Unit and component tests | Vitest and @testing-library/react | MIT |
| Mock API | A local `node:http` server built from the contract examples | GPL-3.0 |
| Browser end to end | Playwright with @axe-core/playwright | Apache-2.0 and MPL-2.0 |

## Rules for UI packages

1. Components call the API only through the generated client. A CI check regenerates the types from the contract and fails on a diff.
2. The live metrics view reads `GET /runs/{id}/metrics:stream` with `fetch()`, because `EventSource` cannot send the bearer token. Samples go into a ring buffer outside React state (`src/runs/metricsStore.ts`), and the store tells the page once per flush, so the page renders the charts once per tick and not once per event.
3. Interactive widgets come from React Aria Components or follow the matching WAI-ARIA pattern. Every Playwright test runs an axe-core check and fails on any violation.
4. The linter runs with warnings as errors and includes the accessibility rules of eslint-plugin-jsx-a11y.
5. The build fails when the initial JavaScript passes 250 kB gzip.
6. Every new dependency carries a license that is compatible with GPL-3.0, and `npm run check:licenses` fails CI otherwise.
7. Building the UI needs Node.js 22.22 or later. `ui/package.json` pins every version exactly, and `ui/package-lock.json` locks the tree.

## Packages

| Package | Scope |
|---|---|
| UI-01 | Project skeleton, generated client, token sign in, layout, mock API server |
| UI-02 | Module library view |
| UI-03 | Test configuration editor with schema validation |
| UI-04 | Run launch and live metrics from the event stream |
| UI-05 | Reports and run comparison |
| UI-06 | Agents and ports view |
| UI-07 | End to end suite against a real controller with fake agents |
