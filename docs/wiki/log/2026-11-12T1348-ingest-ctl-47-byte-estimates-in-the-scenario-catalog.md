## [2026-11-12] ingest | CTL-47 byte estimates in the scenario catalog

CTL-47 makes the controller read the manifest field `expected_bytes`. `registry.Manifest` gets `ExpectedBytes` (scenario name to bytes), and `wiring.Catalog.Scenarios` sets `compiler.Scenario.BytesPerSession` of each scenario from it, so `profile.basis: bytes` and the `throughput_bps` objective compile on a real controller. A scenario without an entry keeps 0 and fails with `ErrNoByteEstimate`, as before. The [Controller](../Controller.md) page describes the change and replaces the paragraph that named the gap.

Tests in `controller/internal/wiring/bytes_test.go`:
- `TestCatalogSetsBytesPerSessionFromManifests` compares the catalog values of the five example scenarios with the `expected_bytes` of the example manifests.
- `TestCatalogCompilesBytesBasisAndThroughputFromManifests` compiles `enterprise_mix_10g.yaml` through the real registry and catalog. It checks the session shares of the bytes basis, the 10 Gbit/s of the worker rates, a `sessions` basis variant with `throughput_bps`, and a `sessions_per_second` objective with the bytes basis.
- `TestCatalogWithoutEstimateFailsWithErrNoByteEstimate` installs a copy of `dns` whose manifest lacks `expected_bytes` and expects `ErrNoByteEstimate` naming `dns/lookup` for the three cases that need an estimate, and success for the `sessions` basis with a session rate.

Findings for the coordinator:
- Only 2 of the 15 manifests under `modules/` carry `expected_bytes` (`hls` and `sip_rtp`). The manifests of the first party `dns` and `http1` modules lack it, although the examples of the same names under `contracts/examples/modules/` have it. Until the dependent MOD package adds a measured estimate to every scenario, a test that uses the bytes basis with the other modules fails with `ErrNoByteEstimate` on a real controller, and `FPS_GATE2_BASIS` in `qa-gate2.yml` must stay. The coordinator removes it after that package merges.
- Neither the registry nor `tools/contracts/validate.py` checks that a key of `expected_bytes` names a scenario of the manifest. A misspelled key shows up only as a missing estimate at compile time. A CON or MOD package could add the rule to the validator, and the guide must then say so.
- The QA-05 observation that the run report counts the FTP data connection and the SIP media stream as sessions of their own is not settled by this package. The guide text below says "one session of the scenario" and leaves that case to MOD-26, who owns the module side.
- Not verified locally: `go build` and `go test` (the sandbox has no Go module proxy). `gofmt -l` is clean, and CI runs the tests.

### Shared page notes

- [Module Development](../Module-Development.md), for the coordinator to apply before merge (the page belongs to MOD-26, whose paths include it, and CTL-47 may not edit it). Add the text below to the section on manifest fields, as the entry for `expected_bytes`. If MOD-26 already has a section or table row for the field, merge the two so the page names the field once. Replace nothing else.

  > #### `expected_bytes`: bytes per session of each scenario
  >
  > `expected_bytes` maps the name of each scenario to the estimated application payload bytes that one session of the scenario moves, in both directions together, with the default parameters of the scenario. Each value is an integer of at least 1, and each key follows the identifier pattern of scenario names (`contracts/schemas/module-manifest.schema.json`, key `expected_bytes`). The schema marks the field as optional, but a module that has scenarios needs an entry for each of them.
  >
  > ```yaml
  > scenarios:
  >   - scenarios/web_get.yaml
  >   - scenarios/web_browse.yaml
  > expected_bytes:
  >   web_get: 17244
  >   web_browse: 70279
  > ```
  >
  > **What the controller does with it.** The controller reads the field into the scenario catalog (`controller/internal/wiring/adapters.go`, `registry.Manifest.ExpectedBytes`) and uses it in two places of the compiler (`controller/internal/compiler/`, see the Test compiler section of [Controller](Controller.md)):
  >
  > - `profile.basis: bytes`. The test configuration gives byte weights, and the compiler turns them into session shares with `share_i = (weight_i / bytes_i) / sum_j (weight_j / bytes_j)`, where `bytes_i` is your estimate. Sessions at these shares then carry the byte weights.
  > - `load.objective.type: throughput_bps`. The session rate is `objective / (8 * mean bytes per session)`, with the mean taken over the mix. A wrong estimate makes the run miss the objective by the same ratio until the calibration of the run loop corrects it.
  >
  > **What happens without it.** A scenario without an entry has no estimate. A test that uses the bytes basis, or the `throughput_bps` objective, and mixes that scenario fails to compile with the error `no bytes per session estimate` (`ErrNoByteEstimate`), and the message names the scenario as `<module>/<scenario>`. The controller never falls back to session shares, because that would run another mix than the one the test asks for. A test with the `sessions` basis and a session rate or concurrency objective does not need the field. Scenarios in `scenarios-pending/` are not in the manifest and need no entry.
  >
  > **The estimate is a starting point.** During a run the controller measures the real bytes per session and replaces the manifest estimate of a scenario after 20 pooled sessions (`controller/internal/runs/calibrate/`, the Throughput rate calibration section of [Controller](Controller.md)). A good estimate keeps the first seconds of a run on target, so measure it and do not guess it.
  >
  > **How to measure it.**
  >
  > 1. Run one session of the scenario to its end with the default parameters, in a test of your module, as `modules/hls/tests/scenarios.rs` does.
  > 2. Add the payload bytes of every connection that the session opens, client to server and server to client. `modules/hls/tests/scenarios.rs` does this with `c2s().len() + s2c().len()` over `net.conns()`.
  > 3. Write the result into the manifest as an integer. Round to a figure you can reproduce, since the value is an estimate and the calibration corrects the rest.
  > 4. Keep the estimate true. A test like `default_parameters_move_the_expected_bytes` in `modules/hls/tests/scenarios.rs` reads `expected_bytes` from the manifest, runs each scenario, and fails when the measured bytes differ from the estimate by 2 percent or more. Add the same test to your module, so that a change of a template or a fixture that alters the size of a session also changes the manifest.
  >
  > The schema defines the estimate as application payload bytes. It holds for the default parameters, so a test configuration that sets a size parameter such as `body_size` moves more or fewer bytes than the estimate says, and the calibration absorbs the difference.
  >
  > Neither the registry nor `tools/contracts/validate.py` checks that a key names a scenario of the manifest, so a misspelled key surfaces only as the missing estimate of the scenario that you meant. Check the keys against the `scenarios` list yourself.

- [Controller](../Controller.md): applied in this pull request, because the page is the CTL component page. The paragraph that said the catalog does not read `expected_bytes` now describes the change, and the `wiring.Catalog` row of the interface table names `BytesPerSession`. Nothing is left to apply.
- [Contracts](../Contracts.md): the row of `expected_bytes` in the table of additions names CTL-21 as the reader. Add CTL-47, which reads the field into the scenario catalog.
- [Testing](../Testing.md): nothing new. The tests run with `make test-ctl`.
- [Home](../Home.md) and [Implementation Plan](../Implementation-Plan.md): the Gate 2 criterion still holds only with a waiver until the dependent MOD package adds measured estimates to every manifest and the coordinator removes `FPS_GATE2_BASIS` from `qa-gate2.yml`.
