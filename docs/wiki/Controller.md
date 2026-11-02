# Controller

The controller is the Go process that stores test configurations, compiles them into agent plans, drives runs, and serves the REST API. This page describes what exists today and grows with each CTL package. The module is `github.com/alrawi/FixingPointSystem/controller` in [`controller/`](https://github.com/alrawi/FixingPointSystem/tree/main/controller), and `go.work` at the repository root lists it.

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
