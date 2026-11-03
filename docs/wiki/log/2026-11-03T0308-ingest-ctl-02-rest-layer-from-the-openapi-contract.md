## [2026-11-03] ingest | CTL-02 REST layer from the OpenAPI contract

We added the package `controller/internal/api/`, which serves every operation of `contracts/openapi/fps-api-v1.yaml` under `/v1`. The types and the server interface come from oapi-codegen v2.6.0, the operation table with each `x-required-scope` comes from the same contract, and a test fails when either generated file is stale. Bearer tokens carry the nested scopes read, run, and admin, every error is an RFC 9457 problem, and `api.Unimplemented` answers each operation with a 501 problem until its package implements it. The contract test calls every operation with five kinds of token and validates each request and response against the contract with kin-openapi. We added the REST layer section to [Controller](Controller.md). The evidence is `docs/handoff/CTL-02.md` and the tests in `controller/internal/api/`.

## Shared page notes

- [Home](Home.md), Controller row. Change the summary to "The controller module, its flags, configuration file, logging, health endpoint, REST layer, and checks".
- [Testing](Testing.md), the CTL rows. `make test-ctl` now also runs the contract test of the REST layer and the check that the generated code in `controller/internal/api/` matches the contract.
- Follow up for the CTL workstream. The binary does not mount the API yet, because the controller configuration has no token setting. The package that adds `tokens` to `controller/internal/app` (CTL-01 paths) also adds one line to `controller/cmd/fps-controller/main.go` after `app.New`, namely `api.New(api.Unimplemented{}, tokens, logger).Mount(a)`, or the implementation that embeds `api.Unimplemented`.
- Follow up for the CON workstream. The contract writes enums such as `RunState` without a type, which OpenAPI 3.1 allows. Adding `type: string` to those enums would let the code generator read the contract without the rewrite in `controller/internal/api/contract/`.
