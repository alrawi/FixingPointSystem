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
| `internal/report/` | Criteria evaluation and the JSON and CSV run reports (CTL-09) |
| `internal/wiring/` | Builds the controller from its packages, the registry and agent adapters, and the combined REST server (CTL-17) |
| `internal/pki/` | The controller CA, enrollment tokens, agent certificates, rotation, revocation, and the mutual TLS configuration (CTL-15) |

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

The package `internal/compiler` turns one test configuration and one agent inventory into one plan per agent. CTL-05 added it. `compiler.Compile` reads no clock and does no I/O. It draws a random number only when neither the configuration nor the request names a seed, so the same request with a seed always yields the same plans, ordered by agent id. The plan types mirror the [canonical JSON mapping](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md#canonical-json-mapping) of `fps.agent.v1.Plan` field for field, and `Plan.MarshalCanonical` writes the layout of `contracts/examples/plans/`. We will switch to generated protobuf types once the controller generates code from `contracts/proto/`.

A request carries five inputs.

| Field | Meaning |
|---|---|
| `RunID` | The run id. Each plan id is `<RunID>/<agent id>` |
| `Config` | The test configuration, decoded with `compiler.ParseConfig`, which rejects unknown keys |
| `Agents` | The agent id, the core count, the backend, and optionally the interface names of each agent |
| `Scenarios` | The installed scenarios with their module name, version, digest, YAML source, an optional bytes per session estimate, and the settings schema of the module manifest as JSON, which stays empty for a module without settings |
| `Seed`, `Rand`, `EngineAPI` | The seed to use when the configuration sets no `timeline.seed`, the source of the random seed when that is zero too, which defaults to `crypto/rand`, and the engine contract version, which defaults to 0.2.0 |

The compiler applies the nine rules below.

1. **Mix.** Each entry resolves to `<module>/<scenario>`, and a bare name must be unique in the catalog. Weights normalize to sum 1 and act as session shares, also for the `bytes` basis, until CTL-12 adds byte based correction. The entry id is the scenario name, with `_2`, `_3`, and so on for repeats.
2. **Workers.** Agents sort by id, and each agent contributes one worker per core, where worker i pins to core i.
3. **Ports.** A location `<agent>/<interface>` pins the port to that one agent, which must be in the inventory and must have the interface, or the compiler fails with `ErrInvalidInventory`. A bare `<interface>` or `*/<interface>` spreads the port across every agent that has the interface, where an agent with an empty interface list counts as having it, and fails with `ErrInvalidInventory` when no agent has it. A plan lists only the ports and the host pool slices of its own agent. When a port runs on fewer than all agents, the mix entries name their default pools explicitly, because an agent that lacks the first pool of a role would otherwise default to another pool, and an agent keeps a mix entry in its plan only when it has a rate for it or emulates the hosts of its server pool.
4. **Host pools.** Each pool divides over the workers of the agents that run its port, in global worker order. Every such worker gets the pool count divided by the number of those workers, and the first workers get one more host each until the remainder is used up. An agent therefore owns a contiguous slice. A pool with fewer hosts than workers is valid and leaves the last workers without hosts, but the compiler fails with `ErrPoolTooSmall` when an agent that runs the port would get no host of the pool. A pool without `mac_base` gets the locally administered base `02:xx:yy:00:00:00`, where xx and yy hold the pool index, which keeps MAC addresses unique across agents.
5. **Rates.** A mix entry can start sessions only on workers that own hosts of its client pool, which is the first client pool when the entry names none. A worker without such hosts gets no rate for the entry, and the entry divides its share of the objective equally over the workers that have them, so the total rate stays as configured. When every worker has hosts of every client pool, every worker carries an equal share, so the share of an agent follows its core count. The agent objective value sums the rates of its workers. `sessions_per_second` divides equally over the eligible workers. `throughput_bps` first becomes a session rate through the weighted bytes per session estimate of the mix and fails with `ErrNoByteEstimate` when a scenario has none. `concurrent_sessions` and `simulated_users` round to whole sessions, which divide over the mix by largest remainder and over the eligible workers of each entry with a rotating start, so the leftover sessions spread across workers.
6. **Phases.** A hold phase runs at the steady target. A linear or step phase ramps up from the current fraction to 1, or ramps down to 0 when the load already stands at 1. A step phase takes one step per started second.
7. **Seed.** The seed of the plans is `timeline.seed`, then `Request.Seed`, and when both are missing a random nonzero seed that the compiler draws once from `Request.Rand`, so every plan of the run carries the same seed and the caller reads it from the plans to record it in the run. A configuration seed of 0 fails with `ErrInvalidConfig`, because the minimum is 1.
8. **Module settings.** `modules.<name>.settings` validates against the JSON Schema that `Scenario.SettingsSchema` carries for the module, and a module without a schema accepts no settings. A module that the catalog lacks or that the mix does not use fails with `ErrUnknownModule`. The plan carries the validated settings in the `settings` field of the entry of the module in `modules`, and leaves the field out for modules without settings. The field is not part of `fps.agent.v1.ModuleRef` yet, so an agent that parses the plan with the current contract rejects a plan that carries settings until a contract change adds `ModuleRef.settings` and the engine reads it.
9. **Checks.** The compiler checks the rules that it relies on, such as unknown pools and ports, pool roles, pool counts against the usable addresses of their prefixes, and the inventory. Full schema validation belongs to the configuration store.

Golden tests compile the contract examples `http_smoke.yaml` on one agent and `enterprise_mix_10g.yaml` on one agent with eight cores, two agents with four and two cores, and two agents with three and two cores whose pools do not divide evenly. Three more cases cover the CTL-18 rules: `pinned_port` pins one client port to one of two agents, `small_pool` has a client pool of three hosts for four workers, and `module_settings` carries a dns `zone_file` setting. The goldens live in `controller/internal/compiler/testdata/<case>/<agent>.json`, and `python3 tools/contracts/check_plans.py --modules contracts/examples/modules controller/internal/compiler/testdata/*/*.json` accepts all of them. A determinism test compiles every case a second time with the inventory and the catalog reversed and expects identical bytes. The tests use the Go golden file helper through a `replace` line in `controller/go.mod`.
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
4. **Capacity.** The validator splits a port location as the compiler does. `<agent>/<interface>` pins the port, so the agent must be in the interface `store.AgentInventory`, which returns the available agents as `[]compiler.Agent`, and an agent that lists interfaces must have the port interface. A bare `<interface>` or `*/<interface>` spreads the port, and it needs at least one available agent with the interface, where an agent with an empty interface list counts as having it. A bare `eth1` is an interface and never an agent name. The report gives `agents_required`, which is the number of distinct pinned agents or one when only spread ports exist, and `agents_available`, which is the number of available agents that a port can use.
5. **Dry run.** When both interfaces are present and no error came up, the validator compiles the test with `compiler.Compile` and turns a compiler error into a finding at the path that the error names. The dry run takes the agents that the ports can use as its inventory. The path pattern covers `modules`, so a settings error becomes a finding at `/modules/<name>/settings`.

A nil catalog or inventory skips its check and adds an info finding, so the operation works before CTL-04 and CTL-06 land. A failing catalog or inventory answers 500. Server errors are logged with `log/slog` and never reach the client.

### Tests of the store

The three examples in `contracts/examples/tests-pending/` validate without findings against the fakes, and a bad `zone_file` gives an error at `/modules/dns/settings`. Every example under `contracts/examples/tests/` stores as YAML and as JSON, reads back unchanged, and validates without findings against fakes of the catalog and the inventory. Each fixture in `internal/store/testdata/invalid/` breaks the schema and names the expected pointer on its first line, as in `# want: /network/ports/0/location`, and the create test expects a 422 problem with an error at that pointer. Each fixture in `internal/store/testdata/unresolved/` passes the schema and must fail validation at its pointer. Coverage of the package is 93.5%.

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

A graceful stop sends `STOP_MODE_RAMP_DOWN` with `Config.DrainTimeout` to every active agent and still evaluates the criteria. A forced stop cancels the pending commands, sends `STOP_MODE_ABORT` to every agent that holds the plan, and ends the run as aborted without a verdict. A forced stop during a ramp down also aborts. The reason of a forced stop names the phase that it interrupted: `stopped while starting` when a start command was still pending, and `stopped by request` once every agent had answered it, so a test must wait for the log message `run started on every agent` before it expects the second reason. A stop before the run starts aborts it, and a stop of an ended run answers 409. `Close` aborts every open run with the reason controller shutdown.

### Fault containment

`Orchestrator.Observe` takes the events of the agent manager, and the controller wires it as `agents.Config.Observer`. An agent that reports `FAILED`, loses its plan, or goes offline after a crash or a missed heartbeat fails within its run. The run turns degraded, names the agent in `FailedAgents`, and keeps the other agents running. A failure while preparing or starting fails the run, and a run whose agents all fail ends as failed.

### Criteria

When a run stops, the orchestrator keeps the newest value of every counter of every agent with its labels and sums them over the agents into `Outcome.Series`. It also sums the unlabeled counters and gauges into `Outcome.Totals` and derives `sessions.failed_ratio`. CON-08 gives the scenario and transport metrics the `side` label, and a total over both sides counts every session twice, so `Totals` adds the labeled counters of the client side and leaves the server side out. A counter that an agent also reports without labels is a run total that repeats its labeled series, so the labeled series add up only for counters without such a total. The sessions that a failed agent attempted without success or failure count as failed, because the crash ended them. In `Series` they form a `sessions.failed` series of the reason `aborted` next to each `sessions.attempted` series. `Outcome.RunID` names the run, so an evaluator can read further metrics such as histograms.

`Config.Evaluator` decides the verdict, and a failing verdict ends the run as failed and a passing one as completed. The default `EvaluateTotals` checks every criterion whose metric appears in the totals and skips the others, and it skips a criterion with a `where` filter because the totals carry no labels. The evaluator of the report package covers the full catalog.

A criterion is a `runs.Criterion`, the `compiler.CriterionConfig` and its `Where` filter. The compiler has no `where` field yet, so `criteriaOf` leaves `Where` empty until it gains one (see the note in the log entry of CTL-20).

### Reasons

`Run.Reason` says in words why a run failed or was aborted, and `GET /v1/runs/{id}` serves it as `reason` in those two states and in no other. The orchestrator fills it at every end that is not a completion.

| End | Reason |
|---|---|
| A criterion fails or is invalid | `criteria failed: ` and up to three failed criteria, each as the metric and its reason, then `; and N more` for the rest |
| A failing verdict without a failed criterion, as when every agent failed | `every agent failed`, or `criteria failed` when an agent survived |
| Every agent failed during the run | `every agent failed` |
| A prepare or start command failed | `prepare: ` or `start: ` and the error |
| A stop request | `stopped by request`, `stopped before preparing`, `stopped while preparing`, or `stopped while starting` |
| The shutdown of the controller | `controller shutdown` |
| A rejected state change | The error of the state machine |

A run that ends as failed or aborted without a reason gets `failed` or `aborted`, so no failed run lacks one.

Every criterion of the verdict carries `observed`, `passed`, and `reason` through `GET /v1/runs/{id}` and the report. A criterion that compared a value has `observed` and `passed`. One whose metric has no value has neither, and an invalid one has `passed` false and no `observed`. `runs.CriterionResult.Invalid` marks the last case. The reason of a criterion with a filter ends with the filter, for example `observed 0.01 < 0.005 does not hold for application=dns,side=client`.

### Tests of the orchestrator

`seed_test.go` checks that the run record takes its seed from the compiled plans, which is the seed of the configuration, then `Config.Seed`, and otherwise one that the compiler draws, and that a plan without a seed fails `Create`. `state_test.go` checks all 49 pairs of states against the allowed table. `reasons_test.go` checks the reason of every way that a run fails or aborts, the client side totals over labeled counters of both sides, the aborted failures of a crashed agent, and the criterion fields of `GET /v1/runs/{id}`. The orchestrator tests use the real agent manager, fake agents, the real compiler, an in memory store, and a fake clock. `TestFailingAgentEndsRunByCriteria` lets one of two agents fail or crash under way and checks that a failure ratio of 0.5% completes the degraded run and a ratio of 30% fails it against the criterion of 10%. Other tests cover finishing agents, missed heartbeats, a graceful stop, forced stops while preparing, running, and ramping down, a prepare error, operator reservations, create errors, and `Close`. The REST tests validate every exchange against the OpenAPI document.
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

The aggregator records the six views once per second of controller time and keeps up to `Config.MaxPoints` points per run. Each view groups the merged series by one key and adds the key `*` for the whole run. A histogram appears in a sample as one value per statistic, such as `sessions.response_time.p99`. CON-08 gives the scenario, port, and transport metrics the `side` label, and a view over both sides would count each session twice, so every view reports the client side. Series without a `side` label, such as the `mix.` and `realism.` gauges, count for both sides. `Window.Side` and `Aggregator.LatestSide` select the server side for callers inside the controller. The REST contract has no side parameter, so the API reports the client side. The aggregator computes both sides of every point, and `Aggregator.Totals` still adds both sides, as the report tests document.

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

`TestClientSideIsTheDefault` in `side_test.go` sends the same sessions and handshakes from both sides and checks that the application, scenario, and tcp views and `Latest` report the client side by default and the server side when selected. `TestCountersEqualSumOfFakeAgentSnapshots` sends six rounds of cumulative snapshots from three fake agents through the real agent manager, one agent with two workers, adds a stale replay and an empty final snapshot, and checks that every merged counter equals the sum of the newest snapshots. `TestMergeMatchesReference` and `TestHistogramsOfFakeAgentsMatchReference` compare the merged histograms bucket by bucket and statistic by statistic with `testdata/reference_merge.json`, which `Snapshot::merge_all` of `engine/fps-stats` wrote. The generator lives in `testdata/refgen` and reruns with the command in its `Cargo.toml`. Further tests cover derived ratios, gauge means, views, windows and steps, limits, malformed and oversized encodings, a fuzz target of the decoder, the stream, and the Prometheus golden file.
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

The gRPC service speaks plain TCP and trusts the `agent_id` of each request, because mutual TLS arrives with CTL-15. The controller logs a warning that agent identities are not verified. We therefore bind `grpc_listen` to a loopback or a lab network until then. CTL-15 added the package that provides the certificates, and the [Mutual TLS](#mutual-tls) section lists the wiring that still has to follow.

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
| `wiring.Catalog` | `store.ModuleCatalog` and `runs.Catalog` with `Scenarios` | The scenario files of the latest installed version of each module, with the module name, version, tree digest, and the `settings` key of the manifest as `SettingsSchema`, which stays empty for a manifest without it. Configurations name a module without a version, so older versions stay out of the catalog |
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
| `getRunReport` | The `ReportHandler` of `Controller.SetReport`, which defaults to `report.API` |
| `listRuns` and captures | `pending`, which answers 501 |

### Hook for run metrics

The metrics package of CTL-08 joins the binary in three calls in `serve` of `cmd/fps-controller/main.go`, where a comment marks the place before `Mount`. `ctl.SetMetrics` takes a value with the methods `GetRunMetrics` and `StreamRunMetrics` of `api.ServerInterface`, `ctl.Observe` adds its agent event observer next to the orchestrator, and `a.Handle("GET /metrics", ...)` mounts the Prometheus endpoint outside `/v1`. The exported fields of `wiring.Controller` give the package the store, the registry, the agent manager, and the orchestrator.

### Tests of the wiring

`TestServerRoutesEveryOperation` walks every operation of the OpenAPI contract with an admin token and checks that exactly the six pending operations answer 501, each with a detail that names it, so every other operation reaches the package that implements it. `TestPendingExamplesValidateThroughTheAPI` stores the three examples of `contracts/examples/tests-pending/` through the REST API, registers fake agents `agent-a` with `eth1` and `eth2` and `local` with `lo`, installs the example modules, and expects every validation to pass the dry run, and `TestCatalogReportsSettingsSchema` expects the `dns` scenarios to carry the settings schema with `zone_file`. Other tests in `internal/wiring` cover the token scopes, the metrics hook, the catalog with two versions of `http1`, the inventory filters, and a gRPC round trip with a fake agent and the observer.

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

## Criteria and reports

CTL-09 adds `controller/internal/report`, which evaluates the pass and fail criteria of a run and serves `GET /v1/runs/{id}/report` in JSON and CSV.

### Criteria evaluation

An `Evaluator` reads the metric names of the catalog through `metrics.Catalog`. Before it compares anything, `Derive` computes every derived gauge of the catalog again from the merged counters with its formula, so `sessions.failed_ratio` and `tls.resumption_ratio` are ratios of run totals and never a mean of per agent ratios. A formula whose divisor is zero or absent leaves the gauge undefined. The evaluator applies the six operators of `contracts/schemas/test-config.schema.json`, and a test reads the operator enum from the schema, so the two lists cannot drift.

Each criterion gets one of four results with a reason in one sentence.

| Result | Meaning | Decides the verdict |
|---|---|---|
| `pass` | The observed value satisfies the rule | No |
| `fail` | The observed value breaks the rule | Yes, the run fails |
| `missing` | The run has no value for a catalog metric, for example a ratio over zero attempts or a gauge that no component computes yet | No |
| `invalid` | The criterion names no catalog metric, puts a statistic on a counter or gauge, names a histogram without a statistic, or uses an unknown operator | Yes, the run fails |

Histogram criteria take one of the statistics of `metrics.Stats`, for example `sessions.response_time.p99`. A run in which every agent failed never passes, as in CTL-07.

### Label filters and the side default

A criterion may carry a `where` object of label names and string values, as `contracts/schemas/test-config.schema.json` allows (decision D12 item 14). The evaluator then counts only the series of the metric whose labels match every pair, for example `where: { application: dns }`. Every name must be a label of the metric in the catalog, so `where: { port: eth0 }` on `bytes.tx` is `invalid` with the reason `metric bytes.tx has no label port`. A filter that selects no series leaves the criterion `missing`, with the filter in the reason.

CON-08 gives every scenario and transport metric the `side` label, which holds `client` or `server`, and a total over both sides counts each session twice. A criterion on a metric with the `side` label and a `where` that names no side therefore counts the client side only, and `where: { side: server }` selects the server side. Totals and the report summary follow the same rule. A series that carries no `side` label at all predates the label, and it counts for a criterion without a side but not for one with a side. The API metric views in `controller/internal/metrics` apply the same default since CTL-23 (see [Views and windows](#views-and-windows)).

The evaluator selects, then combines. Counters and additive gauges add over the selected series, ratio and percentage gauges average, and histograms merge before the statistic is read. A derived gauge divides the sums of its two components over the same selection, so `sessions.failed_ratio` with `where: { application: dns }` is the failed sessions of dns over its attempted sessions, and a ratio is never a mean of ratios. A missing dividend counts as zero, and a missing or zero divisor leaves the ratio undefined. The `Data` type carries either unlabeled `Totals`, which cannot answer a filter, or the labeled `Series` of the run. `Evaluate` serves totals and `EvaluateData` serves both.

### Evaluator for runs

`Evaluator.RunEvaluator` returns a `runs.Evaluator` for `runs.Config.Evaluator`. It maps the results onto `runs.CriterionResult` with their reasons, so a missing criterion is not evaluated and an invalid one is `Invalid` and fails. A test checks that it decides like `runs.EvaluateTotals` for every operator wherever both evaluate a criterion.

The evaluator decides the run state as the report decides the report. It reads the labeled series of the run from the source of `Evaluator.UseSeries` by `runs.Outcome.RunID`, merges the counters of `Outcome.Series` over them, and evaluates every criterion with its filter, the side default, and its histogram statistics. `NewAPI` sets that source when its totals source offers series, as `*metrics.Aggregator` does, so the call that the binary already makes, `report.NewAPI(ctl.Evaluator, ctl.Runs, agg)`, also connects the run evaluator. Without a source and without series the evaluator falls back to `Outcome.Totals`, which hold no histograms. This closes note 4 of the CTL-09 log entry.

### Report

`Evaluator.BuildSeries` joins the run of the orchestrator and the labeled series of the aggregator, and `GetRunReport` uses it whenever the totals source offers series or the run holds series. `Evaluator.Build` joins the run and the totals of an aggregator without labels, as the tests of the handler do. In both, counters of the run take precedence, because the orchestrator counts the open sessions of a failed agent as failed and sees the final snapshot of every agent. Gauges and histogram statistics come from the aggregator, and the derived gauges are computed from the result. With series, the summary and the metric rows count the client side of the metrics that have a side. A run that never reached its criteria, such as a running or aborted run, gets a report without a verdict.

| Format | Content |
|---|---|
| `json`, the default | The `RunReport` schema, with the summary filled from the run totals, every criterion in the verdict with its `reason`, `observed` and `passed` only for compared criteria, `reason` of the run when it failed or aborted, and an empty `realism` list |
| `csv` | One table with the columns `kind,name,op,threshold,value,result,detail`, with rows of kind `run` for the run fields, `criterion` for each criterion, and `metric` for each run total in catalog order |
| `html` | 501 until a package adds it |

The CSV report writes numbers in plain decimal notation and prefixes text cells that start with `=`, `+`, `-`, or `@` with an apostrophe, so spreadsheets do not run them as formulas.

### Tests of the report

`TestGoldenReports` serves a passing run, a run with failing criteria, and a degraded run through the generated router, validates every response against the contract, and compares the JSON and CSV bodies with the golden files in `controller/internal/report/testdata/`. In the degraded run one agent crashed with open sessions, and the report counts them as failed. `TestGoldenLabeledReport` adds a failed run with a reason over a real aggregator that holds two applications on both sides. Its golden files `labeled.json` and `labeled.csv` cover filtered criteria (`application`, `side`), the client side default, a histogram criterion on each side, an invalid label, and a filter without a series. `TestClientSideIsTheDefault` checks the same numbers for a criterion without a side and one with `side=server`, and shows that the totals of the aggregator count each session twice. `TestRunEvaluatorDecidesHistogramsByRunID` checks that the run evaluator reads the aggregator by run id and gives the same results as the report. Other tests cover every operator, every catalog formula, the four results, the error responses, and a report built from the real aggregator of CTL-08 with two agents.
## Metrics in the binary

Bug #119 closed the hook that the Binary wiring section describes. The function `mountMetrics` in `cmd/fps-controller/main.go` runs in `serve` before `ctl.Mount(a)` and makes four calls.

1. `metrics.New(metrics.Config{Logger: logger})` creates one aggregator with the default catalog, the system clock, and the default bounds.
2. `ctl.Observe(agg.Observe)` adds the aggregator to the agent manager observers after the run orchestrator, so every counter snapshot of an agent reaches both.
3. `ctl.SetMetrics(metrics.NewAPI(agg, ctl.Runs))` routes `GET /v1/runs/{id}/metrics` and `GET /v1/runs/{id}/metrics:stream` to the aggregator, with the run orchestrator as the `RunSource`. A run that the orchestrator knows answers 200 before its first snapshot, and the stream ends when the run ends.
4. `a.Handle("GET /metrics", metrics.NewPrometheus(agg))` serves the Prometheus exposition outside `/v1`.

`GET /metrics` needs no token by default, because Prometheus scrapers usually send none, and CTL-19 added the optional setting `metrics_token_file`, which [Metrics endpoint token](#metrics-endpoint-token) describes. The exposition holds run identifiers, application names, scenario names, and port names, so an operator who considers them sensitive sets that token or keeps the listen address on a trusted network. The exported field `wiring.Controller.Runs` already gives the orchestrator, so the wiring package needed no new accessor.

`TestEndToEnd` now runs one more step while the run is running. The fake agent sends a snapshot with `sessions.attempted` equal to 7 for the application `http1` and the scenario `web_get`. The test polls until `GET /v1/runs/{id}/metrics?view=application` returns the key `http1` with that count and `GET /metrics` contains the line `fps_sessions_attempted_total{run_id="<run id>",application="http1",scenario="web_get"} 7`.

## Reports in the binary

Bug #124 joined the criteria evaluator and the report endpoint of CTL-09 to the binary. The wiring adds two pieces.

1. `wiring.New` creates `report.NewEvaluator(nil)` with the embedded metrics catalog before the run orchestrator, keeps it in the exported field `Controller.Evaluator`, and passes `Evaluator.RunEvaluator()` as `runs.Config.Evaluator`. Every stopped run therefore derives the catalog formulas, such as `tls.resumption_ratio`, from its totals and gives each criterion the result pass, fail, missing, or invalid. The simple evaluator `runs.EvaluateTotals` no longer runs in the binary.
2. `wiring.Server` forwards `getRunReport` from `pending` to a `ReportHandler`. `wiring.New` sets `report.NewAPI(ev, ctl.Runs, nil)`, which builds reports from the run totals alone, and `Controller.SetReport` replaces it before `Mount`. The function `mountMetrics` in `cmd/fps-controller/main.go` calls `ctl.SetReport(report.NewAPI(ctl.Evaluator, ctl.Runs, agg))`, so the binary builds reports over the merged metrics of the aggregator. Since CTL-20 the same call also hands the aggregator to the run evaluator, which reads it by run id when a run stops.

`GET /v1/runs/{id}/report` answers JSON by default and CSV with `format=csv`. The format `html` still answers 501 from `report.API`. `TestServerRoutesEveryOperation` now expects six pending operations, and `TestNewServesRunReports` and `TestSetReportRoutesReportOperation` cover the default handler and the hook.

`TestEndToEnd` proves the path through the binary. The example configuration gains the criterion `tls.resumption_ratio >= 0.5`, whose gauge only the CTL-09 evaluator computes. The fake agent snapshot carries `sessions.attempted`, `sessions.succeeded`, `sessions.failed`, `tls.handshakes`, and `tls.resumptions` both with labels and without, because the orchestrator reads only the unlabeled run totals. After the stop the test reads both criteria as passed with the observed values 0 and 0.5 from `GET /v1/runs/{id}`, from the JSON report, and from the criterion rows of the CSV report, and it checks `sessions_attempted` 7 in the report summary.

## Metrics endpoint token

Decision D12 item 2 keeps `GET /metrics` open by default and lets an operator require a bearer token. CTL-19 added the setting.

```yaml
metrics_token_file: /etc/fps/metrics.token   # empty or absent keeps GET /metrics open
```

| Key or flag | Default | Meaning |
|---|---|---|
| `metrics_token_file`, `-metrics-token-file file` | none | File that holds the bearer token for `GET /metrics`. Without it the endpoint stays open |

The setting names a file and not the secret itself, so the token never appears in the configuration, in a command line, or in a log record. The controller trims the white space around the content of the file, as it does for `token_file` of the API tokens. The token holds at least 16 bytes. A file that the controller cannot read, or a token that is too short, ends the process with exit code 2. The flag overrides the file key like every other flag. The token is independent of `tokens`, so an API token does not open the metrics endpoint and the metrics token does not open the API.

With the setting, a request to `GET /metrics` must carry `Authorization: Bearer <token>`, where the scheme name ignores case. Any other request answers 401 with a `WWW-Authenticate: Bearer` challenge and never reaches the exposition. The check hashes the presented value and the configured token with SHA-256 and compares the digests with `crypto/subtle.ConstantTimeCompare`, so neither the content nor the length of the token leaks through timing. `/healthz` stays open in every case. At startup the controller logs `metrics endpoint is open` or `metrics endpoint requires a bearer token` with the path, and never the token.

Prometheus sends the token with `authorization.credentials_file`:

```yaml
scrape_configs:
  - job_name: fps-controller
    authorization:
      credentials_file: /etc/prometheus/fps-metrics.token
    static_configs:
      - targets: ["controller.lab:8443"]
```

`app.RequireBearer(token, next)` implements the check, and `mountMetrics` wraps `metrics.NewPrometheus` with it. Unit tests in `internal/app` cover an open endpoint, a required token, a wrong token, a prefix and an extension of the right token, other schemes, and the file loader. Tests in `cmd/fps-controller` start the controller in process and check the three cases over a real socket, check that the logs never hold the token, and check exit code 2 for an unreadable token file.

## Run ids

Bug #128 gave run ids their own prefix. The run orchestrator used `store.RandomID` as the default of `runs.Config.NewID`, so run ids started with `cfg_` and read like configuration ids in logs, reports, and the API. The package `runs` now has its own `runs.RandomID`, which returns `run_` and 24 random hex digits, and `runs.New` uses it when `NewID` is nil. The constant `runs.RunIDPrefix` names the prefix. Every id matches the contract pattern `^[A-Za-z0-9_-]{1,64}$`, and the OpenAPI document sets no prefix, so the change touches no contract. Clients treat run ids as opaque strings, and the end to end script `engine/fps-agent/tests/controller_e2e.sh` reads the id from the response and only prints it. `TestDefaultRunIDsCarryRunPrefix` checks the generator and a run that the orchestrator creates with the default generator.

## Throughput rate calibration

The package `internal/runs/calibrate/` (CTL-21) applies decision D12 item 10. The compiler turns a `throughput_bps` objective into session rates with the `expected_bytes` estimate of the module manifest, so a session rate is `objective / (8 * mean bytes per session)` (see `splitRates` in `internal/compiler/`). When the estimate is wrong, the run misses the objective by the same ratio. The calibration measures the real bytes per session during the run and returns a `Factor` that scales every compiled session rate. The package uses only the standard library, reads no clock, and uses no random source, and the orchestrator does not call it yet.

### Interface

| Item | Meaning |
|---|---|
| `calibrate.New(Config)` | Validates the configuration and returns a `State` with a factor of one, which leaves the compiled rates unchanged |
| `State.Update([]Sample)` | Folds one measurement window into the state and returns the next state and a `Result`. The receiver stays unchanged, so the same inputs always give the same outputs |
| `Config` | `ObjectiveBps`, and one `Entry` per mix entry with its `ID`, session `Weight`, and the compiler's `ExpectedBytes`. The tuning fields `MaxStep`, `DeadBand`, `MinSessions`, `Forget`, and `MaxFactor` take the defaults below when zero |
| `Sample` | For one mix entry and one window: `Bytes` that the completed sessions moved in both directions, and the number of `Sessions` |
| `Result` | `Factor`, `Changed`, the unbounded `Target`, the implied total `SessionsPerSecond`, and the bytes per session estimate of each entry |

### Method

The calibration estimates a property of the traffic, the bytes per session, and not the achieved throughput. It therefore has no feedback delay, and the ramp up of the agents cannot make it oscillate. Each update decays the pooled byte and session sums of every entry by `Forget` and adds the window. An entry whose pooled session count reaches `MinSessions` replaces its manifest estimate with pooled bytes divided by pooled sessions. The target factor is the ratio of the compiled to the measured weighted mean bytes per session. Three bounds then limit the move to the target.

| Bound | Default | Effect |
|---|---|---|
| `MaxStep` | 0.5 | One update changes the factor by at most a factor of 1.5, up or down, so a 4x wrong estimate needs four updates and never overshoots without noise |
| `DeadBand` | 0.015 | The factor holds while the target stays within 1.5 percent. A move against the direction of the previous move needs 2.25 percent, which stops zigzag around a settled factor |
| `MaxFactor` | 64 | The factor stays within 1/64 and 64 |
| `MinSessions` | 20 | A window with few sessions does not replace the manifest estimate |
| `Forget` | 0.97 | The effective window is about 33 updates, which damps noise and still follows a slow change of the mix |

### What the orchestrator must do

The orchestrator calls `calibrate.New` when a run with a `throughput_bps` objective starts, with the entries and `ExpectedBytes` values that it passed to the compiler. The compiler already normalizes weights, and the package normalizes again. After each measurement window, for example every 5 to 10 seconds while the run is in its steady phase, it builds one `Sample` per mix entry from the metric deltas of that window and calls `Update`. We suggest the client side counters of the scenario view, which are `bytes.tx` plus `bytes.rx` and `sessions.succeeded`, so that each payload byte counts once. When `Result.Changed` is true, it multiplies every `SessionsPerSecond` of the compiled worker rates of that entry by `Result.Factor` and sends the new rates to the agents. That message is `UpdateRates` of CON-05, which is in progress, and the agent handler is AGT-05. Concurrency objectives and `sessions_per_second` objectives need no calibration.

### Tests

`calibrate_test.go` simulates the closed loop with noisy sessions (a standard deviation of 40 percent per session and 5 percent per window) for estimates of 0.25, 0.5, 1, 2, and 4 times the true bytes per session, over 40 seeds each. Every run stays within 5 percent of the objective from update 25 on, every step stays within the bound, settled steps stay within 5 percent, and direction reversals stay few. A noise free run moves monotonically, a step test checks the exact 1.5x sequence, and further tests cover mixes of entries with unnormalized weights, the factor bound, the dead band, the minimum session count, immutability of the state, and every validation error.

## Mutual TLS

CTL-15 added the package `internal/pki`. It gives the control channel between the controller and its agents mutual TLS, and it uses only the standard library (`crypto/x509`, `crypto/tls`, `crypto/ecdsa`). The package is not wired into the gRPC server or the agent yet, so the binary still speaks plain TCP. [What the wiring must change](#what-the-wiring-must-change) lists the rest.

```go
pa, err := pki.Open(pki.Config{Dir: "/var/lib/fps/pki", OnRevoke: endSession}) // CA on first start
tok, expires, _ := pa.NewToken("agent-a", time.Hour)       // the operator hands tok to the agent once
tlsCfg, _ := pa.ServerTLSConfig("ctl.lab.example", "10.0.0.5")
grpcServer := grpc.NewServer(grpc.Creds(credentials.NewTLS(tlsCfg)))
enrollCfg, _ := pa.EnrollTLSConfig("ctl.lab.example", "10.0.0.5")
// serve pa.Handler() behind enrollCfg on a second listener
pa.Revoke("agent-a")                                       // ends the identity at once
```

### The CA

`Open` generates the CA on the first start and loads it afterwards. The directory holds three files. `ca.key` has mode 0600 and the directory has mode 0700, `ca.crt` is public, and `state.json` has mode 0600 and holds the enrollment state. The CA is an ECDSA P-256 key with a 10 year certificate that may sign only end entity certificates (`MaxPathLen` 0). `Open` refuses to start when `ca.key` is readable by group or others, when only one of the two files exists, when the certificate does not match the key, when the CA expired, or when `state.json` is damaged, because each case means that the trust anchor is not what the controller wrote. An empty `Dir` keeps everything in memory for tests. `CAFingerprint` returns the SHA-256 digest of the CA certificate, which an operator can pin on the agent. The controller must be the only writer of the directory.

The controller server certificate comes from the same CA. The authority creates it in memory for the host names and addresses that agents dial, which become its subject alternative names, lets it live for 30 days, and replaces it when a third of its life remains.

### Enrollment tokens

`NewToken(agentID, ttl)` creates a token of the form `id.secret` for one agent identity. The identity must match the Id pattern of the REST contract. The lifetime defaults to one hour and has the cap of seven days. The state keeps only the SHA-256 digest of the secret, so the returned string is the only copy. A check looks up the record by id and compares the two digests with `crypto/subtle.ConstantTimeCompare`, and an unknown id costs the same work as a wrong secret. A token works once. The exchange marks it used before it issues the certificate, under one lock, so 16 concurrent exchanges of one token yield one certificate, and a failed write of the state undoes the mark.

| Failure | Error |
|---|---|
| Unknown id, wrong secret, or malformed token | `ErrInvalidToken`, which never says which |
| A valid token that was used before | `ErrTokenUsed` |
| A valid token past its expiry | `ErrTokenExpired` |
| A CSR that the CA cannot use | `ErrInvalidCSR`, and the token stays unused |

The HTTP handler answers all token and certificate failures with the same 401 body, so a caller learns nothing from the difference.

### Certificates, rotation, and revocation

`Enroll(token, csrDER)` issues a client certificate for the identity that the token names. The CSR must carry an ECDSA key on P-256 or P-384 with a valid signature, and its subject must be empty or name the same identity. The CA copies nothing from the CSR except the public key, so an agent cannot ask for another name, a DNS name, or a CA certificate. The certificate has the subject common name `agentID`, the client authentication usage only, and a lifetime of 24 hours by default (`Config.ClientLifetime`). The identity comes from the common name, which is also what `agents.PeerCertIdentity` reads, so the agent manager needs no change to take the identity from the certificate.

`Renew(cert, csrDER)` issues a new certificate for the identity of a certificate that the agent presented on a TLS connection. It verifies the presented certificate against the CA again, so it works on its own. An expired, foreign, or revoked certificate fails, and the agent then enrolls again with a new token. `RenewAt` gives the time to renew, which is when a third of the lifetime remains, and the response of the handler carries it as `renew_at`. A new key per renewal is the agent's choice. The old certificate stays valid until it expires, so an agent that lost the reply can try again.

The authority keeps an allow list of the serial numbers that it issued, with their identity and expiry. `Revoke(agentID)` deletes the certificates and the unused tokens of the identity, and it calls `Config.OnRevoke` so that the caller can close the open session, because a TLS handshake is the only place where the certificate is checked. A certificate that the CA signed but that the allow list does not hold fails closed. This covers a lost state file and replaces a revocation list. The agent can enroll again only with a token that `NewToken` creates after the revocation. The allow list drops expired entries on every write, so it stays small.

### The TLS configuration

`ServerTLSConfig(names...)` returns the `tls.Config` for the gRPC server. It requires TLS 1.3 and offers `h2`. It sets `ClientAuth` to `RequireAndVerifyClientCert` with the CA as the only client root, so the standard library verifies the chain, the validity, and the client authentication usage. A `VerifyConnection` hook then calls `Authorize`, which checks the allow list and that the serial belongs to the identity in the subject. Session tickets are off, so every connection runs the full handshake and a revocation holds at the next dial. The configuration reads the time from the authority's clock. A client without a certificate, with a certificate of another CA, with an expired certificate, or with a revoked identity never completes the handshake.

`EnrollTLSConfig(names...)` serves the second listener of `Handler`. It does not require a client certificate, because an agent that enrolls has none, and it verifies a certificate that the client does present, which `POST /renew` relies on.

| Endpoint | Body | Answer |
|---|---|---|
| `POST /enroll` | `{"token", "csr_pem"}`, no client certificate | 200 with `agent_id`, `certificate_pem`, `ca_pem`, `not_before`, `not_after`, `renew_at`. 401 for any token failure, 400 for a bad body or CSR |
| `POST /renew` | `{"csr_pem"}`, presents the current certificate | The same answer. 401 without a valid certificate, 400 for a bad CSR or another identity |

`ClientTLSConfig`, `KeyPair`, and `NewCSR` serve Go clients and tests.

### What the wiring must change

The controller wiring (`internal/app`, `internal/wiring`, and `cmd/fps-controller`, outside CTL-15) has to do six things.

1. Open the authority in `wiring.New` from a directory such as `data_dir/pki`, with new settings for the directory, the names for the server certificate, and the address of the enrollment listener.
2. Build the gRPC server with `grpc.Creds(credentials.NewTLS(pa.ServerTLSConfig(names...)))` in place of plain TCP, serve `pa.Handler()` behind `pa.EnrollTLSConfig(names...)` on the second listener, and keep `agents.Config.Identity` at `PeerCertIdentity`, which is already the default. `ClaimedIdentity` and the startup warning then go away. `Register` already answers `UNAUTHENTICATED` when the identity differs from `agent_id`.
3. Give the agent manager a method that ends the session of one agent and pass it as `Config.OnRevoke`. The manager should also end or refuse a session whose certificate expired, once the agent is idle, because Go checks the certificate at the handshake only.
4. Give operators a way to create tokens and revoke identities. The OpenAPI contract has no operation for it, so this needs a contract change (for example an optional `enrollment_token` in the answer of `POST /v1/agents` and a revoke operation) or a subcommand of the binary. A subcommand must not write `state.json` while the controller runs, because the controller is the single writer.
5. Decide what `agent_auto_enroll` means with certificates. We suggest that it only lets an identity with a valid certificate skip `POST /v1/agents`, and that it never replaces a certificate.
6. Update the note in the image that says the controller and gRPC are plain until TLS lands, and have the image provisioning write the enrollment token for the agent the way it writes the REST token today.

The Rust agent (`engine/fps-agent`, AGT) has to do the following.

1. Build the tonic channel with TLS. This needs the tonic `tls` feature with a rustls provider. The channel trusts the CA from a file or a pinned fingerprint, presents the client certificate and key, uses the `https://` scheme, and sets the server name to a name in the controller certificate. The check for `http://` addresses can stay for local tests.
2. Enroll when it has no certificate. It generates an ECDSA P-256 key, writes it with mode 0600, builds a CSR with the common name set to its `--agent-id`, and sends the token and the CSR to the enrollment endpoint over TLS that trusts the CA. The token comes from a file or an environment variable, never from the command line, and the agent deletes it after the exchange. New flags are needed for the CA file, the token source, the enrollment address, and the directory that holds the key and the certificate.
3. Renew. From `renew_at` on, and before the certificate expires, it calls `POST /renew` with its current certificate and a fresh CSR, writes the new files atomically, and uses them for the next dial. It should redial only while no plan runs, because D12 item 12 drops the plan on a lost session. When the certificate is expired or revoked it logs the reason and enrolls again only if a new token exists.
4. Treat `UNAUTHENTICATED` and a handshake failure as permanent, because redialing cannot fix them, and follow the redial limit of D12 item 11.

#### Is a contract change needed

Not for this package, and the first wiring can go ahead without one, because the enrollment endpoint is an HTTPS API that `pki` serves and that no proto file describes. The endpoint adds a new HTTP client to the agent (for example `hyper-util` or `reqwest` with rustls), which is the cost of this choice. The alternative is an `Enroll` and a `Renew` RPC in `fps.agent.v1`. The agent then reuses its tonic stubs and needs no HTTP client, and one listener can serve everything with `VerifyClientCertIfGiven`, since `PeerCertIdentity` already refuses a call without a certificate. That choice is a CON change with a regeneration of `agentpb` and the Rust stubs, and the owner approves contracts, so we did not make it. We recommend the RPC when the owner wants a single listener and one client stack, and the HTTP endpoint otherwise. The REST operations for tokens and revocation need a contract change in both cases, unless a subcommand serves them.

### Tests

The tests open real TLS connections on the loopback interface. `TestMutualTLSAdmitsOnlyValidCertificates` starts a TLS server with `ServerTLSConfig`, shows that a freshly enrolled certificate is accepted and that the server reads the agent identity from the certificate subject, and shows that these clients are rejected: no client certificate, a certificate of another CA, a certificate that the CA signed without a record, a revoked identity, and an expired certificate. The clock of the authority is settable, so the expiry test moves it past `NotAfter`. `TestTokens` covers reuse, expiry at the exact instant, wrong and unknown and malformed tokens, a CSR for another identity, an RSA key, the lifetime limits, and 16 concurrent exchanges of one token. `TestRotation` renews a certificate directly and checks that an expired, foreign, or revoked certificate cannot renew. `TestHandlerEnrollAndRenewOverTLS` drives both endpoints over TLS. Further tests check the file modes of the CA, the refusal of a loose key, a half CA, a mismatched pair, a damaged state, and an expired CA, that the state never holds a token secret, and that used tokens and revocations survive a restart.

`make test-ctl-pki lint-ctl-pki` runs the package alone, and `make lint-ctl test-ctl` covers it with the rest of the module.
