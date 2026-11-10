## [2026-11-10] ingest | CTL-38 Distributions in the compiler and scenario counts in the run report

The compiler's strict decoder accepts the `arrival` and `size` objects of test configuration v0.3 (CON-19) on `profile` and on every mix entry, so a configuration that passes the store's schema no longer fails at the compiler. The declarations are targets and add no command: a test with them compiles to the same plans as the test without them, and `Config.Declarations()` carries them to the report (the arrival of an entry is its own, and the size of an entry overrides `profile.size` by direction). The registry types `int_bytes` as bytes and `now` by its literal format, as scenario contract v0.6 says. The run report gets a scenarios table with the sessions and bytes of each scenario, which the RLM-07 mix check reads. The table is in `Report.Scenarios`, `Report.FileJSON()`, and the CSV report, and the REST report keeps its schema until CTL-39. We described the changes in [Controller](Controller.md).

### Shared page notes

- Realism: the mix check can read `Report.FileJSON()` (the table under `scenarios`) as its run report, in the table form that `realism/mix.py` already accepts. The REST report still holds totals only.
- Implementation Plan: CTL-38 is done once merged. CTL-39 can add `Report.Scenarios` to the REST report after CON-21, and can write `Report.Declared` into the realism results.
- Testing: none.
