## [2026-11-09] ingest | OPS-05 release workflow

The workflow `.github/workflows/release.yml` builds the agent (static musl), the controller (CGO off), one archive per module that has a manifest, and the Python sdist and wheel from a tag `v*` or a manual run. It writes `SHA256SUMS` and release notes from `contracts/CHANGELOG.md` and the merged pull request titles since the previous tag. A real tag creates the GitHub release in a separate `publish` job with `contents: write`, and a dry run (the default for manual runs) keeps the files as the workflow artifact `release-<version>`. The [Operations](../Operations.md#release-workflow) page describes it and how the owner attaches the VM image.

### Shared page notes

- [Testing](../Testing.md): `.github/workflows/release.yml` is not covered by `make` targets. `actionlint` passes on it, and a dry run is the check (workflow_dispatch).
- Controller: `fps-controller -version` reads the Go build info and has no variable for `-ldflags -X`, so a release cannot stamp a version into it. A CTL package could add `var version = "dev"` in `cmd/fps-controller` and a `-X main.version=...` flag in the release workflow.
- Python SDK: `sdk/python/pyproject.toml` has version 0.1.0 and is not tied to the tag. The release job only warns on a mismatch.
- `modules/tls/` has no `manifest.yaml` and is skipped with a notice until it has one.
- Implementation Plan and Swarm Protocol: the owner decides when to cut the first release candidate tag.
