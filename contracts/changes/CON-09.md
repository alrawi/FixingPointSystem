# CON-09 Test configuration and manifest schema additions

Contract. Test configuration schema `schemas/test-config.schema.json` and module manifest schema `schemas/module-manifest.schema.json`, applying decision D12 items 7, 9, 10, 14, and 22.

Versions. The test configuration schema moves from v0.1 to v0.2 and the module manifest schema moves from v0.2 to v0.3. Every change is additive except the seed minimum, which now rejects seed 0.

- `timeline.seed` takes a minimum of 1 (item 9). A configuration may leave the seed out, and the controller then chooses a random nonzero seed and records it in the run.
- `network.ports[].location` documents the location rule (item 7). The form `<agent>/<interface>` pins the port to that one agent, and a bare `<interface>` or `*/<interface>` spreads it across every agent that has the interface. The pattern now accepts the two spread forms.
- A criterion takes an optional `where` object of label names and string values (item 14). The criterion then checks only the series whose labels match, for example `where: { application: dns }`.
- The new top level `modules` object holds `modules.<name>.settings` for each module (item 22). These settings hold for every scenario of the module in the test.
- The manifest adds `settings`, a JSON Schema for the module settings with an object root, like `parameters` (item 22). The example `dns` manifest describes `zone_file` with it.
- The manifest adds `expected_bytes`, the estimated application payload bytes per session of each scenario with default parameters, keyed by scenario name (item 10). The controller derives rates for `throughput_bps` from it and corrects them by calibration.
- The example manifests of `dns` and `http1` carry `expected_bytes`, so their module digests in the example plan `plans/http_dns_smoke.json` changed. The value for `http1/web_get` equals the 17,244 bytes that spike S4 measured.
- The new example `examples/tests/seedless_smoke.yaml` has no seed. The examples under `examples/tests-pending/` use pinned and spread ports, `where`, and `modules`. They stay out of `examples/tests/` until CTL-18 and CTL-20 teach the controller compiler these fields, because the controller compiles every example in that directory and rejects unknown keys.
