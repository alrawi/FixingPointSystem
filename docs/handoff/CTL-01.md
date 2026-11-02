# Handoff CTL-01 Controller skeleton

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #39 |
| Branch | `wp/CTL-01` |
| Pull request | #70 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-ctl01-a67c, lease until 2026-11-02T21:47:09Z |
| Last verified commit | e573827 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We create the Go module `github.com/alrawi/FixingPointSystem/controller` with the `fps-controller` binary. The binary reads flags and a YAML configuration file, logs JSON through `log/slog`, serves `/healthz`, and shuts down gracefully on SIGTERM. A golangci-lint configuration, a path filtered CI workflow, a `test-ctl` make target, and the `go.work` line complete the package.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Create the module, the configuration loader in `internal/app`, and its unit tests
- [x] 3. Add the HTTP server with `/healthz`, the run loop with graceful shutdown, and component tests
- [x] 4. Add `cmd/fps-controller` with flags, signal handling, and its tests
- [x] 5. Add `.golangci.yml`, the `go.work` line, the make targets, and `.github/workflows/controller.yml`
- [x] 6. Ingest the result into the wiki, pass the tests and CI, and finish

## Done and verified

Each step was verified with the command after it.

- Steps 2 to 4. `cd controller && go test -race -count=3 ./...` passes, and `go test -cover ./...` reports 96.2% for `internal/app` and 85.2% for `cmd/fps-controller`.
- Step 5. `make lint-ctl test-ctl test-ops-gowork` passes with golangci-lint v2.12.2 installed, reporting 0 issues, and `golangci-lint config verify` accepts `.golangci.yml`.
- Step 6. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass.

## Remaining

Nothing in scope. Two items belong to later packages. TLS on the listener has no package yet, and CTL-02 mounts the `/v1` API with `App.Handle`.

## Next action

A reviewer agent reviews #70 against the Definition of Done.

## Blockers and open questions

None.

## Decisions taken in this package

- The YAML library is `go.yaml.in/yaml/v3` v3.0.4 under MIT and Apache-2.0, because the standard library has no YAML decoder and `gopkg.in/yaml.v3` is archived. `gopkg.in/check.v1` appears in `go.sum` only as a test dependency of that library.
- The decoder rejects unknown keys, so a misspelled key fails at start up instead of being ignored.
- Layers apply in the order defaults, file, flags, and only flags that appear on the command line override the file.
- The default listen address `localhost:8443` follows the default server of the OpenAPI contract. The server speaks plain HTTP for now.
- `/healthz` sits outside `/v1`, needs no token, and answers only GET.
- Request contexts derive from `context.WithoutCancel`, so a stop signal does not cancel in flight requests during the drain.
- `.golangci.yml` excludes gosec G204 in test files only, because the integration test builds and starts the binary. There are no inline `nolint` or `nosec` comments.
- The workflow file is `.github/workflows/controller.yml` at the owner's request, with the workflow name `ctl`. It runs gofmt and go vet directly, golangci-lint through `golangci/golangci-lint-action` pinned to the v9.2.0 commit, and `make test-ctl`.
- The package also changed `controller/go.sum`, the root `Makefile` CTL block, and the `go.work` use line, which OPS-01 assigns to the package that creates `controller/`.

## Pitfalls

- golangci-lint v2.12 and later need Go 1.25 or newer to build. We installed it locally with `GOTOOLCHAIN=go1.25.14 go install github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.12.2`. The module itself stays on Go 1.24.
- `go test ./...` inside `controller/` fails until `go.work` lists `./controller`.

## Verify the current state

```bash
make lint-ctl test-ctl    # expect 0 golangci-lint issues and two ok packages
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:04:21Z | claude-ctl01-a67c | claim | none | Claimed #39 with card 6587193148, branch created from `origin/main` at 2921645 |
| 2026-11-02T20:17:09Z | claude-ctl01-a67c | checkpoint | e573827 | Steps 2 to 5 done, every CI job passed on this commit including `ctl` |
| 2026-11-02T20:18:50Z | claude-ctl01-a67c | finish | see PR | Wiki ingest done, `origin/main` unchanged since 2921645, status ready for review |
