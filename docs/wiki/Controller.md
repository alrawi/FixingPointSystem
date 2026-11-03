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
| `internal/store/` | Stores and validates test configurations and serves the /v1/configs operations (CTL-03) |
| `internal/registry/` | The local module registry, manifest checks, tree digests, and the /v1/modules operations (CTL-04) |
| `internal/agents/` | The agent manager with Register and Session, the inventory, and /v1/agents (CTL-06) |
| `internal/fakeagent/` | Fake agents for controller tests (CTL-06) |
| `internal/runs/` | The run state machine, the orchestrator, and the run operations of /v1/runs (CTL-07) |
| `internal/metrics/` | Snapshot aggregation, the metrics views and stream, and Prometheus exposition (CTL-08) |
| `internal/wiring/` | Builds the controller from its packages, the registry and agent adapters, and the combined REST server (CTL-17) |

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
## Configuration store

The package `internal/store` keeps test configurations and validates them. CTL-03 added it. It serves three operations of the REST contract through the type `store.Handler`, which embeds `api.Unimplemented`, so every other operation still answers 501.

| Operation | Scope | Result |
|---|---|---|
| `POST /v1/configs` | run | 201 with a `ConfigRef` and a `Location` header, or 422 with a problem whose `errors` name each failing field |
| `GET /v1/configs/{id}` | read | 200 with the stored configuration as canonical JSON, or 404 |
| `POST /v1/configs/{id}:validate` | run | 200 with a `ValidationReport`, or 404 |

### Storage

The interface `store.Repository` has the methods `Create` and `Get` and the sentinel errors `ErrConfigNotFound` and `ErrConfigExists`. `store.Open` implements it on a SQLite file with the pure Go driver `modernc.org/sqlite`, so the controller still builds without cgo. The store keeps one connection, turns on WAL for files, and migrates the table layout with `PRAGMA user_version`. The path `:memory:` opens a private database for tests. Each record holds the id, `metadata.name`, the body, and the creation time in UTC. The handler takes its clock and its id generator from `store.Options`, and the default ids are `cfg_` with 24 random hex digits.

### Bodies

A create request sends JSON or YAML, chosen by its `Content-Type`. The handler accepts `application/json`, any `+json` type, `application/yaml`, any `+yaml` type, and the legacy YAML names, and it answers 415 for anything else. Bodies are limited to 1 MiB, and larger bodies get 413. The decoder turns either syntax into canonical JSON with sorted keys and the original digits of every number. YAML must hold exactly one document with string keys and no duplicate keys, timestamps stay strings, and a budget of 100000 nodes stops alias bombs. A malformed body gets 400.

### Validation

`POST /v1/configs` checks the body against `contracts/schemas/test-config.schema.json` with `santhosh-tekuri/jsonschema`. The package embeds a copy of the schema in `internal/store/schema/`, which `go generate ./internal/store/` refreshes, and a test fails when the copy differs from the contract. Each leaf of the schema error becomes one finding with a JSON Pointer. A missing or an unknown property points at the property itself, for example `/metadata/name` or `/speed`, so the problem names the field and not its parent object.

The validate operation runs five checks on the stored document and returns every finding, sorted by path.

1. **Schema.** The same check as on create. When it fails, the report stops here.
2. **Cross references.** Duplicate port, pool, and phase names, pools on unknown ports, mix pools that do not exist or have the wrong role, an `app_sim` test without a client pool and a server pool, and a `client_sim` test without external servers.
3. **Scenarios.** Each mix entry must resolve through the interface `store.ModuleCatalog`, which returns `[]compiler.Scenario`. A `<module>/<scenario>` reference must exist, and a bare name must be unique.
4. **Capacity.** Every agent that a port location names must appear in the interface `store.AgentInventory`, which returns the available agents as `[]compiler.Agent`, and an agent that lists interfaces must have the port interface. The report gives `agents_required` and `agents_available`.
5. **Dry run.** When both interfaces are present and no error came up, the validator compiles the test with `compiler.Compile` and turns a compiler error into a finding at the path that the error names.

A nil catalog or inventory skips its check and adds an info finding, so the operation works before CTL-04 and CTL-06 land. A failing catalog or inventory answers 500. Server errors are logged with `log/slog` and never reach the client.

### Tests of the store

Every example under `contracts/examples/tests/` stores as YAML and as JSON, reads back unchanged, and validates without findings against fakes of the catalog and the inventory. Each fixture in `internal/store/testdata/invalid/` breaks the schema and names the expected pointer on its first line, as in `# want: /network/ports/0/location`, and the create test expects a 422 problem with an error at that pointer. Each fixture in `internal/store/testdata/unresolved/` passes the schema and must fail validation at its pointer. Coverage of the package is 93.5%.

### Dependencies of the store

| Module | License | Use |
|---|---|---|
| `modernc.org/sqlite` v1.46.1 | BSD-3-Clause | The SQLite driver without cgo, with `modernc.org/libc`, `mathutil`, and `memory` (BSD-3-Clause) |
| `github.com/santhosh-tekuri/jsonschema/v6` v6.0.2 | Apache-2.0 | JSON Schema 2020-12 validation |
| `golang.org/x/text` v0.21.0 | BSD-3-Clause | The English message printer of the schema errors, now a direct requirement |

We stay on `modernc.org/sqlite` v1.46.1 because v1.47.0 and later need Go 1.25. The go directive now reads `go 1.24.0`, which the new modules require, and `golang.org/x/tools`, `mod`, `sync`, and `sys` moved up to the versions that the driver needs.

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

## Run orchestrator

The package `internal/runs` drives test runs. It moves each run through the run state machine of the specification, reserves the agents of the run, compiles one plan per agent, sends the prepare, start, and stop commands, and contains agent faults. It serves `POST /v1/runs`, `GET /v1/runs/{id}`, and `POST /v1/runs/{id}:stop` as the type `runs.API`, which embeds `api.Unimplemented`, so every other run operation still answers 501.

### State machine

A run moves from pending to preparing, running, and stopping, then ends as completed, failed, or aborted. The table lists every allowed transition, and `runs.Transition` rejects every other pair with `ErrInvalidTransition`.

| From | To |
|---|---|
| `pending` | `preparing`, `failed`, `aborted` |
| `preparing` | `running`, `failed`, `aborted` |
| `running` | `stopping`, `failed`, `aborted` |
| `stopping` | `completed`, `failed`, `aborted` |
| `completed`, `failed`, `aborted` | none |

Only `stopping` may complete, because the orchestrator collects the final counters and evaluates the criteria while it stops.

### Lifecycle

`Create` loads the configuration from the store, parses it, picks the agents, reserves them for the run id, and compiles the plans before it answers. A request without agents takes every agent that the operator reserved, and the operator reservation returns to the operator when the run ends. Any error in these steps restores the reservations and creates no run, so `POST /v1/runs` answers 422 for an unknown or invalid configuration or agent, 409 for an agent that is offline, busy, or held by another run and for a request without reserved agents, and 202 with a `Location` header otherwise. The run then proceeds in the background. It sends the plans, starts them, and waits while the agents run. It stops when every agent finished or failed, or when a stop request arrives.

### Stops

A graceful stop sends `STOP_MODE_RAMP_DOWN` with `Config.DrainTimeout` to every active agent and still evaluates the criteria. A forced stop cancels the pending commands, sends `STOP_MODE_ABORT` to every agent that holds the plan, and ends the run as aborted without a verdict. A forced stop during a ramp down also aborts. A stop before the run starts aborts it, and a stop of an ended run answers 409. `Close` aborts every open run with the reason controller shutdown.

### Fault containment

`Orchestrator.Observe` takes the events of the agent manager, and the controller wires it as `agents.Config.Observer`. An agent that reports `FAILED`, loses its plan, or goes offline after a crash or a missed heartbeat fails within its run. The run turns degraded, names the agent in `FailedAgents`, and keeps the other agents running. A failure while preparing or starting fails the run, and a run whose agents all fail ends as failed.

### Criteria

When a run stops, the orchestrator sums the newest unlabeled counters of every agent and derives `sessions.failed_ratio`. The sessions that a failed agent attempted without success or failure count as failed, because the crash ended them. `Config.Evaluator` decides the verdict, and a failing verdict ends the run as failed and a passing one as completed. The default `EvaluateTotals` checks every criterion whose metric appears in the totals and skips the others, which leaves the full catalog to CTL-09.

### Tests of the orchestrator

`state_test.go` checks all 49 pairs of states against the allowed table. The orchestrator tests use the real agent manager, fake agents, the real compiler, an in memory store, and a fake clock. `TestFailingAgentEndsRunByCriteria` lets one of two agents fail or crash under way and checks that a failure ratio of 0.5% completes the degraded run and a ratio of 30% fails it against the criterion of 10%. Other tests cover finishing agents, missed heartbeats, a graceful stop, forced stops while preparing, running, and ramping down, a prepare error, operator reservations, create errors, and `Close`. The REST tests validate every exchange against the OpenAPI document.
## Capabilities endpoint

`controller/internal/api/capabilities.go` serves `GET /v1/capabilities` with the read scope. The type `CapabilitiesAPI` embeds `api.Unimplemented`, so it answers every other operation with the 501 problem, and the binary composes it with the other handlers. The package imports neither `internal/registry` nor `internal/agents`, because `internal/agents` already imports `internal/api`. It reads two narrow interfaces that it defines itself, and the binary adapts the registry and the agent manager to them when it wires the controller, which follow up #102 tracks.

| Interface | Method | Adapter in the binary |
|---|---|---|
| `ModuleLister` | `ModuleSummaries(ctx)` returns `[]api.ModuleSummary` | `Registry.List` with the summary of each entry |
| `AgentInventory` | `AgentCapabilities(ctx)` returns `[]api.AgentCapability` | `Manager.List` with `agents.ToAPI` and every backend of the latest registration |

An `AgentCapability` carries the `Agent` schema and every backend that the agent supports, because the schema names only one backend. The response holds four parts.

| Field | Value |
|---|---|
| `engine_api` | `api.EngineAPIVersion`, which is 0.2.0 for contracts v0.2 |
| `api_version` | `api.APIVersion`, which equals `info.version` of the OpenAPI document |
| `backends` | The union of the backends of all agents, in the order kernel, af_xdp, dpdk |
| `agents` and `modules` | The lists of the two sources in their own order |

Every array is present and empty when its source is empty or nil. A failing source gives a 500 problem whose detail names the source. `TestContractCapabilities` serves two fake agents and two modules and validates the response against the OpenAPI document with the contract loader. `TestCapabilitiesVersions` compares the two version constants with the OpenAPI document and the heading of `contracts/README.md`, so a contract release that forgets them fails the build.

## Metrics aggregation

The package `internal/metrics` merges the counter snapshots of the agents of a run into run metrics and serves them. It implements `GET /v1/runs/{id}/metrics` and `GET /v1/runs/{id}/metrics:stream` as the type `metrics.API`, which embeds `api.Unimplemented`, and `GET /metrics` in the Prometheus text format as the handler `metrics.Prometheus`. The binary creates one `Aggregator` with `metrics.New`, calls `Aggregator.Observe` from `agents.Config.Observer` next to `Orchestrator.Observe`, mounts `metrics.NewAPI(agg, orchestrator)` with the other operation sets, and mounts `metrics.NewPrometheus(agg)` at `/metrics`.

### Merge rules

The aggregator finds the run of a snapshot from its plan id, which the compiler writes as `<run id>/<agent id>`. It keeps the newest value of every series of every agent and worker by sequence number, so a repeated snapshot after a reconnection changes nothing, and a snapshot that leaves a series out keeps its earlier cumulative value. An agent that sends per worker snapshots contributes only those, because `per_worker` replaces the aggregate snapshot. The merge applies one rule per kind.

| Kind | Rule |
|---|---|
| Counter | Sum |
| Gauge in a count or rate unit | Sum |
| Gauge in the units `ratio`, `percent`, or `percentage_points` | Mean |
| Gauge with a catalog `formula` | Recomputed from the merged counters of its group, never merged |
| Histogram | Bucket by bucket sum of the HDR counts |

The catalog comes from an embedded copy of `contracts/metrics/catalog.yaml`, and a test keeps the copy equal to the contract. The aggregator drops entries whose metric, kind, or labels the catalog does not admit, histograms whose encoding is malformed, and series beyond `Config.MaxSeries` for one run, and `Aggregator.Dropped` counts them.

### HDR histograms

We decode the HdrHistogram V2 encoding, compressed or not, with our own decoder in `hdr.go`. The Go library `hdrhistogram-go` panics on truncated input, allocates the dense counts array that a header asks for, and cannot bound the inflated size, while our decoder keeps the counts sparse and bounds every length. Every histogram lands in the layout of `engine/fps-stats`, which has a lowest value of 1 and 3 significant digits, so a merge of agent histograms is exact. A histogram of another layout moves into that layout by the value of each bucket, as the add of HdrHistogram does. The statistics `p50`, `p90`, `p99`, `p999`, `min`, `max`, `mean`, and `count` follow the Rust crate `hdrhistogram`.

### Views and windows

The aggregator records the six views once per second of controller time and keeps up to `Config.MaxPoints` points per run. Each view groups the merged series by one key and adds the key `*` for the whole run. A histogram appears in a sample as one value per statistic, such as `sessions.response_time.p99`.

| View | Metrics | Key |
|---|---|---|
| `application` | Scenario and flow action metrics, and the `mix.` gauges | `application` |
| `scenario` | Scenario and flow action metrics | `application/scenario` |
| `port` | Port and host metrics | `port` |
| `tcp` | Transport metrics that start with `tcp.` | `port` |
| `tls` | Transport metrics that start with `tls.` | `port` |
| `realism` | The `realism.` gauges | `application` |

`GET /v1/runs/{id}/metrics` returns the points between `from` and `to`, both inclusive, and `step_s` keeps the newest point of every step. A run that the orchestrator knows but that sent no snapshot yet answers with an empty sample list. The responses use the type `metrics.Sample` with float64 values in place of the generated float32, because a float32 holds integers exactly only up to 2^24, and the tests validate every response against the OpenAPI document.

### Stream

`GET /v1/runs/{id}/metrics:stream` sends Server Sent Events once per `Config.StreamInterval`, which defaults to one second. Each tick sends one event per current sample of every requested view, or of all six views when the request names none, with the sample as JSON in `data` and the tick number in `id`. When the run ends, the stream sends the final samples and an event named `end` whose data holds the run state, and closes.

### Prometheus

A catalog name maps to `fps_` followed by the name with dots replaced by underscores, and counters add `_total`, as `contracts/README.md` describes. Every series carries the label `run_id` and its catalog labels. A histogram becomes a summary with the quantiles 0.5, 0.9, 0.99, and 0.999, a `_count`, and a `_sum` that equals the HDR mean times the count. The handler writes the text format 0.0.4 with the standard library, so the controller module gained no dependency, and `testdata/prometheus.txt` holds its golden output. The handler serves every run that the aggregator holds, up to `Config.MaxRuns`, and `Aggregator.Forget` drops a run.

### Tests of the aggregator

`TestCountersEqualSumOfFakeAgentSnapshots` sends six rounds of cumulative snapshots from three fake agents through the real agent manager, one agent with two workers, adds a stale replay and an empty final snapshot, and checks that every merged counter equals the sum of the newest snapshots. `TestMergeMatchesReference` and `TestHistogramsOfFakeAgentsMatchReference` compare the merged histograms bucket by bucket and statistic by statistic with `testdata/reference_merge.json`, which `Snapshot::merge_all` of `engine/fps-stats` wrote. The generator lives in `testdata/refgen` and reruns with the command in its `Cargo.toml`. Further tests cover derived ratios, gauge means, views, windows and steps, limits, malformed and oversized encodings, a fuzz target of the decoder, the stream, and the Prometheus golden file.
## Binary wiring

CTL-17 wired every package into `fps-controller`, so the binary serves the whole `/v1` REST API and the agent gRPC service. The package `internal/wiring` builds the components, adapts them to one another, and combines the REST handlers, and `cmd/fps-controller` runs the REST server and the gRPC service side by side.

### Settings of the binary

The configuration file gained six keys, and the binary gained two flags. Every earlier key and flag keeps its meaning.

```yaml
data_dir: /var/lib/fps          # module registry in data_dir/modules, store in data_dir/configs.db
store_path: /var/lib/fps/db/configs.db   # optional, overrides the store file, or ":memory:"
grpc_listen: localhost:8444     # host:port of the agent gRPC service, plain TCP until CTL-15
agent_auto_enroll: false        # accept agents that no operator enrolled with POST /v1/agents
tokens:
  - { name: ci, scope: run, token_file: /etc/fps/ci.token }
  - { name: ops, scope: admin, token: change-me-0123456789 }
```

| Key or flag | Default | Meaning |
|---|---|---|
| `data_dir`, `-data-dir dir` | none | Directory of the module registry and the configuration store. Without it the store lives in memory and the registry in a temporary directory, so every configuration and module ends with the process, and the controller logs a warning |
| `store_path` | `data_dir/configs.db`, or `:memory:` without `data_dir` | SQLite file of the configuration store. The controller creates its directory |
| `grpc_listen`, `-grpc-listen address` | `localhost:8444` | TCP address of `ControllerService`, which agents dial |
| `agent_auto_enroll` | false | Lets any agent identity register without an enrollment |
| `tokens` | none | Bearer tokens of the REST API, each with a `name`, a `scope` of `read`, `run`, or `admin`, and exactly one of `token` and `token_file` |

A token holds at least 16 bytes. A `token_file` holds the token alone, and the controller trims the white space around it. Names must be unique, and two entries must not hold the same token. A token that breaks a rule, or a token file that the controller cannot read, ends the process with exit code 2. Without tokens every `/v1` operation answers 401, and the controller logs a warning at startup.

The gRPC service speaks plain TCP and trusts the `agent_id` of each request, because mutual TLS arrives with CTL-15. The controller logs a warning that agent identities are not verified. We therefore bind `grpc_listen` to a loopback or a lab network until then.

### Components

`wiring.New` builds the components in one order and closes what it opened when a step fails.

1. The token check, from `tokens`, as `api.StaticTokens`.
2. The configuration store, with `store.Open` on the resolved store path.
3. The module registry, with `registry.Open` on `data_dir/modules` or a temporary directory.
4. The agent manager, with `agents.ClaimedIdentity`, `agent_auto_enroll`, and an observer that fans each event out to every function registered with `Controller.Observe`.
5. The run orchestrator over the store, the agent manager, and the registry catalog. Its `Observe` method is the first observer, which `runs` needs to contain agent faults.
6. The REST handlers and the combined server.

`Controller.Run` serves `ControllerService` and calls `Manager.Run` for the heartbeat sweeps. When its context ends, it aborts the open runs, ends every agent session with `UNAVAILABLE`, and stops the gRPC server within `shutdown_timeout`. The binary cancels both servers when either fails, so a dead gRPC listener never leaves a REST server without agents. The new lifecycle records are `agent service listening` with `addr`, `agent service shutting down`, and `agent service stopped`, and tests read the gRPC port from the `addr` of the listening record.

### Adapters

Two adapter types connect the registry and the agent manager to the interfaces that the other packages define.

| Adapter | Implements | Source |
|---|---|---|
| `wiring.Catalog` | `store.ModuleCatalog` and `runs.Catalog` with `Scenarios` | The scenario files of the latest installed version of each module, with the module name, version, and tree digest. Configurations name a module without a version, so older versions stay out of the catalog |
| `wiring.Catalog` | `api.ModuleLister` with `ModuleSummaries` | Every installed version from `Registry.List` |
| `wiring.Inventory` | `store.AgentInventory` with `AvailableAgents` | Agents that are online or reserved, with cores, the first known backend, and the port names, as the orchestrator compiles them |
| `wiring.Inventory` | `api.AgentInventory` with `AgentCapabilities` | Every agent through `agents.ToAPI`, with every backend of its latest registration |

### Combined server

`wiring.Server` is the single `api.ServerInterface` that `api.New` receives. It embeds the handler of every package, and each of those embeds `api.Unimplemented`. Go promotes the method of the package that implements an operation, because it sits one level above the 501 methods that the other handlers inherit. The operations that no package implements sit in an embedded `pending` type at the same level, so no selector is ambiguous. A package that implements a pending operation deletes it from `pending` and embeds its handler, and the compiler reports an ambiguous selector while both remain.

| Operations | Handler |
|---|---|
| `/v1/agents` and its actions | `agents.API` |
| `GET /v1/capabilities` | `api.CapabilitiesAPI` |
| `/v1/configs` and `:validate` | `store.Handler` |
| `/v1/modules` | `registry.API` |
| `POST /v1/runs`, `GET /v1/runs/{id}`, `POST /v1/runs/{id}:stop` | `runs.API` |
| `GET /v1/runs/{id}/metrics` and `:stream` | The `MetricsHandler` of `Controller.SetMetrics`, or 501 |
| `listRuns`, captures, and `getRunReport` | `pending`, which answers 501 |

### Hook for run metrics

The metrics package of CTL-08 joins the binary in three calls in `serve` of `cmd/fps-controller/main.go`, where a comment marks the place before `Mount`. `ctl.SetMetrics` takes a value with the methods `GetRunMetrics` and `StreamRunMetrics` of `api.ServerInterface`, `ctl.Observe` adds its agent event observer next to the orchestrator, and `a.Handle("GET /metrics", ...)` mounts the Prometheus endpoint outside `/v1`. The exported fields of `wiring.Controller` give the package the store, the registry, the agent manager, and the orchestrator.

### Tests of the wiring

`TestServerRoutesEveryOperation` walks every operation of the OpenAPI contract with an admin token and checks that exactly the seven pending operations answer 501, each with a detail that names it, so every other operation reaches the package that implements it. Other tests in `internal/wiring` cover the token scopes, the metrics hook, the catalog with two versions of `http1`, the inventory filters, and a gRPC round trip with a fake agent and the observer.

`TestEndToEnd` in `cmd/fps-controller` builds the binary and starts it with a temporary configuration that holds an admin token, a run token in a token file, a read token, and a data directory. It runs eleven steps.

1. It checks that a request without a token answers 401 and that a run token cannot enroll an agent.
2. It enrolls the agent `local` with the admin token.
3. A fake agent from `internal/fakeagent` registers over gRPC on the logged port with the port `lo` and automatic heartbeats.
4. It installs the example modules `http1` and `dns` of `contracts/examples/modules/` as gzip compressed tar archives.
5. It lists the modules and reads the capabilities with one agent, two modules, and the kernel backend.
6. It stores `contracts/examples/tests/http_smoke.yaml` as YAML.
7. It validates the configuration, which passes the module and capacity checks without findings.
8. It starts a run on `local`, waits until the run is running, and checks the fake agent state.
9. It stops the run and expects it completed with a passing verdict after the prepare, start, and stop commands.
10. It sends SIGTERM and expects exit code 0 with the stopped records of both servers.
11. It starts the binary again on the same data directory and finds the configuration and both modules.

## Metrics in the binary

Bug #119 closed the hook that the Binary wiring section describes. The function `mountMetrics` in `cmd/fps-controller/main.go` runs in `serve` before `ctl.Mount(a)` and makes four calls.

1. `metrics.New(metrics.Config{Logger: logger})` creates one aggregator with the default catalog, the system clock, and the default bounds.
2. `ctl.Observe(agg.Observe)` adds the aggregator to the agent manager observers after the run orchestrator, so every counter snapshot of an agent reaches both.
3. `ctl.SetMetrics(metrics.NewAPI(agg, ctl.Runs))` routes `GET /v1/runs/{id}/metrics` and `GET /v1/runs/{id}/metrics:stream` to the aggregator, with the run orchestrator as the `RunSource`. A run that the orchestrator knows answers 200 before its first snapshot, and the stream ends when the run ends.
4. `a.Handle("GET /metrics", metrics.NewPrometheus(agg))` serves the Prometheus exposition outside `/v1`.

`GET /metrics` needs no token, because Prometheus scrapers usually send none. The exposition holds run identifiers, application names, scenario names, and port names, so an operator who considers them sensitive keeps the listen address on a trusted network. The exported field `wiring.Controller.Runs` already gives the orchestrator, so the wiring package needed no new accessor.

`TestEndToEnd` now runs one more step while the run is running. The fake agent sends a snapshot with `sessions.attempted` equal to 7 for the application `http1` and the scenario `web_get`. The test polls until `GET /v1/runs/{id}/metrics?view=application` returns the key `http1` with that count and `GET /metrics` contains the line `fps_sessions_attempted_total{run_id="<run id>",application="http1",scenario="web_get"} 7`.
