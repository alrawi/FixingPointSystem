## [2026-11-11] ingest | CTL-42 the CON-23 mapping table in the controller

`report.MapRealism` now implements the table of `contracts/changes/CON-23.md`. The metrics `realism.check_failures`, `realism.parser_errors`, `realism.decrypted_ratio`, `realism.ja4_match_ratio`, `realism.duplicate_ratio`, `realism.structure_failures`, `realism.transport_deviations`, and the three `mix.*` series of the classifier get the literal target and the outcome that the table gives. Every other metric, and every metric that finds no entry to take its outcome from, keeps the generic rule of CON-22 (`check passed`). The evidence is `docs/handoff/CTL-42.md`. `realism_con23_test.go` maps the 17 cases of `contracts/examples/realism/check-metrics.yaml` to their literal `target` and `passed`.

### Shared page notes

- Controller: `docs/wiki/Controller.md` carries the new table in its section on the realism report. No shared page needs a change.
- Go could not build in the sandbox (no module proxy), so the unit tests ran in the CI of the pull request only.
