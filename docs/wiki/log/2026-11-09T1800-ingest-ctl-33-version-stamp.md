## [2026-11-09] ingest | CTL-33 version stamp in fps-controller

`fps-controller` has `var version = ""` in package main. The release workflow builds with `-ldflags "-X main.version=<version>"`, and `-version` and the startup log report the stamped value, with the Go build information as the fallback when the variable is empty. The release job's check step fails when `-version` does not print the stamped version. A test builds the command with `-X main.version=1.2.3-test` into a temporary directory and reads the version back. The [Operations](../Operations.md#release-workflow) and [Controller](../Controller.md) pages describe it.

### Shared page notes

- Contract: `GET /v1/capabilities` does not report the controller version, because `Capabilities` has no field for it and `api_version` is the REST contract version. A CON package could add an optional `controller_version`, and the api package would take the value through an option.
- Testing: `TestVersionStamp` runs `go build` and is skipped with `-short`.
