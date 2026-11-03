# Web UI

This page describes the web UI of the UI workstream. Decision D8 puts a full web UI in v1 that uses only the public REST API, and packages UI-00 to UI-07 build it in `ui/`.

## Stack decision

UI-00 proposed the stack as decision record [D11](https://github.com/alrawi/FixingPointSystem/blob/main/docs/decisions/0011-web-ui-stack.md), which waits for owner approval. The record compares React 19 with Vite, Svelte 5 with SvelteKit 3 in static mode, and Vue 3.5 with Vite. It proposes React 19 with Vite 8 and TypeScript, React Aria Components for accessible widgets, openapi-typescript and openapi-fetch for the generated client, uPlot for live charts, and Vitest with Playwright for tests. Every named package carries MIT, Apache-2.0, or MPL-2.0, which are compatible with GPL-3.0.

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

The [UI stack README](https://github.com/alrawi/FixingPointSystem/blob/main/ui/docs/README.md) lists the stack and the rules that UI packages follow once the owner accepts D11.
