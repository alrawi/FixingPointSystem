## [2026-11-10] ingest | CTL-39 Scenario totals and realism results in the REST report

`Report.API` now fills the parts that CON-21 added to the REST report. `RunReport.scenarios` carries one entry per application and scenario with the three session counts and both byte directions of the client side, built from the same rows as the CTL-38 table, so the entries add up to `run.summary`. `CriterionResult.where` carries the label filter of each criterion as the configuration states it. `RunReport.realism` stays an empty list, but `Report` has a `Realism` field that `Report.API` maps, so the package that stores realism results (CON-22 and its controller follow up) only has to set it. The file form of the report keeps the CTL-38 table under `scenarios`. We updated the [Controller](../Controller.md) page, and we repaired its heading `Metrics in the binary`, which CTL-38 had merged with a sentence of its own.

### Shared page notes

- [Web UI](../Web-UI.md): the report and comparison views (UI-05 follow up) can read `scenarios` and `where` from the live controller, and `realism` stays empty until the realism delivery package lands.
- [Realism](../Realism.md): `Report.Realism` in `controller/internal/report` is where a delivered realism report becomes `RunReport.realism`. The mix check keeps reading the table form of the file report, and can also read `scenarios` of the REST report (RLM-13).
- Coordinator: the binary wires `report.NewAPI(c.Evaluator, c.Runs, nil)` in `controller/internal/wiring/controller.go`, so a controller without the aggregator as totals source sends an empty `scenarios`. The `cmd/fps-controller` wiring that passes the aggregator gives the real tables.
