# Controller

The controller is the Go process that stores test configurations, compiles them into agent plans, drives runs, and serves the REST API. This page describes what exists today and grows with each CTL package. The module is `github.com/alrawi/FixingPointSystem/controller` in [`controller/`](https://github.com/alrawi/FixingPointSystem/tree/main/controller), and it builds on its own with `GOWORK=off`.

## Layout

The module has two packages after CTL-01.

| Path | Purpose |
|---|---|
| `cmd/fps-controller/` | The binary. It parses flags, loads the configuration, handles SIGINT and SIGTERM, and maps failures to exit codes |
| `internal/app/` | Configuration loading and validation, the JSON logger, the HTTP server, `/healthz`, and graceful shutdown |

Later packages add their own directories under `internal/`, as the [Implementation Plan](Implementation-Plan.md) assigns them. The `app.App` type owns every dependency, and the module has no package level mutable state. A later package mounts its handlers with `App.Handle` before `App.Run` starts, for example CTL-02 for the `/v1` API.

## Running the binary

We start the controller with a configuration file, flags, or both.

```bash
go run ./controller/cmd/fps-controller -config controller.yaml
go run ./controller/cmd/fps-controller -listen 127.0.0.1:9000 -log-level debug
go run ./controller/cmd/fps-controller -version
```

| Flag | Default | Meaning |
|---|---|---|
| `-config file` | none | YAML configuration file |
| `-listen address` | `localhost:8443` | TCP address of the HTTP server, as host:port |
| `-log-level level` | `info` | `debug`, `info`, `warn`, or `error` |
| `-version` | false | Print the module version and exit |

Settings come from three layers, and a later layer wins. The layers are the built in defaults, the configuration file, and the flags that appear on the command line. The process exits with 0 after a clean shutdown, with 1 when it fails at run time, for example because the port is taken, and with 2 for invalid flags or configuration.

## Configuration file

The file is one YAML mapping. Every key is optional, and an unknown key is an error, so a misspelled key never passes silently. Durations use Go syntax such as `500ms` or `1m30s`.

```yaml
listen: localhost:8443        # host:port of the HTTP server
log_level: info               # debug, info, warn, or error
shutdown_timeout: 15s         # how long in flight requests may run after a stop signal
read_header_timeout: 10s      # how long a client may take to send request headers
```

The default listen address matches the default server in [`contracts/openapi/fps-api-v1.yaml`](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/openapi/fps-api-v1.yaml). The server speaks plain HTTP until a later package adds TLS.

## Logging

The controller writes one JSON object per line to standard error through `log/slog`. Each record carries `time`, `level`, and `msg`, and the lifecycle records are `controller starting`, `controller listening` with `addr`, `controller shutting down` with `timeout`, and `controller stopped`. Tests find the bound port by reading the `addr` of the listening record, which lets them listen on port 0.

## Health and shutdown

`GET /healthz` returns 200 with `{"status":"ok"}` and needs no token, because it sits outside the `/v1` API. Other methods on that path return 405.

On SIGINT or SIGTERM the server closes its listener at once and waits up to `shutdown_timeout` for in flight requests. Request contexts stay alive during that wait. Requests that outlive the timeout lose their connections, and the process exits with 1.

## Checks

The controller has its own make targets and CI workflow.

| Command | Runs |
|---|---|
| `make lint-ctl` | `gofmt -l`, `go vet ./...`, and `golangci-lint run`, which it skips with a notice when the binary is missing |
| `make test-ctl` | `go test -race ./...` in `controller/` |

The workflow [`.github/workflows/controller.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/controller.yml) runs the same checks, with golangci-lint v2.12.2 through `golangci/golangci-lint-action` pinned by commit. [`controller/.golangci.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/controller/.golangci.yml) enables errcheck, errorlint, gosec, govet, ineffassign, revive, staticcheck, and unused, and checks formatting with gofmt and goimports. Its one exclusion allows gosec G204 in test files, because the integration test builds the binary and starts it.

The tests cover three levels. Unit tests check configuration parsing and the logger, component tests run the server on a loopback listener and check `/healthz`, draining, and the shutdown timeout, and an integration test builds `fps-controller`, sends SIGTERM, and expects exit code 0 with the four lifecycle records.

## Dependencies

The module uses the standard library and one dependency, `go.yaml.in/yaml/v3`, which is the maintained successor of `gopkg.in/yaml.v3` under the MIT and Apache-2.0 licenses.

## REST layer

CTL-02 added the package `internal/api/`, which serves the `/v1` API of [`contracts/openapi/fps-api-v1.yaml`](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/openapi/fps-api-v1.yaml). Every operation of the contract answers today, and operations that later packages own answer 501 with a problem.

| Path | Purpose |
|---|---|
| `internal/api/api.gen.go` | Generated types, the `ServerInterface`, and its parameter binding wrapper, from oapi-codegen v2.6.0 with the standard library server |
| `internal/api/routes.gen.go` | Generated table of every operation with its method, path, and the scope from `x-required-scope` |
| `internal/api/server.go` | `api.New` and `Server.Mount`, which route the operations, the colon actions, and the 404 and 405 catch all |
| `internal/api/auth.go` | The `read`, `run`, and `admin` scopes, the `Authenticator` interface, and `StaticTokens` |
| `internal/api/problem.go` | RFC 9457 problem responses |
| `internal/api/unimplemented.go` | `Unimplemented`, which answers every operation with a 501 problem |
| `internal/api/contract/` | Loads the contract as a document that kin-openapi reads |
| `internal/api/gen/` | The generator command and the test that the generated files are current |

### Generated code

We generate the code with the oapi-codegen library, pinned in `controller/go.mod`, so the generator needs no binary on the path.

```bash
cd controller/internal/api && go generate .
```

`TestGeneratedCodeIsCurrent` in `internal/api/gen/` regenerates both files in memory and fails when they differ from the committed ones, so `make test-ctl` and the ctl workflow catch a contract change that nobody regenerated. A second test keeps the version in the generated header equal to the version in `go.mod`.

The contract is OpenAPI 3.1, and kin-openapi reads the 3.0 schema dialect. `contract.Load` therefore reads a view of the contract with two rewrites that keep the set of valid documents the same. A reference to a JSON Schema file under `contracts/schemas/` becomes an open object that names the file in `x-fps-external-schema`, because those files use 2020-12 keywords. An enum of strings without a type gains `type: string`, so the generator emits typed constants such as `RunStateRunning`. The bodies of `createConfig` and `getConfig` and the manifest of `getModule` are therefore plain maps in Go, and the packages that own them validate them against their JSON Schema files.

### Routing

A later package builds the server and mounts it on the app before `App.Run`.

```go
srv := api.New(impl, tokens, logger) // impl embeds api.Unimplemented
srv.Mount(a)                         // a is the *app.App
```

The contract names actions with a colon, as in `POST /configs/{id}:validate`. A `net/http` wildcard must fill a whole segment, so `Mount` registers `POST /v1/configs/{id}`, picks the operation by the suffix of the segment, and strips the suffix from the path value. A colon always marks an action, so a plain parameter never matches a segment that contains one. A path that no operation knows answers 404, and a known path under another method answers 405 with an `Allow` header.

### Tokens and scopes

Every operation needs a bearer token whose scope covers the operation's `x-required-scope`. The scopes nest, so a `run` token passes every `read` operation and an `admin` token passes all of them. The check runs before parameter binding, so an anonymous caller learns nothing from parameter errors.

| Case | Status | `WWW-Authenticate` |
|---|---|---|
| No bearer token | 401 | `Bearer realm="fps"` |
| Unknown token | 401 | `Bearer realm="fps", error="invalid_token"` |
| Scope too low | 403 | `Bearer realm="fps", error="insufficient_scope", scope="run"` |

`StaticTokens` keeps only SHA-256 digests of the tokens. Handlers read the caller's scope with `api.ScopeFromContext`. The controller configuration has no token setting yet, so the binary does not mount the API yet.

### Problems

Every error is an `application/problem+json` body with type `about:blank`, the status text as title, the status, a detail, and the request path as instance. Parameter binding errors, such as a missing `view` on `getRunMetrics`, answer 400.

### Contract test

`TestContractEveryOperation` walks the contract and calls each operation with a token of the required scope, an admin token, no token, an unknown token, and a token one scope too low. It validates every request and every response against the contract with `openapi3filter` from kin-openapi, and it checks that each 501 names the operation it reached, which proves the colon routing. `TestContractParameterErrors` and `TestUnroutedRequests` cover the 400, 404, and 405 problems.

### Dependencies of the REST layer

| Module | License | Used by |
|---|---|---|
| `github.com/oapi-codegen/runtime` v1.3.1 | Apache-2.0 | The generated parameter binding, so the binary links it |
| `github.com/oapi-codegen/oapi-codegen/v2` v2.6.0 | Apache-2.0 | The generator and its freshness test only |
| `github.com/getkin/kin-openapi` v0.133.0 | MIT | The contract loader and the contract tests only |

We stay on these versions because later releases of oapi-codegen and kin-openapi need Go 1.25, and the module builds with Go 1.24.
