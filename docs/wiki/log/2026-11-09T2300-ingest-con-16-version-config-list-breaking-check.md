## [2026-11-09] ingest | CON-16 controller version, configuration listing, and REST breaking check

The OpenAPI document moves to 0.7.0, additively. `Capabilities` gains the optional `controller_version`, and `GET /configs` (`listConfigs`) lists the stored configurations with `page_size` and `page_token` like `listRuns`. The comment of `MixEntry.weight` says "session share"; the protocol stays at 0.2.3 because a comment changes no wire behavior. The new workflow `openapi-breaking.yml` runs oasdiff on the OpenAPI documents against the base and fails on a breaking change, and `tools/contracts/test_openapi_breaking.py` proves it with a deliberately breaking fixture. See [Contracts](../Contracts.md#version-configuration-listing-and-breaking-check) and `contracts/changes/CON-16.md`.

### Shared page notes

- Testing: the REST documents now have an automatic breaking check (`openapi-breaking.yml`, oasdiff `v1.11.7` built with `go install`). The line that says the repository has none, in the CTL-34 and CON-14 entries, is stale.
- Controller page: `listConfigs` answers 501 until a CTL package serves it, and `controller_version` stays absent until the api package takes the build version through an option (CTL-33 finding). CLI page: `fps config list` can use `listConfigs` once the controller serves it.
- CI: the pinned oasdiff version is in `.github/workflows/openapi-breaking.yml` (`OASDIFF_VERSION`). A dependency bot or the coordinator raises it by hand.
- Go code the coordinator regenerates (`api.gen.go`): the stub, `APIVersion` 0.7.0, the `pending` method, and `pendingOps` were already edited by this package.
