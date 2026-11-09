# Handoff CTL-26 Serve the HTML report

| Field | Value |
|---|---|
| Issue | #250 |
| Branch | `wp/CTL-26` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-c26a7e, lease until 2026-11-09T04:07:27Z |
| Last verified commit | none |

## Goal

`GET /v1/runs/{id}/report?format=html` serves the page of `report/html` with the HTML content type and a Content-Security-Policy of `default-src 'none'; style-src 'unsafe-inline'`, built from the same report as JSON plus the per second samples and the objective of the run.

## Plan

- [ ] 1. report.API gets a sample source, an objective source, and an HTML renderer hook (report imports no html because html imports report); html provides the renderer.
- [ ] 2. API test in package html (reuses the unexported no external requests checker), 404 test, wiring of the renderer and the configuration store.
- [ ] 3. Controller wiki page, log entry, checks, PR, CI.

## Done and verified

Nothing yet.

## Remaining

All steps.

## Next action

Implement step 1 in `controller/internal/report/rest.go`.

## Blockers and open questions

none

## Decisions taken in this package

- Import cycle: `report/html` imports `report`, so `report.API` cannot call `html.Render`. `report.API` holds a `HTMLRenderer` func that `html.Renderer` implements.

## Pitfalls

Go cannot build locally; CI is the build loop.

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08 | claude-c26a7e | claim | none | claimed #250 |
