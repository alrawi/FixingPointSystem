# Web UI

The web UI of FixingPointSystem is a static single page application that the controller serves. It talks to the controller only through the REST API in [`contracts/openapi/fps-api-v1.yaml`](../contracts/openapi/fps-api-v1.yaml). The stack and the rules for UI packages are in [docs/README.md](docs/README.md) and decision record [D11](../docs/decisions/0011-web-ui-stack.md).

## Requirements

Building the UI needs Node.js 22.22 or later and npm. Every dependency is pinned to an exact version in `package.json`, and `package-lock.json` locks the whole tree. Install it with `npm ci`, never with `npm install`, unless you change a dependency on purpose.

## Commands

The root Makefile runs the checks through `mk/ui.mk`, and `.github/workflows/ui.yml` runs the same targets in CI.

| Command | Runs |
|---|---|
| `make lint-ui` | Prettier check, ESLint with warnings as errors, `tsc` in strict mode, the client currency check, and the license check |
| `make test-ui` | Vitest unit, component, and contract tests, the production build, and the bundle budget |
| `make e2e-ui` | The Playwright smoke tests with axe-core against the build and the mock server |

The npm scripts behind them work on their own inside `ui/`.

| Script | Purpose |
|---|---|
| `npm run dev` | Vite dev server on port 5173, which forwards `/v1` to `FPS_API_TARGET`, by default the mock server |
| `npm run mock` | The mock controller API on `FPS_MOCK_PORT`, by default 8090 |
| `npm run build` | Type check and production build into `dist/` |
| `npm run preview` | Serves `dist/` and forwards `/v1` like the dev server |
| `npm run gen:api` | Regenerates `src/api/schema.d.ts` from the contract |
| `npm run check:api` | Fails when `src/api/schema.d.ts` differs from a fresh generation |
| `npm run check:licenses` | Fails when a locked package carries a license outside the GPL-3.0 compatible allow list; `-- --list` prints the counts |
| `npm run check:bundle` | Fails when the initial JavaScript of `dist/` passes 250 kB gzip |
| `npm test` | Vitest once |
| `npm run e2e` | Playwright, after `npx playwright install chromium-headless-shell` |

To try the UI against the mock server, run `npm run mock` and `npm run dev` in two terminals, open http://localhost:5173, and sign in with the token `fps-mock-token`.

## Layout

| Path | Contents |
|---|---|
| `src/api/schema.d.ts` | Types generated from the contract. Never edit it by hand |
| `src/api/client.ts` | The typed client that every component uses, with the bearer token middleware and `ApiError` |
| `src/api/eventStream.ts` | The `fetch()` reader for `text/event-stream`, with `Last-Event-ID` reconnection and token expiry |
| `src/auth/` | The session that holds the token in `sessionStorage`, and the sign in page |
| `src/layout/` | The page frame with the skip link, banner, main navigation, and main region |
| `src/pages/` | One file per view |
| `mock/` | The mock controller API, built from `contracts/examples/` and the schema examples |
| `scripts/` | The generator, the license check, and the bundle check |
| `e2e/` | Playwright smoke tests |

## Sign in

The user pastes an API token. The UI checks it with `GET /capabilities` before it keeps it, and keeps it in `sessionStorage`, so the token ends with the browser tab. A 401 or 403 answer to any later call signs the user out with a notice.

## Mock server

`mock/server.ts` serves `GET /capabilities`, `/modules`, `/modules/{name}/{version}`, `/agents`, and `/runs` over `node:http` and accepts the bearer token `fps-mock-token`. Its data comes from the contract examples. The modules and their scenarios come from `contracts/examples/modules/`, the agent ports from the network of `contracts/examples/tests/http_smoke.yaml`, and the version strings from the `examples` of the `Capabilities` schema. The contract tests in `mock/contract.test.ts` validate every response against the response schema of its operation, so the mock cannot drift from the contract. Later packages add routes as their views need them.
