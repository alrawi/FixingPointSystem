# CON-16 Controller version, configuration listing, and REST breaking check

Contract. Controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`, a comment of the agent protocol `proto/fps/agent/v1/agent.proto`, and a new CI check for the REST documents. The change takes findings of CTL-33, CLI-02, CTL-34, and CON-14. The owner approved it.

Versions.

- OpenAPI moves from 0.6.0 to 0.7.0. The change adds an operation and an optional response field, which a reader can see in the document, so it takes a minor number, as CON-12, CON-14, and CON-15 did. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The agent protocol stays at 0.2.3. The edit of the `MixEntry.weight` comment changes no field, no number, no type, and no behavior of any peer: the compiler already writes the session share since CTL-34, and the engine passes the field through. A version number says what a peer must implement, and nothing changes for an agent or a controller, so a bump would only make the header comment, `buf.yaml`, `contracts/README.md`, the Rust constant `ENGINE_API` and its test move for no reason. The controller admits agents by the major and minor part of `engine_api`, so a minor bump would even be harmful. `buf breaking` ignores comments.

REST changes.

- `Capabilities` gains the optional `controller_version` (string), the version of the controller build, such as a release number. It is not the REST contract version, which `api_version` keeps. The field is absent when the build carries no version. The api package takes the value through an option (CTL-33 finding).
- New operation `listConfigs`, `GET /configs`, scope `read`, tag `configs`. It takes the shared parameters `page_size` (1 to 500, default 50) and `page_token`, as `listRuns` does, and answers 200 with `{items: ConfigRef[], next_page_token}`, newest first. `createConfig` stays on the same path. The CLI-02 finding asked for it, because `fps config list` shows the uploads of the local machine only.

Protocol changes. The comment of `MixEntry.weight` says "session share" (between 0 and 1, the shares of a plan sum to 1, and the compiler converts byte weights of the bytes basis). It replaces "normalized weight within the basis", which no longer describes what the compiler writes. `contracts/README.md`, plan rule 3, says the same.

CI. The new workflow `.github/workflows/openapi-breaking.yml` runs on `pull_request` and on `push` to `main` for `contracts/openapi/**` (and for its own files). It runs on `ubuntu-24.04` with `permissions: contents: read` and no secret. It builds oasdiff from the pinned module version `v1.11.7` with `go install`, which the Go toolchain verifies against the public checksum database, then runs the fixture tests and compares each OpenAPI document with the version in the base commit (`tools/contracts/check_openapi_breaking.py`, which runs `oasdiff breaking --fail-on ERR`). The fixtures are `tools/contracts/fixtures/openapi-breaking/` (`base.yaml`, the additive `additive.yaml`, and the deliberately breaking `breaking.yaml`), and `tools/contracts/test_openapi_breaking.py` asserts that the check passes the first pair and fails the second with exit code 1 and the expected rules. The test is skipped where oasdiff is not installed (the `contracts` job of `ci.yml`), and a failure in the workflow, where `FPS_REQUIRE_OASDIFF` is set. This change also passes the check against the base, because it adds only.

Compatibility. The change is additive. A client that ignores `controller_version` and `listConfigs` sees no difference. The controller answers `listConfigs` with 501 until a CTL package serves it.

Generated code. `sdk/python/fixingpoint/_generated.py` (`list_configs`, `Capabilities.controller_version`) and `ui/src/api/schema.d.ts` are regenerated.

Outside the catalog paths. Go, for the coordinator: `controller/internal/api/api.gen.go` is regenerated with `go generate ./internal/api`. This change already adds what compiles after that: `ListConfigs` in `controller/internal/api/unimplemented.go` (501 through `notImplemented`), `APIVersion` 0.7.0 in `capabilities.go`, `ListConfigs` on `pending` in `controller/internal/wiring/server.go`, and `listConfigs` in `pendingOps` of `wiring/wiring_test.go`. Serving `listConfigs` and filling `controller_version` are CTL work, and `fps config list` using the operation is CLI work.
