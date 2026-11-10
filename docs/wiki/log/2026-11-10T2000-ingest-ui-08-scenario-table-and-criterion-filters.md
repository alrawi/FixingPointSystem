## [2026-11-10] ingest | UI-08 Scenario table, criterion filters, and realism results in the report views

The report view shows the scenario table of `RunReport.scenarios`, with the counts and the failed ratio of each application and scenario. The comparison view puts the scenarios of two runs side by side, pairs them by application and scenario name, and ends each row with the change of the failed ratio. Criteria now pair by metric, operator, value, and `where`, so two criteria that differ only in their label filter are two rows in both views, and the label of a criterion ends in `where application=dns`. Realism results pair by `metric` and `labels` and by property when a result has no metric, and both views show a Labels column. A report without `scenarios` (a controller older than 0.9.0) shows a sentence in place of the table.

The mock serves `scenarios` (the counts of an application divide among its scenarios by weight, so the rows add up to the run summary), `where` on criteria (evaluated over the scenarios of one application for the filter `application`), and `metric` and `labels` on the realism results. `mock/runs.test.ts` validates the new fields against the OpenAPI document with Ajv. Component tests with literal values cover a report with scenarios, two criteria that differ only in `where`, a comparison of those, and a scenario that only one run has. axe scans all of them. No dependency was added. We described the change in [Web UI](Web-UI.md).

### Shared page notes

- Testing: no change. The new cases run inside `make test-ui` (`ui/src/pages/Reports.test.tsx`, `ui/src/reports/compare.test.ts`, `ui/mock/runs.test.ts`).
- Contracts: no change. The REST 0.10.0 fields were enough.
- Implementation Plan: UI-08 is done once merged. UI-09 (reservations in the agents view) is untouched.
