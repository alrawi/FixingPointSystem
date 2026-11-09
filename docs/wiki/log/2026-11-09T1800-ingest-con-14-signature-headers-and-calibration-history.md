## [2026-11-09] ingest | CON-14 signature headers and calibration history

The OpenAPI document moves to 0.5.0. `installModule` names the optional headers `X-Module-Signature-Key-Id` and `X-Module-Signature` and the 400 and 403 answers that CTL-28 already gives, and `Run` gains the optional `calibration` array of `CalibrationStep` objects that CTL-24 and CTL-12 fill. The change is additive. The Python SDK and the UI schema are regenerated, and the controller maps the steps in `runs.ToAPI`. The coordinator regenerates `controller/internal/api/api.gen.go` on the branch, because the header parameters add `InstallModuleParams` to the `InstallModule` signature. See [Contracts](../Contracts.md#signature-headers-and-calibration-history).

### Shared page notes

- Controller page: the section "Passing the signature" says that the OpenAPI contract does not name the headers yet. It does now, from 0.5.0.
- Implementation Plan: CON-14 of phase 2 is done once merged. CON-15 follows on the same files.
- Testing: the repository has no automatic OpenAPI breaking check, only `buf breaking` for the proto files. A tool such as oasdiff in CI would check the additive rule for the REST contract.
- Paths outside the catalog entry: `controller/internal/api/capabilities.go` (version constant), `controller/internal/api/unimplemented.go` and `controller/internal/registry/http.go` (new `InstallModule` argument), `controller/internal/runs/rest.go` and `rest_calibration_test.go` (mapping and its test), `sdk/python/fixingpoint/_generated.py`, and `ui/src/api/schema.d.ts`.
