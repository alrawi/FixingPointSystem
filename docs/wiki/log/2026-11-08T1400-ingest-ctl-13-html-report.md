## [2026-11-08] ingest | CTL-13 HTML report

The package `controller/internal/report/html` renders the run report of CTL-09 as one self contained HTML page with a summary, the criteria with observed values, results, and reasons, the realism results, and inline SVG line charts of the per second series. The page has an inline style sheet, follows `prefers-color-scheme`, prints well, and makes no external request. Golden pages for the pass, fail, degraded, and labeled reports and a parser based check for external references guard it. The [Controller](../Controller.md) page has a section `HTML report`.

### Shared page notes

- Controller REST: `GET /v1/runs/{id}/report?format=html` is in the OpenAPI contract and answers 501 from `report.API.GetRunReport`. A follow up in `controller/internal/report/` must call `html.FromReport` and `html.Render`, with samples from `Aggregator.Samples`, and serve the body as `html.ContentType`. CTL-13 owns only the new subpackage, so the handler change is left out.
- Objective: `report.Report` does not hold the load objective, so the page takes it from `html.Options.Objective`. The handler needs the configuration of the run to fill it.
- Realism: the report has an empty `realism` list, and the page shows rows once `Options.Realism` has them.
- Testing: the new Make targets `lint-ctl-report-html` and `test-ctl-report-html` are in `mk/ctl.mk`. `lint-ctl` and `test-ctl` already cover the package.
