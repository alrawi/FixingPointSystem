## [2026-11-08] ingest | CTL-26 Serve the HTML report

`GET /v1/runs/{id}/report?format=html` now answers 200 with the page of `report/html`, the content type `text/html; charset=utf-8`, and `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'`. The handler builds the report as for JSON and adds the per second samples of the run (`report.SampleSource`, set by `NewAPI` from the aggregator) and its objective (`report.ObjectiveSource`, `report.ConfigObjectives` over the configuration store). `report` cannot import `report/html`, so the page comes from a `report.HTMLRenderer` function that `html.Renderer` implements. The [Controller](../Controller.md) page has the details in `HTML report`, section `Serving`.

### Shared page notes

- Wiring: `controller/internal/wiring/controller.go` gained `htmlReport`, called on the `report.API` of `New` and of `SetReport`. `cmd/fps-controller` is unchanged. CTL-27 edits the same package, so expect a small merge in `controller.go`.
- Testing: the API tests are in `controller/internal/report/html/serve_test.go` and run under the existing `lint-ctl-report-html` and `test-ctl-report-html` targets, which cover the subpackage. The `report` package targets cover the rest. No new target.
