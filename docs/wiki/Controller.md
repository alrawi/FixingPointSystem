# Controller

The controller is the Go process that stores test configurations, compiles them into agent plans, drives runs, and serves the REST API. This page describes what exists today and grows with each CTL package. The module is `github.com/alrawi/FixingPointSystem/controller` in [`controller/`](https://github.com/alrawi/FixingPointSystem/tree/main/controller), and it builds on its own with `GOWORK=off`.

## Layout

The module has two packages after CTL-01.

| Path | Purpose |
|---|---|
| `cmd/fps-controller/` | The binary. It parses flags, loads the configuration, handles SIGINT and SIGTERM, and maps failures to exit codes |
| `internal/app/` | Configuration loading and validation, the JSON logger, the HTTP server, `/healthz`, and graceful shutdown |
| `internal/api/` | The REST layer generated from the OpenAPI contract, with problem responses and token scopes (CTL-02) |
| `internal/compiler/` | Compiles a test configuration and an agent inventory into per agent plans (CTL-05) |

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
## Test compiler

The package `internal/compiler` turns one test configuration and one agent inventory into one plan per agent. CTL-05 added it. `compiler.Compile` is a pure function. It reads no clock, draws no random numbers, and does no I/O, so the same request always yields the same plans, ordered by agent id. The plan types mirror the [canonical JSON mapping](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md#canonical-json-mapping) of `fps.agent.v1.Plan` field for field, and `Plan.MarshalCanonical` writes the layout of `contracts/examples/plans/`. We will switch to generated protobuf types once the controller generates code from `contracts/proto/`.

A request carries five inputs.

| Field | Meaning |
|---|---|
| `RunID` | The run id. Each plan id is `<RunID>/<agent id>` |
| `Config` | The test configuration, decoded with `compiler.ParseConfig`, which rejects unknown keys |
| `Agents` | The agent id, the core count, the backend, and optionally the interface names of each agent |
| `Scenarios` | The installed scenarios with their module name, version, digest, YAML source, and an optional bytes per session estimate |
| `Seed`, `EngineAPI` | The seed to use when the configuration sets no `timeline.seed`, and the engine contract version, which defaults to 0.2.0 |

The compiler applies six rules.

1. **Mix.** Each entry resolves to `<module>/<scenario>`, and a bare name must be unique in the catalog. Weights normalize to sum 1 and act as session shares, also for the `bytes` basis, until CTL-12 adds byte based correction. The entry id is the scenario name, with `_2`, `_3`, and so on for repeats.
2. **Workers.** Agents sort by id, and each agent contributes one worker per core, where worker i pins to core i. Every worker carries an equal share, so the share of an agent follows its core count.
3. **Host pools.** Each pool divides over all workers in that order. Every worker gets the pool count divided by the number of workers, and the first workers get one more host each until the remainder is used up. An agent therefore owns a contiguous slice, and the compiler fails with `ErrPoolTooSmall` when an agent would get no host of a pool. A pool without `mac_base` gets the locally administered base `02:xx:yy:00:00:00`, where xx and yy hold the pool index, which keeps MAC addresses unique across agents.
4. **Rates.** `sessions_per_second` divides equally over the workers. `throughput_bps` first becomes a session rate through the weighted bytes per session estimate of the mix and fails with `ErrNoByteEstimate` when a scenario has none. `concurrent_sessions` and `simulated_users` round to whole sessions, which divide over the mix by largest remainder and over the workers with a rotating start, so the leftover sessions spread across workers.
5. **Phases.** A hold phase runs at the steady target. A linear or step phase ramps up from the current fraction to 1, or ramps down to 0 when the load already stands at 1. A step phase takes one step per started second.
6. **Checks.** The compiler checks the rules that it relies on, such as unknown pools and ports, pool roles, pool counts against the usable addresses of their prefixes, and the inventory. Full schema validation belongs to the configuration store.

Golden tests compile the contract examples `http_smoke.yaml` on one agent and `enterprise_mix_10g.yaml` on one agent with eight cores, two agents with four and two cores, and two agents with three and two cores whose pools do not divide evenly. The goldens live in `controller/internal/compiler/testdata/<case>/<agent>.json`, and `python3 tools/contracts/check_plans.py --modules contracts/examples/modules controller/internal/compiler/testdata/*/*.json` accepts all of them. A determinism test compiles every case a second time with the inventory and the catalog reversed and expects identical bytes. The tests use the Go golden file helper through a `replace` line in `controller/go.mod`.

## Module registry

The package `internal/registry` keeps the installed modules in one local directory and serves the four `/v1/modules` operations. CTL-04 added it. The registry is the only part of the controller that reads module packages, and it never runs module code.

| Path under the registry root | Content |
|---|---|
| `index.json` | Format version 1 and one entry per installed module, with name, version, type, license, summary, scenario names, and tree digest |
| `modules/<name>/<version>/` | The module tree with `manifest.yaml` at its root |
| `staging/` | Trees that an install is still checking |

### Install

`Registry.Install` takes a tar archive, optionally gzip compressed, with `manifest.yaml` at its root, and `Registry.InstallDir` takes a directory such as `modules/http1`. Both run the same checks in the same order.

1. **Archive.** Only directories and regular files are allowed. Every path is relative, uses only the characters of the `relpath` pattern of the manifest schema, and has no `.` or `..` segment. The registry refuses links, device entries, and duplicate entries, and it bounds the unpacked size and the file count, by default 256 MiB and 10,000 files.
2. **Schema.** The manifest validates against `contracts/schemas/module-manifest.schema.json`. The package embeds a copy of the schema, a test fails when the copy differs from the contract, and `go generate ./internal/registry` refreshes it. JSON Schema patterns follow ECMA-262 and the `relpath` pattern uses lookahead, so the validator runs patterns with `regexp2` in ECMAScript mode.
3. **License.** The `license` field is an SPDX expression that must allow distribution with GPL-3.0 code. Every AND operand must be compatible, and at least one operand of each OR. Every license and exception must be on the allow list, so `MIT OR LicenseRef-acme` fails as unknown. Matching ignores case for identifiers but not for the operators, and the deprecated `+` suffix and bare `GPL-3.0` forms are unknown.
4. **Semantic rules.** Function names are unique, a manifest with `functions` and an `extension` lists the `render` hook, and the module name is not one of the names that rule 5 of the template grammar reserves.
5. **Files.** Every scenario, asset path, extension component, and fixture capture that the manifest names exists as a regular file. Each scenario file has a `scenario` field whose name is an identifier and unique within the module. Full scenario validation belongs to the configuration validator.
6. **Digest.** The registry computes the module tree digest of plan rule 1 in `contracts/README.md` and records it in the index. A test checks that the example `http1` and `dns` modules get the digests of `contracts/examples/plans/http_dns_smoke.json`.

The tree then moves into place and the index is rewritten through a temporary file and a rename. A module version is immutable, so installing a name and version that the registry holds fails with `ErrModuleExists`.

### Allowed licenses

| Group | Identifiers |
|---|---|
| Copyleft | GPL-3.0-only, GPL-3.0-or-later, GPL-2.0-or-later, LGPL-3.0-only, LGPL-3.0-or-later, LGPL-2.1-only, LGPL-2.1-or-later, AGPL-3.0-only, AGPL-3.0-or-later, MPL-2.0 |
| Permissive | Apache-2.0, MIT, MIT-0, X11, ISC, 0BSD, Zlib, BSL-1.0, BSD-2-Clause, BSD-3-Clause, Artistic-2.0, Unicode-3.0, Unicode-DFS-2016, CDLA-Permissive-2.0 |
| Content | Unlicense, CC0-1.0, CC-BY-4.0, CC-BY-SA-4.0 |
| Exceptions | LLVM-exception, Classpath-exception-2.0, GCC-exception-3.1 |

Well known incompatible licenses, such as GPL-2.0-only, EPL-2.0, CDDL-1.0, SSPL-1.0, and BUSL-1.1, fail with `ErrLicenseIncompatible`, and every other identifier fails with `ErrLicenseUnknown`.

### Operations

`registry.NewAPI(reg, logger)` returns a type that embeds `api.Unimplemented` and implements the four module operations, so the controller composes it with the types of the other packages.

| Operation | Behavior |
|---|---|
| `listModules` | Summaries sorted by name and semver precedence, with the `type` filter and pages of `page_size` entries, 50 by default. The page token encodes the last module of the page, so a page stays stable when other modules come and go. |
| `installModule` | 201 with the summary and a `Location` header. A manifest or tree problem answers 422 with one finding per problem, whose path is a JSON Pointer into the manifest or `file:` and a path in the tree. A broken archive answers 400, an archive over the limit 413, and an installed version 409. |
| `getModule` | The manifest as JSON, which validates against the manifest schema, or 404 |
| `deleteModule` | 204, or 404 for a module the registry does not hold |

### Lookup for other packages

`registry.Lookup` resolves module references without the HTTP layer. `Manifest(ctx, name, version)` returns the validated manifest with its function catalog, and `Latest(ctx, name)` returns the installed version with the highest precedence, because test configurations name a module without a version. Both wrap `ErrModuleNotFound`. `Registry.List` and `Registry.Entry` return index entries with scenario names and digests, which the compiler request needs.

### Dependencies of the registry

| Module | License | Why the standard library does not suffice |
|---|---|---|
| `github.com/santhosh-tekuri/jsonschema/v6` v6.0.2 | Apache-2.0 | Validates JSON Schema 2020-12, which the manifest contract uses |
| `github.com/dlclark/regexp2` v1.11.5 | MIT | Runs ECMA-262 patterns with lookahead, which `regexp` cannot express |
## Agent manager

The package `internal/agents` is the agent manager of the controller. CTL-06 added it. It serves the controller side of `ControllerService` from [`agent.proto`](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/proto/fps/agent/v1/agent.proto), keeps the agent inventory, and serves the four `/v1/agents` operations. Agents dial the controller, call `Register`, and hold one `Session` stream open, as the [agent control protocol](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md#agent-control-protocol) describes.

```go
mgr, err := agents.New(agents.Config{ControllerID: "ctl-1"}) // PeerCertIdentity by default
agentpb.RegisterControllerServiceServer(grpcServer, mgr)
go mgr.Run(ctx)                                             // heartbeat timeouts
srv := api.New(agents.NewAPI(mgr), tokens, logger)          // /v1/agents
```

### Generated code

The package `internal/agents/agentpb` holds the messages and the gRPC client and server of `agent.proto`. The command in `agentpb/gen` compiles the proto file with protocompile, so no `protoc` binary is needed, and passes the request to `protoc-gen-go` and `protoc-gen-go-grpc`, which `controller/go.mod` pins as tools and which run through `go tool`. The contract carries no `go_package`, so the generator maps the file to the package with an `M` option. `go generate` in `agentpb` writes the files, and `TestGeneratedCodeIsCurrent` fails when they differ from the contract.

### Registration and sessions

`Register` checks four rules in the order of the contract and answers with the gRPC codes that the contract names.

| Check | Code on failure |
|---|---|
| `agent_id` matches the Id pattern of the REST contract | `INVALID_ARGUMENT` |
| `Config.Identity` returns an identity, and it equals `agent_id` | `UNAUTHENTICATED` |
| The identity is enrolled with `POST /v1/agents`, or `Config.AutoEnroll` holds | `PERMISSION_DENIED` |
| The major and minor version of `Capabilities.engine_api` is in `Config.EngineAPIs`, which defaults to 0.1 and 0.2 | `FAILED_PRECONDITION` |

`PeerCertIdentity` takes the identity from the verified client certificate, first the common name and then the first DNS name. `ClaimedIdentity` trusts the request and serves tests and local development only. Each `Register` returns a new 256 bit session token and revokes the previous one. The first message of a session must be `open` with the latest token, and a newer session ends the older one with `ABORTED`. The manager answers every heartbeat with an acknowledgement, routes each command result to its caller by `command_id`, and drops a counter snapshot whose sequence is not newer than the last one of the same plan, so duplicates after a reconnection never reach the run.

### Inventory and reservation

Each agent has one of the four states of the REST contract.

| State | Condition |
|---|---|
| `offline` | No open session |
| `busy` | The agent reports `PREPARING`, `PREPARED`, `RUNNING`, or `STOPPING` |
| `reserved` | A holder reserves the agent |
| `online` | Every other case |

`Reserve(id, holder)` succeeds for an online agent and repeats for the same holder. It fails with `ErrAgentOffline`, `ErrAgentReserved`, or `ErrAgentBusy` otherwise. `Release(id, holder)` ends the reservation of that holder, and an empty holder releases any reservation. The REST operations reserve for the holder `operator` and release with an empty holder, and they answer 404 for an unknown agent, 409 for a conflict, and 422 for an invalid id. `POST /v1/agents` enrolls an identity with labels and answers 201, and enrolled labels override the labels that the agent reports.

### Heartbeat timeouts and commands

`Sweep` reads `Config.Clock` and marks every agent offline whose latest heartbeat is older than the heartbeat timeout. It ends the session with `DEADLINE_EXCEEDED` and keeps the reservation, so the run that holds the agent decides whether to mark itself degraded. `Run` calls `Sweep` once per heartbeat interval, and tests call `Sweep` with a fake clock. `Prepare`, `Start`, and `Stop` send one command and wait for its result. A command other than stop waits for the previous one, a failed command returns a `*CommandError` with the agent's code, and a session that ends first returns `ErrSessionLost`. `Config.Observer` receives an event for each registration, session open, offline transition, state change, and new snapshot. `Shutdown` ends every session with `UNAVAILABLE`.

### Fake agents

The package `internal/fakeagent` gives other controller packages agents that dial a `ControllerService` in tests. `fakeagent.Serve(t, mgr)` serves the manager on an in-memory listener, and `fakeagent.Start` registers an agent and opens its session. A fake agent answers prepare, start, and stop by the agent state machine and sends a heartbeat after each state change. A test drives it with six methods.

| Method | Effect |
|---|---|
| `Heartbeat` | Sends one heartbeat and waits for the acknowledgement |
| `Finish` | Ends a running plan on its own with a final snapshot |
| `Fail` | Moves the agent to `FAILED` with a message |
| `Crash` | Closes the connection without a message |
| `MissHeartbeats` | Stops all heartbeats while the session stays open |
| `Reconnect` | Registers again with the current state and plan |

`Options.OnCommand` lets a test fail or delay a command, and `Options.AutoHeartbeat` sends heartbeats on the wall clock. `TestThreeAgents` registers three fake agents, reserves two, releases one, runs a plan on one, and lets the third miss its heartbeats until `Sweep` marks it offline and it reconnects with its reservation intact. A contract test validates every `/v1/agents` response against the OpenAPI document.

### Dependencies of the agent manager

The module gained three direct dependencies and two tools.

| Module | License | Used by |
|---|---|---|
| `google.golang.org/grpc` v1.79.3 | Apache-2.0 | The gRPC server, the generated code, and the fake agents |
| `google.golang.org/protobuf` v1.36.11 | BSD-3-Clause | The generated messages, and `protoc-gen-go` as a tool |
| `github.com/bufbuild/protocompile` v0.14.1 | Apache-2.0 | The generator and its freshness test only |
| `google.golang.org/grpc/cmd/protoc-gen-go-grpc` v1.5.1 | Apache-2.0 | A tool of the generator only |

The grpc release needs Go 1.24.0, so `go mod tidy` writes `go 1.24.0` in place of `go 1.24`, and it raised `golang.org/x/net`, `x/sys`, `x/text`, `x/sync`, `x/mod`, and `x/tools` as indirect dependencies.
