## [2026-11-11] ingest | CTL-46 one property wording for KS metrics

`report.MapRealism` now words the property of a KS metric as the examples of CON-23 do: `KS statistic of inter_arrival of http1` and `KS p value of request_bytes of http1`. Before, the controller wrote `inter arrival of http1, KS statistic`, which the run report showed to operators. The value of the label `property` keeps its underscore. The evidence is `docs/handoff/CTL-46.md`. `realism_con23_test.go` compares property for the KS metrics and the conformance ratio, and `TestRendererShowsTheKSWordingOfTheMapping` shows the wording on the HTML page.

### Shared page notes

- Controller: `docs/wiki/Controller.md` carries the new wording in its section on the realism report. No shared page needs a change.
- The examples of CON-23 word the other metrics differently from the controller (`failures of the temporal check` against `realism.check_failures of check temporal`, `parser errors of http1 by tshark` against the generic wording). The example test leaves their property out. A follow up package could align them.
- Go could not build in the sandbox (no module proxy), so the unit tests ran in the CI of the pull request only.
