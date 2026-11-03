## [2026-11-03] ingest | UI-00 Web UI stack decision

We proposed the web UI stack as decision record [D11](https://github.com/alrawi/FixingPointSystem/blob/main/docs/decisions/0011-web-ui-stack.md) with status Proposed and added it to the decision index. The record compares React 19 with Vite, Svelte 5 with SvelteKit 3 in static mode, and Vue 3.5 with Vite against license, bundle size, accessibility, OpenAPI 3.1 client generation, live charts, test tools, and maintenance health, with sources from the npm registry and project documentation. It proposes React 19 with Vite 8, TypeScript 6 in strict mode, React Aria Components, openapi-typescript with openapi-fetch, TanStack Query, uPlot, Vitest, and Playwright. We measured baseline bundles of 68.7 kB, 24.9 kB, and 12.8 kB gzip for React, Vue, and Svelte, and openapi-typescript generated types from `contracts/openapi/fps-api-v1.yaml` without warnings. `ui/docs/README.md` lists the stack and the rules that UI packages follow once the owner accepts the record. We created the component page [Web UI](Web-UI.md). The handoff is `docs/handoff/UI-00.md`, and the package waits for owner approval.

### Shared page notes

- Home and the sidebar should list [Web UI](Web-UI.md) under Engineering.
- The TypeScript section of [Coding Standards](Coding-Standards.md) says the stack is pending UI-00. Once the owner accepts D11, it should name React 19, Vite 8, and TypeScript 6 in strict mode and link the record.
- The specification decision table should gain a D11 row once the owner decides.
