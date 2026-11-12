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
| `internal/registry/signing/` | Ed25519 signatures for module archives and the registry index, key ids, and the trust store with rotation (CTL-11) |
| `internal/agents/` | The agent manager with Register and Session, the inventory, and /v1/agents (CTL-06) |
| `internal/fakeagent/` | Fake agents for controller tests (CTL-06) |
| `internal/runs/` | The run state machine, the orchestrator, and the run operations of /v1/runs (CTL-07) |
| `internal/metrics/` | Snapshot aggregation, the metrics views and stream, and Prometheus exposition (CTL-08) |
| `internal/report/` | Criteria evaluation and the JSON and CSV run reports (CTL-09) |
| `internal/report/html/` | The self contained HTML run report with inline SVG charts (CTL-13) |
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
| `-version` | false | Print the version and exit. The version is the value that `-ldflags "-X main.version=..."` stamped in, or the module version of the Go build information when the build has no stamp, or `(devel)`. The startup log line `controller starting` carries the same `version` field. `GET /v1/capabilities` does not report it, because the contract has no field for the controller version (`api_version` is the REST contract version) |

The binary also has two subcommands for operators, `fps-controller token` and `fps-controller revoke`, which [Operators, tokens, and revocation](#operators-tokens-and-revocation) describes.

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
| `make lint-ctl-registry`, `make test-ctl-registry` | The same checks for `internal/registry` alone (CTL-25) |
| `make lint-ctl-captures`, `make test-ctl-captures` | The same checks for `internal/captures`, the upload path of `internal/agents`, and `internal/fakeagent` (CTL-14) |

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
## Signature checks at install and in plans

CTL-28 connected the signing package of CTL-11 to the registry and the compiler. `Registry.Install` verifies the signature of an archive before it unpacks anything, the registry stores the signature with the module, and the compiler copies it into every plan.

### Trust policy

The registry takes a `TrustPolicy` through `registry.Open(root, limits, registry.WithTrust(policy))`. Two settings of the controller configuration fill it, and the controller logs a warning at start up when no trust store is set.

```yaml
trust_store_file: /etc/fps/trust.json   # empty or absent: signatures are not required
allow_unsigned_modules: false           # development setting, only meaningful with a trust store
```

| Key | Default | Meaning |
|---|---|---|
| `trust_store_file` | none | JSON file that `signing.TrustStore.Encode` writes. The registry reads it when it opens and again at every install, so a key that an operator revokes in the file stops verifying without a restart. A missing or malformed file stops the controller at start, and an install fails closed when the file goes bad later |
| `allow_unsigned_modules` | `false` | Lets the registry install a module that has no signature, and logs a warning for each. A signature that fails is refused in every case |

CTL-31 added the flag forms `-trust-store-file file` and `-allow-unsigned-modules`. They follow the usual layering, so a flag that appears on the command line overrides the configuration file, and `-allow-unsigned-modules=false` switches the file setting off.

The rules, in the order that `Install` applies them:

1. **No trust store.** The registry does not require signatures. It installs every module that passes validation and logs a warning, so a controller that nobody configured for signing and the example modules keep working. A signature that accompanies the module cannot be checked, so the registry ignores it and stores none, because a stored signature always means that the registry verified it.
2. **Trust store, no signature.** The install fails with `ErrUntrusted`, which wraps `signing.ErrNoSignature`, unless `allow_unsigned_modules` is on. `InstallDir` follows the same rule, so seeding a registry from a module directory needs a signature too.
3. **Trust store, signature.** `Verifier.VerifyArchive` digests the archive and checks the signature before the registry unpacks. A modified archive, an unknown, revoked, or expired key, a wrong key id, and a truncated signature all fail with `ErrUntrusted` wrapping the cause of the signing package, for example `signing.ErrBadSignature` or `signing.ErrUnknownKey`. The development setting does not soften this. An archive that cannot be read still fails with `ErrArchive` or `ErrArchiveTooLarge`.

To verify the bytes that it later unpacks, the registry first copies the archive into a file in `staging/`, which it removes at the end, bounded by the size limit plus 1 KiB per allowed file. After unpacking and validation it checks that the digest of the tree equals the digest that the signature covered. `InstallDir` verifies the digest of the staged tree before the module enters the registry.

### Passing the signature

The signature is not part of the module tree, so it travels beside the archive. The Go call is `Install(ctx, archive, registry.WithSignature(sig))`. The REST operation `installModule` reads two request headers, `X-Module-Signature-Key-Id` and `X-Module-Signature`, which the OpenAPI contract names since 0.5.0 (CON-14).

| Header | Value |
|---|---|
| `X-Module-Signature-Key-Id` | The key id, for example `ed25519:fe812c12f3ab4ce6ac5db69ac352f906` |
| `X-Module-Signature` | The 64 byte signature in standard base64 |

A request with only one of the two, or a signature that is not base64, answers 400. A refused signature answers 403 with the reason in the problem detail.

### Stored signature and plans

`index.json` gives each entry that was installed with a verified signature two more fields, `signature` (standard base64) and `signature_key_id`. Entries without a signature leave them out, and the index format version stays 1, so older indexes load as before. `Entry.StoredSignature` decodes them. The signature does not change the tree digest.

The compiler stays free of I/O. The catalog adapter in `internal/wiring` copies the two fields of the entry into `compiler.ModuleRef`, and the compiler writes them to `PlanModule.signature` (base64, the proto3 JSON form of bytes) and `PlanModule.signatureKeyId`, the fields of `fps.agent.v1.ModuleRef`. A module without a signature adds no fields, so the golden plans did not change. The compiler still signs nothing and does not verify. The agent verifies the signature against its own trust store (AGT-04).

### Tests

`make test-ctl-trust` runs the registry, compiler, app, and wiring packages. The registry tests reuse the key of the CTL-11 golden vectors (seed 0x07) and cover a signed module in plain and gzip form, a modified archive, an unknown, revoked, and retired key, a truncated signature, an unsigned module with and without the development setting, no trust store, a trust store file that changes and breaks, `InstallDir`, the spool bound, and the HTTP headers. The compiler tests check that a plan carries the signature and key id of a module and that unsigned modules add no fields.

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

The compiler applies the ten rules below.

1. **Mix.** Each entry resolves to `<module>/<scenario>`, and a bare name must be unique in the catalog. Weights normalize to sum 1. With `profile.basis: sessions` they are the session shares. With `bytes` (CTL-34) the compiler turns the byte weights into session shares, `share_i = (weight_i / bytes_i) / sum_j (weight_j / bytes_j)` with `bytes_i` the manifest `expected_bytes` of the scenario, so sessions at these shares carry the byte weights when the estimates are right. A scenario without a positive finite estimate fails the compilation with `ErrNoByteEstimate` for the `bytes` basis, whatever the objective, because a silent fallback to session shares would run another mix than the one that the test asks for. The plan always carries session shares in `mix[].weight` (the worker rates divide a session rate by them). The byte weighted mix control (CTL-12, [Byte weighted mix control](#byte-weighted-mix-control)) corrects the rates during the run when the estimates are wrong. The entry id is the scenario name, with `_2`, `_3`, and so on for repeats.
2. **Workers.** Agents sort by id, and each agent contributes one worker per core, where worker i pins to core i.
3. **Ports.** A location `<agent>/<interface>` pins the port to that one agent, which must be in the inventory and must have the interface, or the compiler fails with `ErrInvalidInventory`. A bare `<interface>` or `*/<interface>` spreads the port across every agent that has the interface, where an agent with an empty interface list counts as having it, and fails with `ErrInvalidInventory` when no agent has it. A plan lists only the ports and the host pool slices of its own agent. When a port runs on fewer than all agents, the mix entries name their default pools explicitly, because an agent that lacks the first pool of a role would otherwise default to another pool, and an agent keeps a mix entry in its plan only when it has a rate for it or emulates the hosts of its server pool.
4. **Host pools.** Each pool divides over the workers of the agents that run its port, in global worker order. Every such worker gets the pool count divided by the number of those workers, and the first workers get one more host each until the remainder is used up. An agent therefore owns a contiguous slice. A pool with fewer hosts than workers is valid and leaves the last workers without hosts, but the compiler fails with `ErrPoolTooSmall` when an agent that runs the port would get no host of the pool. A pool without `mac_base` gets the locally administered base `02:xx:yy:00:00:00`, where xx and yy hold the pool index, which keeps MAC addresses unique across agents.
5. **Rates.** A mix entry can start sessions only on workers that own hosts of its client pool, which is the first client pool when the entry names none. A worker without such hosts gets no rate for the entry, and the entry divides its share of the objective equally over the workers that have them, so the total rate stays as configured. When every worker has hosts of every client pool, every worker carries an equal share, so the share of an agent follows its core count. The agent objective value sums the rates of its workers. `sessions_per_second` divides equally over the eligible workers. `throughput_bps` first becomes a session rate through the weighted bytes per session estimate of the mix (the mean over the session shares, so for the `bytes` basis the byte weighted mean bytes per session of rule 1) and fails with `ErrNoByteEstimate` when a scenario has none. `concurrent_sessions` and `simulated_users` round to whole sessions, which divide over the mix by largest remainder and over the eligible workers of each entry with a rotating start, so the leftover sessions spread across workers.
6. **Phases.** A hold phase runs at the steady target. A linear or step phase ramps up from the current fraction to 1, or ramps down to 0 when the load already stands at 1. A step phase takes one step per started second.
7. **Seed.** The seed of the plans is `timeline.seed`, then `Request.Seed`, and when both are missing a random nonzero seed that the compiler draws once from `Request.Rand`, so every plan of the run carries the same seed and the caller reads it from the plans to record it in the run. A configuration seed of 0 fails with `ErrInvalidConfig`, because the minimum is 1.
8. **Module settings.** `modules.<name>.settings` validates against the JSON Schema that `Scenario.SettingsSchema` carries for the module, and a module without a schema accepts no settings. A module that the catalog lacks or that the mix does not use fails with `ErrUnknownModule`. The plan carries the validated settings in the `settings` field of the entry of the module in `modules`, and leaves the field out for modules without settings. The field is not part of `fps.agent.v1.ModuleRef` yet, so an agent that parses the plan with the current contract rejects a plan that carries settings until a contract change adds `ModuleRef.settings` and the engine reads it.
9. **Checks.** The compiler checks the rules that it relies on, such as unknown pools and ports, pool roles, pool counts against the usable addresses of their prefixes, and the inventory. Full schema validation belongs to the configuration store.
10. **Arrival and size declarations.** The strict decoder accepts the optional `arrival` and `size` objects of test configuration v0.3 (CON-19) on `profile` and on every mix entry, with the types `Arrival`, `SizeDist`, and `Size`, which keep each parameter as a pointer so that a parameter that is absent differs from 0. They are targets for the realism check and add no command, so a plan with them equals the plan without them (CTL-38). `Config.Declarations()` returns them for the report: `profile.arrival`, `profile.size`, and for each mix entry its own `arrival` and its `size` after each direction that the entry names has overridden the same direction of `profile.size`. An entry without an `arrival` of its own has none and does not take `profile.arrival`, which describes the gaps of the whole run. The method returns nil when the test declares nothing. The decoder stays strict inside the new objects, so a misspelled parameter is an error, and the schema of the configuration store decides which parameters each distribution takes.

Golden tests compile the contract examples `http_smoke.yaml` on one agent and `enterprise_mix_10g.yaml` on one agent with eight cores, two agents with four and two cores, and two agents with three and two cores whose pools do not divide evenly. Three more cases cover the CTL-18 rules: `pinned_port` pins one client port to one of two agents, `small_pool` has a client pool of three hosts for four workers, `module_settings` carries a dns `zone_file` setting, and `distributions` carries the arrival and size declarations of rule 10 and has the plan of `module_settings` without the setting, which shows that the declarations add no command (`TestDistributionsLeaveThePlanUnchanged` compares the two compilations as well). `enterprise_mix_10g` is the golden for the `bytes` basis: the byte weights 60, 39.9, and 0.1 become the session shares 0.0827, 0.6875, and 0.2298 with the test estimates of 250 kB, 20 kB, and 150 B, and the worker rates carry 10 Gbit/s. `bytes_basis_test.go` checks the same properties by arithmetic (shares, byte shares, objective, and the missing estimate). The goldens live in `controller/internal/compiler/testdata/<case>/<agent>.json`, and `python3 tools/contracts/check_plans.py --modules contracts/examples/modules controller/internal/compiler/testdata/*/*.json` accepts all of them. A determinism test compiles every case a second time with the inventory and the catalog reversed and expects identical bytes. The tests use the Go golden file helper through a `replace` line in `controller/go.mod`.
### Criterion labels

A criterion with a `where` filter becomes a plan criterion with `labels` (CTL-36, CON-15): `Criterion.Labels` is a copy of the filter, and `where: {application: dns, side: server}` gives `"labels": {"application": "dns", "side": "server"}` in every plan. A criterion without a filter carries no labels. The controller still applies the filter to the series that it aggregates, as the report does, and the agent applies the labels as protocol 0.2.3 describes.

An agent below engine_api 0.2.3 skips `labels`, counts every series of the metric, and gives another verdict, so the controller never sends it such a plan and never drops the labels without a word. Two checks refuse the plan:

- `compiler.Compile` fails with `compiler.ErrAgentTooOld` when an agent of `Request.Agents` has an `EngineAPI` that is older than 0.2.3 or does not parse. The message names the criterion, the agent, and its version, and says to remove the filter or upgrade the agent. The run orchestrator fills `EngineAPI` from the registration of each reserved agent, so `POST /v1/runs` fails at once, with the 422 of an invalid request and before the run starts. An empty `EngineAPI` means unknown and passes this check, which keeps the inventory that validates a configuration (`wiring.Inventory`) free of versions, because the agents of a validation need not be the agents of the run.
- `agents.Manager.Prepare` fails with `agents.ErrPlanUnsupported`, without sending, when the plan has a criterion with labels and the agent reports an engine_api below 0.2.3. It covers an agent whose version changed after the compile and a caller that builds a plan itself.

The Rust agent applies the labels since AGT-14 (at least `side`), so a test configuration with `where` runs on the real agent. That is the safe behavior, because a skipped label would measure another rule.

## Configuration store

The package `internal/store` keeps test configurations and validates them. CTL-03 added it. It serves four operations of the REST contract through the type `store.Handler`, which embeds `api.Unimplemented`, so every other operation still answers 501. CTL-37 added `listConfigs`.

| Operation | Scope | Result |
|---|---|---|
| `GET /v1/configs` | read | 200 with `{items, next_page_token}`, the `ConfigRef` of every stored configuration newest first, or 400 for a bad parameter |
| `POST /v1/configs` | run | 201 with a `ConfigRef` and a `Location` header, or 422 with a problem whose `errors` name each failing field |
| `GET /v1/configs/{id}` | read | 200 with the stored configuration as canonical JSON, or 404 |
| `POST /v1/configs/{id}:validate` | run | 200 with a `ValidationReport`, or 404 |

### Storage

The interface `store.Repository` has the methods `Create`, `Get`, and `List` and the sentinel errors `ErrConfigNotFound` and `ErrConfigExists`. `store.Open` implements it on a SQLite file with the pure Go driver `modernc.org/sqlite`, so the controller still builds without cgo. The store keeps one connection, turns on WAL for files, and migrates the table layout with `PRAGMA user_version`. The path `:memory:` opens a private database for tests. Each record holds the creation sequence `seq`, the id, `metadata.name`, the body, and the creation time in UTC. Schema version 2 (CTL-37) added `seq` as `INTEGER PRIMARY KEY AUTOINCREMENT`, so the number is never reused and survives `VACUUM`, and the migration copies the rows of version 1 in rowid order, which keeps their creation order. The handler takes its clock and its id generator from `store.Options`, and the default ids are `cfg_` with 24 random hex digits.

### Listing

`GET /v1/configs` (`listConfigs`, CTL-37) lists the stored configurations newest first, with `page_size` from 1 to 500 (default 50) and `page_token`, and answers `{items, next_page_token}` where each item is the `ConfigRef` of `createConfig`. It follows the design of `listRuns` (CTL-35). The order is the creation sequence `seq` and not the clock, so it is total even when two configurations share a timestamp. The page token is the opaque sequence of the last item of the page (`base64url("c" + seq)`), and the next page holds the configurations below it. A configuration stored between two requests gets a higher sequence, so it never shifts or repeats an item of a later page. `next_page_token` is absent on the last page, also when the last page is full, because the query reads one row more than the page holds. A `page_size` outside the range or a token that `listConfigs` did not return is a 400 problem. Unlike the run store, the configuration store is a file, so a token stays valid across a restart of the controller. The listing reads the summary columns only and never the bodies.

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

The listing tests store configurations through the handler and validate every `listConfigs` response, and each request that is meant to be valid, against `contracts/openapi/fps-api-v1.yaml` with kin-openapi. They cover the empty list, the order, a walk by pages of two, a page that ends exactly at the oldest configuration, a configuration stored between two pages, the 400 problems, and the read scope. The SQLite tests check the order against a clock that runs backwards, the order after a reopen of the file, and the migration of a version 1 file.

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
4. **Semantic rules.** Function names are unique, a manifest with `functions` and an `extension` lists the `render` hook, and the module name is not reserved. CTL-50 made the reserved names the eight that `tools/contracts/validate.py` rejects: `vars`, `session`, `last`, `true`, and `false` from rule 5 of the template grammar, `profiles` and `dist`, which the grammar reserves as roots, and `all`, which the metric catalog uses as the label value for every application (`reservedModuleNames` in `manifest.go`). A module named `dist` fails with the finding `/name`.
5. **Files.** Every scenario, asset path, extension component, and fixture capture that the manifest names exists as a regular file. Each scenario file has a `scenario` field whose name is an identifier and unique within the module. Each key of `expected_bytes` must name one of these scenarios (CTL-50, `unknownEstimateKeys`), and a key that does not fails with the finding `/expected_bytes/<key>`, so a misspelled key no longer installs and leaves the real scenario without an estimate. The check waits when a scenario file gave no name, because the finding for that file already fails the install, and it does not count the files of `scenarios-pending/`.
6. **Scenarios.** CTL-25 added the contract checks of scenario contract v0.3 (`scenario.go`). Each scenario file that the manifest lists, and each `scenarios-pending/*.yaml` file, validates against `contracts/schemas/scenario.schema.json`. The package embeds a copy of this schema too, `TestEmbeddedScenarioSchemaMatchesContract` fails when the copy differs, and `go generate ./internal/registry` refreshes both copies. A small parser for the template grammar then checks every placeholder in the template fields of the actions (`send`, and `host`, `port`, `bytes`, `min_bytes`, `count`, `ms`, and `set` values, also inside `loop` and `branch` bodies) for the two roots that v0.3 added. A `profiles.<name>` reference must name a profile that the scenario declares, and a bare `profiles` or a reference with more fields fails. A `dist.<name>(...)` call must name `pareto`, `lognormal`, `exponential`, or `uniform` and follow the rules of CON-10: the argument count and keyword names, the required parameters, `float` parameters that also take an `int`, `cap` as an `int`, literal `alpha`, `scale`, `sigma`, `mean`, and `cap` above 0, literal `low` and `high` at least 0, and `low` not above `high`. A syntax error or unbalanced braces fails as well. The result type of a core call matters only as the argument of an outer `dist` call, and CTL-38 completed the table `coreResult` for the functions of scenario contract v0.6: `int_bytes` is `bytes`, and `now` has the type that its literal format fixes (`'http_date'` is text, `'epoch_s'` and `'epoch_ms'` are int), or `any` for a format that is a variable, an unknown literal, or not given. The registry still rejects no core call itself, because the contract validator reports a wrong argument or an unknown format. `TestCoreCallResultTypes` checks each case through an outer `dist.uniform` call. The registry does not check module function calls or the other references, because the contract validator and the compiler own them, and it does not index the pending scenarios. A finding has the path `file:<path>#<JSON Pointer>`, for example `file:scenarios/web_page.yaml#/flows/0/client/1/send`. Tests install the example `web_profile.yaml` listed and pending, and reject an undeclared profile and each distribution rule.
7. **Digest.** The registry computes the module tree digest of plan rule 1 in `contracts/README.md` and records it in the index. A test checks that the example `http1` and `dns` modules get the digests of `contracts/examples/plans/http_dns_smoke.json`.

The tree then moves into place and the index is rewritten through a temporary file and a rename. A module version is immutable, so installing a name and version that the registry holds fails with `ErrModuleExists`. With a trust store configured, an install also verifies the module signature first, as [Signature checks at install and in plans](#signature-checks-at-install-and-in-plans) describes.

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
## Module signing and signed index

CTL-11 added the package `internal/registry/signing`. It signs module archives and the registry index with Ed25519, and it uses only the standard library (`crypto/ed25519`, `crypto/x509`, `archive/tar`). CTL-28 connected it to the install path and the compiler, and [Where the other packages use it](#where-the-other-packages-use-it) lists what the others must still do.

```go
s, _ := signing.GenerateSigner(rand.Reader)                  // key pair; keep s.MarshalPEM() secret
digest, sig, _ := s.SignArchive(archive, signing.Limits{})   // sig.KeyID and sig.Bytes fill ModuleRef
ts := signing.NewTrustStore()
ts.Add(s.PublicKey())                                        // or signing.ParseTrustStore(file)
v := signing.NewVerifier(ts, nil)                            // nil clock means time.Now
_, err := v.VerifyArchive(archive, sig, signing.Limits{})    // errors.Is(err, signing.ErrBadSignature)
```

### Archive signatures

A module is identified by its tree digest, the `sha256:` string of plan rule 1. The signature of a module is an Ed25519 signature over the UTF-8 bytes of that digest string, which is what `ModuleRef.signature` and `ModuleRef.signature_key_id` carry (CON-05). The signature is outside the module tree, so signing changes no digest.

`ArchiveDigest` computes the digest from a tar archive, optionally gzip compressed, in one pass and without writing to disk. It applies the entry rules of registry install: directories and regular files only, relpath characters, no `.` or `..` segment, no duplicates, and the same size and file limits. A test checks that the digest of an archive equals `registry.TreeDigest` of the unpacked tree, so the two copies of the rule cannot drift. The digest covers file names and contents. File modes, times, owners, and empty directories are not signed, which matches the digest that plans carry.

`Signer.SignArchive` returns the digest and the `Signature` (key id and 64 bytes). `Verifier.VerifyArchive` digests the archive, checks the signature, and returns the digest so that the caller can compare it with the digest of the index or the plan. A modified, added, removed, renamed, or truncated file changes the digest, and verification fails with `ErrBadSignature`.

### Key ids

The id of a key is `ed25519:` and the hex of the first 16 bytes of the SHA-256 of the 32 byte public key, so it has 40 characters and needs no registry to assign it. An id cannot name any key other than the one it came from. A signature that carries the id of another trusted key fails with `ErrBadSignature`, and an id that names no key fails with `ErrUnknownKey`.

### Trust store

A `TrustStore` holds the public keys that a verifier accepts. Each key has one of three standings.

| Standing | Verifies | How it gets there |
|---|---|---|
| active | At any time. A store may hold several active keys. | `Add` |
| retired | Strictly before its cutoff, never at or after it (`ErrKeyExpired`) | `Retire(id, cutoff)`, which may be called again to move the cutoff |
| revoked | Never (`ErrKeyRevoked`) | `Revoke(id)`; the standing is final, `Retire` and `Add` refuse the key afterwards |

Signatures carry no time, so the cutoff applies to the time of the verification, which the verifier reads from its injected clock. The store is a JSON file of public keys only (`Encode`, `ParseTrustStore`), meant to be reviewed and kept in version control. The parser refuses unknown fields, a key id that is not derived from its public key, a retired key without an RFC 3339 `retired_after`, a cutoff on another standing, and duplicates.

```json
{
  "version": 1,
  "keys": [
    {"key_id": "ed25519:fe812c12f3ab4ce6ac5db69ac352f906", "public_key": "<base64>", "status": "active"},
    {"key_id": "ed25519:...", "public_key": "<base64>", "status": "retired", "retired_after": "2027-03-01T00:00:00Z"},
    {"key_id": "ed25519:...", "public_key": "<base64>", "status": "revoked"}
  ]
}
```

The verifier checks in a fixed order, so the error names the first thing that is wrong: no signature (`ErrNoSignature`, which the caller may accept in developer mode), a signature that is not 64 bytes (`ErrSignatureLength`), an unknown key id, a revoked key, a retired key past its cutoff, and last the signature itself.

### Signed index

The index lists every module version with its digest, and the signed form lets a client trust the list. `Index` has a format name (`fps-registry-index/1`), a `serial`, and the modules with name, version, and digest. Summaries and scenario names stay out, because a client reads them from the manifest after it verified the digest. The registry's own `index.json` is a different file for local use and stays as it is; the publisher builds an `Index` from `Registry.List`.

The canonical form is the byte string that the signature covers.

- JSON with no whitespace. The object has the keys `format`, `modules`, and `serial`, in that order.
- `modules` is an array of objects with the keys `digest`, `name`, and `version`, in that order, sorted by name and then by version as bytes (so `0.10.0` sorts before `0.2.0`, and `1.0.0` before `1.0.0-rc.1`). A name and version pair appears once.
- Names match the manifest identifier pattern, versions the manifest semver pattern, and digests are `sha256:` and 64 lowercase hex digits. These patterns allow no character that JSON escapes, so the form has no escapes and any language can produce the same bytes. `serial` is an unsigned decimal integer with no leading zero.

```
{"format":"fps-registry-index/1","modules":[{"digest":"sha256:...","name":"dns","version":"0.1.0"}],"serial":3}
```

`SignedIndex` is the published file: `index` (the same fields), `key_id`, and `signature` in base64. `Verifier.VerifyIndex` canonicalizes the parsed index and checks the signature over those bytes, so the layout of the file does not matter and every value does. `ParseSignedIndex` rejects unknown fields and trailing data, so nothing unsigned rides along. Changing a digest, a version, a name, the serial, or the list, or flipping a signature bit fails with `ErrBadSignature`. An archive signature and an index signature never verify as each other, because an archive signature covers text that starts with `sha256:` and an index signature covers text that starts with `{`.

The `serial` guards against replay. A client that remembers the highest serial it verified refuses a lower one, and the publisher increments it with every new index. The package does not store serials.

Golden vectors in `golden_test.go` pin the key id, a digest signature, the canonical index, and its signature for a fixed key, and a second implementation (Python `cryptography`) produced the same signature and key id. The agent can test its verifier against them.

### Key rotation

Rotation never leaves a time when a module cannot verify, because the new key is trusted before it signs and the old key is retired with a cutoff, not removed.

1. **Generate** the new key pair on the publisher (`GenerateSigner`) and keep the private PEM out of the repository and out of the registry.
2. **Trust** the new public key in every trust store (`Add`) and roll the store out to controllers and agents. Until a verifier has the new key, it rejects what the new key signed with `ErrUnknownKey`, so this step comes before signing.
3. **Sign** new modules and each new index with the new key. Both keys are active now.
4. **Re-sign** the modules and the index that the old key signed. A signature is detached, so re-signing changes no digest. Plans that carry the old signature stay valid until the cutoff.
5. **Retire** the old key with a cutoff that leaves time for step 4 and for the agents to update (`Retire`, for example 30 days ahead). Modules signed by the old key keep verifying until the cutoff.
6. **Remove** the old key from the store file after the cutoff, or keep it as retired for the audit trail. A retired key past its cutoff already fails.

If a private key leaks, skip the grace period: `Revoke` the key at once, roll out the store, and re-sign everything with a new key. A revoked key fails at any time, including for signatures made before the leak, so installed modules need re-signing before they can be installed again. An index signed by a revoked key fails too, which is the signal to publish a new index with a higher serial.

Private keys carry the same risk as the CA key of [Mutual TLS](#mutual-tls). Keep them outside the registry root, readable by the publisher only, and use one key per environment.

### Where the other packages use it

- **Install.** CTL-28 did this: [Signature checks at install and in plans](#signature-checks-at-install-and-in-plans) describes the trust policy and the stored signature. The remaining advice is for the signed index. When a signed index is the source of modules, check `VerifyIndex` first, require a serial at least as high as the last one seen, and require that the digest of each downloaded archive equals the `Find` result for its name and version.
- **Compiler.** CTL-28 did this. For each module of a plan, the compiler copies the digest as it did before, and the signature and key id from the stored signature of the install. The fields stay empty for an unsigned module. The compiler signs nothing itself, because the controller holds no private key.
- **Agent (AGT-04).** The agent holds a trust store file and calls `VerifyDigest(ref.digest, Signature{ref.signature_key_id, ref.signature})` for each module after it checks the tree digest, and refuses a failure unless developer mode is on (`--module-trust-store FILE` and `--dev-modules`, done in AGT-04; see [Agent](Agent.md)). The agent reads its trust store file at startup, so a rotation reaches an agent only after the new file is on the agent and the agent restarted. The agent is Rust, so it implements the same checks with an Ed25519 library: verify 64 bytes over the digest text under the key that the id names, with the same retired, revoked, and cutoff rules. The golden vectors are its test input.
- **Publisher tooling.** There is no command line tool yet. Generating keys, signing an archive, and writing the index file need a small command, which is a follow up.

### Tests

`make test-ctl-signing` and `make lint-ctl-signing` run the package alone. The tests cover the original and each modification of an archive and of an index, an unknown, revoked, retired, and wrong key, a truncated and a long signature, the lifecycle of the trust store with its file, the canonical form, and the golden vectors.

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

`PeerCertIdentity` takes the identity from the verified client certificate, first the common name and then the first DNS name. `ClaimedIdentity` trusts the request and serves tests and local development only. `EndSession(id)` ends the open session of one agent with `UNAUTHENTICATED`, marks it offline with the reason `identity revoked`, and forgets its session token. The authority calls it when an operator revokes the identity. A certificate is checked at the handshake only, so the manager also reads its end (`Config.Expiry`, by default `PeerCertExpiry`). `Register` and `Session` refuse a certificate that has expired, and `Sweep` ends the session of an agent whose certificate expired once the agent holds no plan, with the reason `certificate expired`. A running plan never loses its session to an expiry, and the agent dials again with its renewed certificate. Each `Register` returns a new 256 bit session token and revokes the previous one. The first message of a session must be `open` with the latest token, and a newer session ends the older one with `ABORTED`. The manager answers every heartbeat with an acknowledgement, routes each command result to its caller by `command_id`, and drops a counter snapshot whose sequence is not newer than the last one of the same plan, so duplicates after a reconnection never reach the run.

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

### First response frame of a session

CTL-40 made `Manager.Session` send the response headers (`stream.SendHeader(metadata.MD{})`, through `session.sendHeader`) as soon as `open` returns. Go's gRPC server otherwise sends them with the first message, and an idle session has none until a command or a heartbeat arrives. The Rust agent waits for the headers inside tonic's `client.session(...)` before it enters its loop and heartbeats, so on an idle controller its session never carried a heartbeat, and the sweep ended it 5 to 6 s after the open message (IMG-07, [The agent session gate](Image.md#the-agent-session-gate-img-07)). A command can send the first message before `sendHeader` runs, and that message already carries the headers, so `session` remembers whether they went out and `sendHeader` then does nothing. The stream contract does not change: the headers carry no metadata and no message.

`TestIdleSessionSendsHeadersAtOnce` (`internal/agents/headers_test.go`) uses a client that, like tonic, waits for the response headers (`Stream.Header`) before it heartbeats. The client of `internal/fakeagent` does not wait, so no earlier test saw the gap. The test registers an agent with a heartbeat timeout of 3 s, fails when the headers do not arrive within 2 s (and then shows that the sweep ends the agent), and otherwise holds the idle session open for three heartbeat timeouts (nine heartbeats at one interval each, with a sweep after each) on the fake clock.

### Fake agents

The package `internal/fakeagent` gives other controller packages agents that dial a `ControllerService` in tests. `fakeagent.Serve(t, mgr)` serves the manager on an in-memory listener, and `fakeagent.Start` registers an agent and opens its session. A fake agent answers prepare, start, and stop by the agent state machine and sends a heartbeat after each state change. A test drives it with six methods. A fake agent also takes `UpdateRates` (CON-05) in the running state: it validates every entry of the command against the mix of its plan, applies all of them or none, and keeps the session rate that it runs (`Agent.Rates`). With `Options.Traffic` it simulates sessions at that rate, each with `BytesPerSession` bytes, and sends the client side counters in a snapshot per tick, so the bytes that it reports follow the rate. `Agent.TrafficTotals` and `Agent.StopTraffic` serve tests.

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

The package `internal/runs` drives test runs. It moves each run through the run state machine of the specification, reserves the agents of the run, compiles one plan per agent, sends the prepare, start, and stop commands, and contains agent faults. It serves `GET /v1/runs`, `POST /v1/runs`, `GET /v1/runs/{id}`, and `POST /v1/runs/{id}:stop` as the type `runs.API`, which embeds `api.Unimplemented`, so every other run operation still answers 501.

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

`GET /v1/runs` (`listRuns`, CTL-35) lists the runs newest first, with the optional filter `state`, `page_size` from 1 to 500 (default 50), and `page_token`, and answers `{items, next_page_token}`. `Orchestrator.List` orders the runs by a creation sequence that the orchestrator assigns under its lock, not by the clock, so the order is total even when two runs share a timestamp. The page token is the opaque sequence of the last run of the page, and the next page holds the runs below it. A run that starts between two requests gets a higher sequence, so it never shifts or repeats an item of a later page, and `next_page_token` is absent on the last page, also when the last page is full. An unknown `state`, a `page_size` outside the range, or a token that `listRuns` did not return is a 400 problem. The run store is in memory, so a token does not survive a restart of the controller.

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
## Captures

The package `internal/captures` starts packet captures on the agents of a run and serves the pcapng files. CTL-14 added it. `captures.API` embeds `api.Unimplemented` and implements `listCaptures`, `startCapture`, `stopCapture` (CTL-36), and `downloadCapture`, and `wiring.New` builds a `captures.Service` over the agent manager and the orchestrator and hands the API to the combined server.

### Start and state

`POST /v1/runs/{id}/captures` needs a run in the `running` state and answers 409 otherwise. The service chooses one capture id (`cap-` and 16 hex digits) and sends `start_capture` to every working agent of the run in parallel, with the plan id `<run id>/<agent id>` that the compiler gives each plan. The body maps to the command: `port` to `port`, `bpf` to `filter`, and `max_bytes` (default 104857600) to the byte limit of each agent. An agent that refuses stays out of the capture and the service records why. The request fails and keeps nothing when no agent starts: 422 when every agent answered `INVALID_ARGUMENT`, such as an unknown port or a filter that does not compile, 501 when none supports captures, and 409 otherwise. The answer is 202 with the `Capture` schema, a `Location` header, and the state `running`.

The state of a capture lives in memory next to the state of the runs, and the files live below the capture directory. A capture is `running` while an agent that started it has not reported an end, `completed` once every such agent reported one, and `failed` when no agent could report one, for example because every agent went offline after the run ended. `bytes` appears in the completed state and is the sum of the file sizes. `files` (CTL-36, CON-15) lists every agent that was asked, ordered by agent identity, in every state: `agent`, `started` (false for an agent that did not start the capture), `error` (why it did not start, such as an engine_api below 0.2.0), and, once the agent reported its end, `packets`, `bytes`, `dropped`, and `digest`, which come from `captures.AgentFile`. `GET /v1/runs/{id}/captures` lists the captures in the order that they started, and for a run that ended it first sends `stop_capture`, which is idempotent, to the agents that still run a capture, so that the list reports the final state.

`export_tls_keys` of the request goes into `StartCaptureCommand.export_tls_keys` for every agent (CTL-36). An agent of agent protocol 0.2.3 or later puts the TLS key log lines into the file when it is true; the Rust agent embeds them as a pcapng decryption secrets block since AGT-14. An older agent ignores the field and writes the lines whenever it has them, and the Rust agent does so until AGT-14, so a capture of TLS sessions is as sensitive as a private key and the capture directory has mode 0700. The agent keeps whole sessions, so it refuses a snap length, which the service never sets.

### Stop

`POST /v1/runs/{id}/captures/{capture_id}:stop` (scope `run`, no body) answers 200 with the `Capture` after it sent `stop_capture` to every agent that started the capture and has not reported an end (`Service.Stop`). It fetches no file. The state is `completed` when at least one agent delivered an end and `failed` when none did, and the files are then ready for the download. The operation is idempotent: a capture that already ended answers with its state and sends no command, because the service keeps the report of each agent, and the agent answers a repeated stop with its first result. When an agent of a run that still runs cannot answer, for example because it is offline, the capture would be neither running nor ended, so the answer is 409 with the agent in the detail and the stop may be repeated. Once the run ended, such an agent counts as lost, as in the list. The answer is 404 for an unknown run or capture and for a capture that the retention sweep deleted.

### Download

`GET /v1/runs/{id}/captures/{capture_id}` answers with `application/vnd.tcpdump.pcap`, `Content-Disposition: attachment`, and `Content-Length`. A download ends a capture that still runs, as `stopCapture` does: the service sends `stop_capture` to each agent, which answers with the packet count, the size, and the SHA-256 digest of its file. It then fetches the file of each agent that the service does not hold yet and serves the files one after the other, in agent order, which is a valid pcapng file because every file is a complete section. Later downloads read the stored files and ask no agent. `X-FPS-Capture-Agents` lists the agents whose files the body holds and `X-FPS-Capture-Missing` the agents that could not deliver theirs. The answer is 502 when no agent delivered a file, 501 when the reason is that no agent can serve it, and 404 for an unknown run or capture.

### Transfer

The transfer follows CON-13. `agents.Manager.FetchCapture` chooses a single use transfer id, sends `fetch_capture` with a chunk limit of 256 KiB, and waits for the answer. The agent then opens the client streaming call `UploadCapture` on the same connection, and the manager passes the chunks to a `CaptureSink`. `Manager.UploadCapture` in `internal/agents/capture.go` checks the call before it reads a byte:

| Check | Answer |
|---|---|
| The transfer id was not issued, was used, or has ended | `NOT_FOUND` |
| Another call serves the transfer | `ALREADY_EXISTS` |
| The session token of `open` is not the token of the session that carried the command | `UNAUTHENTICATED` |
| The client certificate names another agent than the one that received the command | `NOT_FOUND`, the answer for an unknown id, so an agent learns nothing about the transfers of another |
| The session of the command ended | `ABORTED` |
| `open` names another capture or offset than the command | `INVALID_ARGUMENT` |
| A chunk has a gap or overlap, 0 or more than 262144 bytes, or ends beyond the file size; or `end` disagrees with the bytes received | `INVALID_ARGUMENT` |

A transfer ends with the call, so a broken call is final, and the next attempt has a new id. An agent that does not open the call within the heartbeat timeout after its answer fails the transfer with `ErrTransferTimeout`, and the end of the session fails it with `ErrSessionLost`. A call that arrives before the manager has read the answer waits for it, because the two travel on different streams.

The manager sends `fetch_capture` only to an agent that reports `engine_api` 0.2.2 or later, and `start_capture` and `stop_capture` only to 0.2.0 or later. Below that, and when the agent answers `UNIMPLEMENTED`, the call fails with `agents.ErrCaptureUnsupported`, which the service treats as "this agent cannot serve the file" and not as a protocol fault. The Rust agent serves `fetch_capture` since AGT-09 and filters a capture by `port` since AGT-12 (a plan port name, a server port, a list, or a range), so `POST /v1/runs/{id}/captures` works against the real agent; the controller e2e test starts, stops, downloads, and verifies captures over mutual TLS.

The manager also keeps labeled criteria away from an agent that cannot apply them (CTL-36, [Criterion labels](#criterion-labels)).

### Storage, resume, and verification

The capture directory is `<data_dir>/captures`, or a temporary directory that ends with the process when no data directory is set. A file lives at `<run id>/<capture id>/<agent id>.pcapng`, and a transfer in progress at the same path with the suffix `.part`. A retention sweep deletes old captures, see [Retention](#retention-of-capture-files).

When a transfer breaks, the service reads the size of the part file and sends a new `fetch_capture` with a new transfer id and a `start_offset` equal to that size, up to five times. The agent keeps the file unchanged, so the resumed bytes continue the same file. The sink appends and, at `end`, hashes the whole part file, the bytes of every attempt together, and accepts it only when the size and the digest equal both the end message and what `stop_capture` reported. A file that fails is removed, the agent receives `verified: false`, and the next attempt starts at offset zero. A file that fails every attempt is never served, and the download answers 502 with the mismatch in its detail. A part file that is already complete is verified without a transfer. A file larger than 8 GiB per agent is refused.

### Retention of capture files

Nothing else removes a capture file, so `captures.Service` sweeps the capture directory (CTL-32). The setting `capture_retention` (flag `-capture-retention`, default `168h`, must be positive) says how long a capture lives, and `wiring.New` hands it to the service together with the controller clock. The service owns a `time.Ticker` and sweeps once at start, which cleans up after an earlier process, and then every hour (`Config.SweepInterval`). `Service.Close`, which `Controller.Close` calls before it closes the store, stops the ticker and waits for a sweep in progress. `Retention` zero in `captures.Config` starts no sweep, so a service built without it keeps every file as before.

The age of a capture is the newest modification time of its directory and the files in it, or the time that the service started it, whichever is later, compared with the clock of the service. A capture is deleted when that time lies more than `capture_retention` in the past. The sweep then removes the directory `<run id>/<capture id>` and the capture itself, so a download of it answers 404 like a download of an unknown capture, and the list of the run no longer shows it. A capture that was never downloaded holds no file but expires the same way. The sweep also deletes directories that no capture of the running process owns, which a restart leaves behind because the capture records live in memory, by the age of their files, and it removes a run directory once it is empty and old.

The sweep never deletes what is in use:

- It takes the lock `capture.work` that a stop and a fetch hold for their whole length, with `TryLock`. A capture whose transfer runs is skipped and the next sweep tries again, so the sweep never waits for a transfer.
- `Service.Prepare` counts the download that it returns, and `Download.Close` releases it. The REST handler closes the download after it wrote the body, and the sweep skips a capture with open downloads. Code that calls `Prepare` itself must call `Close`.
- A capture that still runs is never deleted, because an agent still writes into it. A running capture of a run that ended becomes final at the next list or download, and the sweep can delete it after that.

The REST contract has no operation that deletes a run, so there is no deletion of capture files with a run. A later package that adds one can call the same removal.

`Config.Clock` is a one method interface, `Now() time.Time`, so a test moves time by hand. `retention_test.go` shows a file that stays at 6 days 23 hours and is deleted at 7 days 1 hour, then a download that answers 404.

### Tests of the captures

Fake agents from `internal/fakeagent` (`capture.go`) answer the three commands, store a deterministic file per capture, and upload it in chunks, with options to break an attempt after a number of bytes, to corrupt an upload, to answer `UNIMPLEMENTED`, or never to open the call. `internal/captures` tests every operation behind the real router and token check and validate each JSON exchange against the contract: a start, a list, and a download; a download that resumes after two interrupted uploads, each time at the offset that the controller held and with a new transfer id; a digest mismatch that is rejected and never stored, and one that the next attempt repairs; an agent that cannot serve its file; an agent of engine api 0.2.1 that never receives `fetch_capture`; two agents in one download; the errors of the start; and a lost agent. `stop_test.go` (CTL-36) covers `stopCapture` and `Capture.files` against fake agents and the contract: a stop that ends two agents, listed in the order of their identities, without a fetch or an upload, with the packets, bytes, dropped count, and SHA-256 digest of each file, and `export_tls_keys` in the command; a repeated stop that sends no command; the file of an agent that did not start with its error; the 404 answers; and an agent that is gone, which gives 409 while the run runs and a failed capture after it ended. In `TestDownloadResumesAfterAnInterruptedUpload` the fake agent breaks an upload by half closing the call, which keeps the order of the stream, so the controller has read every chunk that the agent sent before it sees the break. The earlier version cancelled the call after a 50 ms sleep, and on a loaded machine the cancel could overtake chunks that the controller had not read, so the resume offset was lower than the bytes that the fake agent counted and the test failed (CTL-36). `internal/agents/capture_test.go` covers the checks of the upload call in the table above.

## Capabilities endpoint

`controller/internal/api/capabilities.go` serves `GET /v1/capabilities` with the read scope. The type `CapabilitiesAPI` embeds `api.Unimplemented`, so it answers every other operation with the 501 problem, and the binary composes it with the other handlers. The package imports neither `internal/registry` nor `internal/agents`, because `internal/agents` already imports `internal/api`. It reads two narrow interfaces that it defines itself, and the binary adapts the registry and the agent manager to them when it wires the controller, which follow up #102 tracks.

| Interface | Method | Adapter in the binary |
|---|---|---|
| `ModuleLister` | `ModuleSummaries(ctx)` returns `[]api.ModuleSummary` | `Registry.List` with the summary of each entry |
| `AgentInventory` | `AgentCapabilities(ctx)` returns `[]api.AgentCapability` | `Manager.List` with `agents.ToAPI` and every backend of the latest registration |

`controller_version` (REST 0.7.0, CTL-37) is the version of the controller build, the same string that `fps-controller -version` prints, and it is not `api_version`. `NewCapabilitiesAPI` takes it through the option `api.WithControllerVersion`, `wiring.Options.Version` carries it, and `serve` in `cmd/fps-controller/main.go` passes `buildVersion()`: the value of `-ldflags "-X main.version=..."`, else the module version of the Go build information, else `(devel)`. An empty version leaves the field out, as the contract says for a build without one. The end to end test of the binary builds it with a stamp and checks that `GET /v1/capabilities` reports it.

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

Both metrics endpoints take an optional `side` query parameter (CON-12, API 0.4.0): `client`, the default, or `server`; any other value is a 400. `GET /v1/runs/{id}/metrics` returns the points between `from` and `to`, both inclusive, and `step_s` keeps the newest point of every step. A run that the orchestrator knows but that sent no snapshot yet answers with an empty sample list. The responses use the type `metrics.Sample` with float64 values in place of the generated float32, because a float32 holds integers exactly only up to 2^24, and the tests validate every response against the OpenAPI document.

### Stream

`GET /v1/runs/{id}/metrics:stream` sends Server Sent Events once per `Config.StreamInterval`, which defaults to one second. Each tick sends one event per current sample of every requested view, or of all six views when the request names none, with the sample as JSON in `data` and the tick number in `id`. When the run ends, the stream sends the final samples and an event named `end` whose data holds the run state, and closes.

### Prometheus

A catalog name maps to `fps_` followed by the name with dots replaced by underscores, and counters add `_total`, as `contracts/README.md` describes. Every series carries the label `run_id` and its catalog labels. A histogram becomes a summary with the quantiles 0.5, 0.9, 0.99, and 0.999, a `_count`, and a `_sum` that equals the HDR mean times the count. The handler writes the text format 0.0.4 with the standard library, so the controller module gained no dependency, and `testdata/prometheus.txt` holds its golden output. The handler serves every run that the aggregator holds, up to `Config.MaxRuns`, and `Aggregator.Forget` drops a run.

### Tests of the aggregator

`TestClientSideIsTheDefault` in `side_test.go` sends the same sessions and handshakes from both sides and checks that the application, scenario, and tcp views and `Latest` report the client side by default and the server side when selected. `TestCountersEqualSumOfFakeAgentSnapshots` sends six rounds of cumulative snapshots from three fake agents through the real agent manager, one agent with two workers, adds a stale replay and an empty final snapshot, and checks that every merged counter equals the sum of the newest snapshots. `TestMergeMatchesReference` and `TestHistogramsOfFakeAgentsMatchReference` compare the merged histograms bucket by bucket and statistic by statistic with `testdata/reference_merge.json`, which `Snapshot::merge_all` of `engine/fps-stats` wrote. The generator lives in `testdata/refgen` and reruns with the command in its `Cargo.toml`. Further tests cover derived ratios, gauge means, views, windows and steps, limits, malformed and oversized encodings, a fuzz target of the decoder, the stream, and the Prometheus golden file.
## Binary wiring

CTL-17 wired every package into `fps-controller`, so the binary serves the whole `/v1` REST API and the agent gRPC service. The package `internal/wiring` builds the components, adapts them to one another, and combines the REST handlers, and `cmd/fps-controller` runs the REST server and the gRPC service side by side.

### Settings of the binary

The configuration file gained six keys, and the binary gained two flags. Every earlier key and flag keeps its meaning. CTL-27 added the keys of mutual TLS, which [Mutual TLS in the controller](#mutual-tls-in-the-controller) describes.

```yaml
data_dir: /var/lib/fps          # module registry in data_dir/modules, store in data_dir/configs.db
store_path: /var/lib/fps/db/configs.db   # optional, overrides the store file, or ":memory:"
grpc_listen: localhost:8444     # host:port of the agent gRPC service, mutual TLS (CTL-27)
agent_auto_enroll: false        # admit a certificate holder that the inventory does not list
tokens:
  - { name: ci, scope: run, token_file: /etc/fps/ci.token }
  - { name: ops, scope: admin, token: change-me-0123456789 }
```

| Key or flag | Default | Meaning |
|---|---|---|
| `data_dir`, `-data-dir dir` | none | Directory of the module registry and the configuration store. Without it the store lives in memory and the registry in a temporary directory, so every configuration and module ends with the process, and the controller logs a warning |
| `store_path` | `data_dir/configs.db`, or `:memory:` without `data_dir` | SQLite file of the configuration store. The controller creates its directory |
| `grpc_listen`, `-grpc-listen address` | `localhost:8444` | TCP address of `ControllerService`, which agents dial. It requires mutual TLS unless `insecure_plain_grpc` is set |
| `agent_auto_enroll` | false | Admits an agent whose identity the inventory does not list. With mutual TLS the identity comes from the client certificate, see [agent_auto_enroll](#agent_auto_enroll-with-certificates) |
| `tokens` | none | Bearer tokens of the REST API, each with a `name`, a `scope` of `read`, `run`, or `admin`, and exactly one of `token` and `token_file` |
| `trust_store_file` | none | Trust store of module signing keys. With it the registry refuses a module without a verified signature, see [Trust policy](#trust-policy) (CTL-28). Without it signatures are not required and the controller logs a warning |
| `capture_retention`, `-capture-retention duration` | `168h` | How long the controller keeps a stored capture, as a Go duration (`7d` is not one, write `168h`). It must be positive. See [Retention](#retention-of-capture-files) (CTL-32) |
| `allow_unsigned_modules` | false | Development setting that lets a registry with a trust store install unsigned modules. A bad signature is still refused |

A token holds at least 16 bytes. A `token_file` holds the token alone, and the controller trims the white space around it. Names must be unique, and two entries must not hold the same token. A token that breaks a rule, or a token file that the controller cannot read, ends the process with exit code 2. Without tokens every `/v1` operation answers 401, and the controller logs a warning at startup.

The gRPC service requires mutual TLS since CTL-27, and it reads the identity of an agent from its client certificate. Only the explicit setting `insecure_plain_grpc` serves plain TCP, where the service trusts the `agent_id` of each request, and the controller then logs warnings at start up and for every identity that it accepts. CTL-15 added the package that provides the certificates, and [Mutual TLS in the controller](#mutual-tls-in-the-controller) describes the wiring.

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
| `wiring.Catalog` | `store.ModuleCatalog` and `runs.Catalog` with `Scenarios` | The scenario files of the latest installed version of each module, with the module name, version, tree digest, the `settings` key of the manifest as `SettingsSchema`, which stays empty for a manifest without it, and the manifest `expected_bytes` entry of the scenario as `BytesPerSession`, which stays 0 for a scenario without one (CTL-47). Configurations name a module without a version, so older versions stay out of the catalog |
| `wiring.Catalog` | `api.ModuleLister` with `ModuleSummaries` | Every installed version from `Registry.List` |
| `wiring.Inventory` | `store.AgentInventory` with `AvailableAgents` | Agents that are online or reserved, with cores, the first known backend, and the port names, as the orchestrator compiles them |
| `wiring.Inventory` | `api.AgentInventory` with `AgentCapabilities` | Every agent through `agents.ToAPI`, with every backend of its latest registration |

### Combined server

`wiring.Server` is the single `api.ServerInterface` that `api.New` receives. It embeds the handler of every package, and each of those embeds `api.Unimplemented`. Go promotes the method of the package that implements an operation, because it sits one level above the 501 methods that the other handlers inherit. The operations that no package implements sit in an embedded `pending` type at the same level, so no selector is ambiguous. A package that implements a pending operation deletes it from `pending` and embeds its handler, and the compiler reports an ambiguous selector while both remain.

| Operations | Handler |
|---|---|
| `/v1/agents` and its actions | `agents.API` |
| `GET /v1/capabilities` | `api.CapabilitiesAPI` |
| `GET /v1/configs`, `POST /v1/configs`, `GET /v1/configs/{id}`, and `:validate` | `store.Handler` |
| `/v1/modules` | `registry.API` |
| `GET /v1/runs`, `POST /v1/runs`, `GET /v1/runs/{id}`, `POST /v1/runs/{id}:stop` | `runs.API` |
| `GET /v1/runs/{id}/metrics` and `:stream` | The `MetricsHandler` of `Controller.SetMetrics`, or 501 |
| `getRunReport` and `attachRealismReport` | The `ReportHandler` of `Controller.SetReport`, which defaults to `report.API` (CTL-41 added the second operation) |
| `listCaptures`, `startCapture`, `stopCapture`, `downloadCapture` | The `CaptureHandler` of `Handlers.Captures`, which `New` sets to `captures.API` (CTL-14) |

### Hook for run metrics

The metrics package of CTL-08 joins the binary in three calls in `serve` of `cmd/fps-controller/main.go`, where a comment marks the place before `Mount`. `ctl.SetMetrics` takes a value with the methods `GetRunMetrics` and `StreamRunMetrics` of `api.ServerInterface`, `ctl.Observe` adds its agent event observer next to the orchestrator, and `a.Handle("GET /metrics", ...)` mounts the Prometheus endpoint outside `/v1`. The exported fields of `wiring.Controller` give the package the store, the registry, the agent manager, and the orchestrator.

### Tests of the wiring

`TestServerRoutesEveryOperation` walks every operation of the OpenAPI contract with an admin token and checks that exactly the two pending operations (`getRunMetrics` and `streamRunMetrics`, since CTL-36 serves `stopCapture` and CTL-37 serves `listConfigs`) answer 501, each with a detail that names it, so every other operation reaches the package that implements it. `TestPendingExamplesValidateThroughTheAPI` stores the three examples of `contracts/examples/tests-pending/` through the REST API, registers fake agents `agent-a` with `eth1` and `eth2` and `local` with `lo`, installs the example modules, and expects every validation to pass the dry run, and `TestCatalogReportsSettingsSchema` expects the `dns` scenarios to carry the settings schema with `zone_file`. `TestVersionReachesCapabilities` starts the controller with `Options.Version` and expects it as `controller_version`, and expects `GET /v1/configs` to answer 200. Other tests in `internal/wiring` cover the token scopes, the metrics hook, the catalog with two versions of `http1`, the inventory filters, and a gRPC round trip with a fake agent and the observer.

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
| `json`, the default | The `RunReport` schema, with the summary filled from the run totals, every criterion in the verdict with its `reason`, `observed` and `passed` only for compared criteria, `reason` of the run when it failed or aborted, `scenarios` from the [scenarios table](#scenarios-table), `where` on each criterion that has a filter, and the realism results that [attachRealismReport](#realism-reports-attached-to-a-run) stored in `realism`, an empty list for a run without a report |
| `csv` | One table with the columns `kind,name,op,threshold,value,result,detail`, with rows of kind `run` for the run fields, `criterion` for each criterion, `scenario` for each row of the [scenarios table](#scenarios-table), and `metric` for each run total in catalog order |
| `html` | The page of `internal/report/html`, served as `text/html; charset=utf-8` with `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'` (see [HTML report](#html-report)). 501 when the `API` has no renderer |

The CSV report writes numbers in plain decimal notation and prefixes text cells that start with `=`, `+`, `-`, or `@` with an apostrophe, so spreadsheets do not run them as formulas.

### Scenarios table

A report built from labeled series (`Evaluator.BuildSeries`) carries `Report.Scenarios`, the scenarios table of CTL-38. It has one `ScenarioCount` per scenario, ordered by `<application>/<scenario>`, the key of the scenario view of the metrics, with `sessions` from `sessions.attempted` and `bytes` from `bytes.tx` plus `bytes.rx`. Like the totals it counts the client side of each metric, because the server side sees the same sessions again, and the counters of the orchestrator take precedence as everywhere in the report. A report built from totals alone has no table. The RLM-07 mix check reads the table in the form `{"scenarios": [{"scenario", "sessions", "bytes"}]}`.

CTL-39 fills `RunReport.scenarios` of the REST report (contract 0.9.0, CON-21) from the same rows. `Report.ScenarioTotals` holds one `api.ScenarioTotals` per scenario, ordered by application and then by scenario name, with `application` and `scenario` apart, `sessions_attempted`, `sessions_succeeded`, `sessions_failed` (the sum over every reason), `bytes_tx`, and `bytes_rx`. A count that the run did not record is 0, and a scenario appears once the run recorded any of the five counters for it. Because both the table and the run summary count the client side, the sum of a property over all entries equals the property of `run.summary`. `Report.API` always sends the property: an empty list for a report built from totals alone, as a controller of 0.9.0 or later does. The file form keeps the table of CTL-38. `Report.FileJSON()` renders the `RunReport` with that table under `scenarios`, which shadows the REST property of the same name because the outer field of an embedding struct wins in `encoding/json`, so the RLM-07 mix check keeps reading `{"scenarios": [{"scenario", "sessions", "bytes"}]}`. The CSV report writes one row of kind `scenario` per scenario, with the scenario in `name`, the sessions in `value`, and `bytes=<n>` in `detail`. Run metrics series of the mix, if a package emits them, carry no `source` label, because a series without it is a run metrics series (catalog 0.4.0, CON-20), and the table adds no mix series.

`Report.API` also copies the `where` filter of each criterion of the verdict into `CriterionResult.where`, exactly as the configuration states it. The controller adds no `side` entry, and a criterion without a filter has no `where`. Two criteria that differ only in their filter keep their own.

`Report.Realism` holds the realism results that the controller stores for the run, as `[]api.RealismResult`, and `Report.API` sends them in `realism` in their order, with an empty list when there are none. `GetRunReport` fills it from the realism store that `PUT /v1/runs/{id}/realism` writes, as [the next section](#realism-reports-attached-to-a-run) describes.

`Report.Declared` holds the `compiler.Declarations` of the test configuration, the targets of the realism check, and `FileJSON` writes them under `declared`. `GetRunReport` reads them through the `DeclarationSource` of `UseDeclarations`. `ConfigObjectives` is one, and `UseObjectives` adopts a source that is also a `DeclarationSource`, so the binary needs no change. A configuration that is gone or unreadable leaves the report without declarations.

### Realism reports attached to a run

`PUT /v1/runs/{id}/realism` (`attachRealismReport`, scope `run`, CTL-41) stores the report that the realism runner wrote for the capture of a run, and `GetRunReport` serves its results in `realism`. `report.API.AttachRealismReport` answers in this order, and the first failing step answers.

| Step | Answer |
|---|---|
| The run does not exist | 404 |
| The run is `pending`, `preparing`, `running`, or `stopping` | 409, with the state in the detail. Only `completed`, `failed`, and `aborted` accept a report |
| The content type is not `application/json` (parameters such as `charset` are fine) | 415 |
| The body is longer than 4 MiB (4194304 bytes), `report.MaxRealismBytes` | 413. A body of exactly 4 MiB is served |
| The body is not one JSON document | 400 |
| The document breaks `realism/schema/realism-report.schema.json`, or two results share `metric` and `labels`, or a value does not fit a 32 bit float | 422, with `errors` that name each failing field by JSON Pointer into the document, sorted by path |
| Otherwise | 200 with `RealismAttachment`: `realism`, the entries the run now holds, and `replaced`, true when the run held a report before |

The controller validates with a byte copy of the report schema in `controller/internal/report/schema/`, which `go generate ./internal/report` refreshes and `TestEmbeddedRealismSchemaMatchesSource` compares with the source, as `internal/store` does for the test configuration schema. It compiles the schema on the first report. It does not compare `capture.sha256` with the capture files of the run, and it stores the document as given.

`report.MapRealism` makes one `RealismResult` for each metric of each check, in the order of the document, by the table of `contracts/changes/CON-23.md`, which replaced the table of CON-22 and keeps its two rows.

| Metric | `target` | `passed` |
|---|---|---|
| `realism.conformance_ratio` | `>= ` and the `threshold` of the check, for example `>= 0.99` | The `passed` of the entry of `applications` with the same `application` |
| `realism.ks_p_value` and `realism.ks_statistic` | `p value >= ` and the `alpha` of the comparison of the same application and property, for example `p value >= 0.05` | True when the `status` of that comparison is `pass`. The property `request_bytes` is the `payload_size` comparison of direction `client`, and `response_bytes` that of direction `server` |
| `realism.check_failures` | `== 0` | The `passed` of the check whose kind is the label `check` |
| `realism.parser_errors`, `realism.structure_failures`, `realism.transport_deviations` | `== 0` | The value is 0 |
| `realism.decrypted_ratio` | `>= ` and `min_decrypted` of the check, for example `>= 0.9` | The value is at least `min_decrypted` |
| `realism.ja4_match_ratio` | `>= 1` | The value is 1 |
| `realism.duplicate_ratio` | `<= ` and `max_duplicate_ratio` of the check | The `passed` of the entry of `applications` with the same `application` |
| `mix.share_pct` | `within ` and `tolerance_points`, ` points of `, and the `configured_pct` of the row, for example `within 2 points of 60` | The `ndpi_passed` of the row of `applications` of the basis `metrics_basis` with the same `application` |
| `mix.abs_error_pp` | `<= ` and `tolerance_points` | The same `ndpi_passed` |
| `mix.max_abs_error_pp` | `<= ` and `tolerance_points` | True when `ndpi_passed` holds in every row of `applications` of the basis `metrics_basis` |
| Any other metric | `check passed` | The `passed` of its check |

A metric takes its row only inside the check kind that CON-23 gives it, and the mix rows only for the series with `source` `classifier`. The last row is generic: a metric that the table does not name, a series with `source` `metrics`, a check whose `metrics_basis` is null, and a metric that finds no entry to take its outcome from (a conformance, content, or mix label without a row, a KS metric without a comparison) get `check passed` and the outcome of their check, which a document that conforms to the schema never causes for the metrics of the table. When a document holds two checks of one kind, `realism.check_failures` passes only if both passed. Numbers in a `target` use the shortest decimal form that reads back to the same value. `property` is wording for an operator, and the wording depends on the metric and its labels only, so a metric gets it whichever row gives its target (CTL-48, after CTL-46 did it for the KS metrics). It follows the examples of CON-23: `conformance of http1 by ndpi`, `KS p value of inter_arrival of http1` (and `KS statistic of ...`, with the value of the label `property` unchanged), `failures of the temporal check`, `parser errors of http1 by tshark` (`parser errors of all applications by tshark` for the application `all`), `decrypted TLS sessions`, `JA4 matched TLS sessions`, `duplicate payloads of http1`, `structure failures of web_browse`, `ttl deviations of client SYN packets` (`server SYN-ACK packets` for the side `server`), `mix share of http1 by ndpi`, `mix error of http1 by ndpi`, and `largest mix error by ndpi`. A metric that the examples do not name, and a series that lacks a label its wording needs (a mix series without `classifier`, such as one with `source` `metrics`), get its name followed by its labels in key order. `TestMapRealismCON23Examples` compares `property` for every case of `check-metrics.yaml`. Every check kind now carries `realism.check_failures`, so a failing `parser_errors` check shows in `realism` through it (the open question of CON-22, which CON-23 settled).

A second valid report replaces the first as a whole, and a refused report leaves the earlier one. The reports live in a `report.RealismStore`, by default a `MemoryRealism`, because the orchestrator keeps its runs in memory too. `API.UseRealism` replaces it, and `MemoryRealism.Document` returns the document as sent, for a later read operation. The wiring routes the operation to the handler of `Controller.SetReport`, which `ReportHandler` now requires to serve both operations, so the report that was attached is the report that is read. The operation left the pending list of `wiring_test.go`, whose entries are now `getRunMetrics` and `streamRunMetrics`.

`realism_test.go` runs the operation through the generated router and checks every response against the contract. `TestMapRealismFixture` and `TestAttachRealismReportOfEndedRuns` hold the literal results of a conformance and a temporal check (`testdata/realism-report.json`, eight entries, two of them failing for each check). Other tests cover the replacement, the refusals that keep the earlier report, the four states of a run that has not ended, 404, 415, 400, the size limit at 4 MiB and one byte above, the findings of the schema with their JSON Pointers, the scope `run`, and the generic rule. `realism_con23_test.go` maps every case of `contracts/examples/realism/check-metrics.yaml` (17 cases, one passing and one failing for each of the eight check kinds, and a second failing case for `parser_errors`) to the literal `target` and `passed` of the example, and tests the cases where the generic rule stays.

### Tests of the report

`TestGoldenReports` serves a passing run, a run with failing criteria, and a degraded run through the generated router, validates every response against the contract, and compares the JSON and CSV bodies with the golden files in `controller/internal/report/testdata/`. In the degraded run one agent crashed with open sessions, and the report counts them as failed. `TestGoldenLabeledReport` adds a failed run with a reason over a real aggregator that holds two applications on both sides. Its golden files `labeled.json` and `labeled.csv` cover filtered criteria (`application`, `side`), the client side default, a histogram criterion on each side, an invalid label, and a filter without a series. `TestClientSideIsTheDefault` checks the same numbers for a criterion without a side and one with `side=server`, and shows that the totals of the aggregator count each session twice. `TestRunEvaluatorDecidesHistogramsByRunID` checks that the run evaluator reads the aggregator by run id and gives the same results as the report. Other tests cover every operator, every catalog formula, the four results, the error responses, and a report built from the real aggregator of CTL-08 with two agents. `TestGoldenLabeledReport` also carries the two `scenario` rows of the table in `labeled.csv`, and since CTL-39 `labeled.json` holds the literal `scenarios` (dns/query 1000 attempted, 990 succeeded, 10 failed, 1 000 000 bytes sent and 2 000 000 received, and http1/web_get 3000 attempted, 3000 succeeded, none failed, 5 000 000 sent and 30 000 000 received) and the `where` of seven criteria, while the three other golden files end with an empty `scenarios` list. `scenarios_test.go` (CTL-38 and CTL-39) checks the table against the same literal counts over the two sided aggregator, that the entries of the REST report add up to its summary, the precedence of the orchestrator counters, a report without a table, the file report in the table form of the RLM-07 check beside the REST report, `where` on criteria with and without a filter, the stored realism results, and the declaration source over a stored configuration.
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

## HTML report

CTL-13 adds `controller/internal/report/html`, which renders the report of CTL-09 as one self contained HTML page. It uses only the standard library, `html/template` and `embed`.

### Page

The page holds, in this order, a header with the verdict and the state, the reason of the run, the summary (run id, configuration, state, duration, objective, times, seed, engine and module versions, agents, and the run totals), the criteria table, the realism table, and the charts. The criteria table shows each criterion with its rule, its label filter, its observed value, its result as the word `PASS`, `FAIL`, `MISSING`, or `INVALID`, and its reason, so the result never depends on color alone. A criterion that compared no value shows a dash. Every string of the report passes through `html/template`, so markup in a criterion reason, a module name, or an agent id shows as text.

The charts are inline SVG that `chart.go` draws: one line chart per per second series with grid lines, value ticks in three significant digits (`610M`), a time axis in seconds from the first sample, and a line of minimum, maximum, last value, and sample count under each chart. A series that no sample holds gets no chart. The page draws the whole run values `sessions.concurrent`, `sessions.rate`, `throughput.bps`, `sessions.failed_ratio`, and `sessions.response_time.p99` from the samples with the key `*`. The style sheet is inline, and the colors are variables that `prefers-color-scheme: dark` redefines, so the page follows the system theme. A print style turns the page light, lets tables break across pages, and keeps a chart on one page. The page has no script, font, image, or link to another host.

### Interface

`html.FromReport(report.Report, html.Options)` builds a `html.Page`, a plain model of strings and numbers, and `html.Render(Page)` returns the document. `Options` carries what the report model does not hold: the `Objective` text (the run record does not keep the load objective, so the caller reads it from the test configuration and may use `html.FormatObjective`), the `Samples` of `metrics.Aggregator.Samples(id, api.MetricViewApplication, metrics.Window{})`, and the `Realism` rows. `html.Renderer` fills the rows from `Report.Realism`. `html.ContentType` is the media type of the page.

### Serving

CTL-26 serves the page. `GET /v1/runs/{id}/report?format=html` builds the report exactly as for JSON, then adds what the report model lacks and renders the page. `report` cannot import `report/html`, which imports `report`, so `report.API` holds a function, `report.HTMLRenderer`, and `html.Renderer` is that function. It calls `FromReport` and `Render`.

| Part | Source |
|---|---|
| Samples | `report.SampleSource`, which `*metrics.Aggregator` satisfies. `NewAPI` sets it when its totals source is one, and `UseSamples` sets it otherwise. The handler asks for `Samples(id, api.MetricViewApplication, metrics.Window{})`. A run that the source does not know gets a page without charts, and any other error answers 500 |
| Objective | `report.ObjectiveSource`, set with `UseObjectives`. `report.ConfigObjectives{Store}` parses the stored configuration of `run.ConfigID` and returns its `load.objective`. A configuration that is gone or unreadable gives a page without objective |
| Renderer | `UseHTML(html.Renderer)`. Without it the format answers 501 as before |

The response has the status 200, `Content-Type: text/html; charset=utf-8`, `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'` (`report.HTMLContentSecurityPolicy`), `Cache-Control: no-store`, and `X-Content-Type-Options: nosniff`. A run that does not exist answers the same 404 problem as the JSON format, because the handler looks the run up before it renders anything.

In the binary, `wiring.Controller` sets the renderer and the objective source over its configuration store on the `report.API` that `New` builds and on the one that `SetReport` receives, so `mountMetrics` needs no change and the aggregator is the sample source.

### Tests

`TestServesHTMLReportOfFinishedRun` fetches the HTML report of a finished run through the real router and token check, checks the four headers, the objective, the verdict, and a chart, and runs the `violations` checker of `TestNoExternalRequests` on the served body. `TestHTMLReportOfUnknownRunIsTheJSONNotFound` compares the 404 of the html format with the 404 of JSON. Further tests cover a run without samples, a run without configuration, a failing sample source, and the 501 without a renderer. The tests live in `controller/internal/report/html/serve_test.go` because they reuse the unexported checker.

`TestGoldenPages` renders the report goldens `pass`, `fail`, `degraded`, and `labeled` of `controller/internal/report/testdata/` and compares the pages with `controller/internal/report/html/testdata/<name>.html`. The goldens carry a deterministic ramp as the per second series, because the JSON goldens hold no samples. `go test ./internal/report/html/ -update` rewrites them. `TestNoExternalRequests` parses each page and fails on any `src`, `href`, `srcset`, `action`, or similar attribute with an `http`, `https`, or protocol relative URL, on any `<script>` element, on a `<link rel=stylesheet>`, on `<img>`, `<iframe>`, and similar elements, and on `@import` or `url()` in the style, and `TestViolationsFindsExternalReferences` shows that the check catches each case. `TestEscapesReportStrings` puts markup in a criterion reason, and `TestEscapesEveryField` and `TestNoExternalRequestsWithHostileStrings` put a script, an image, and a link in every text field of the page. `TestFromReport`, `TestFromReportVerdicts`, and `TestChartsFromSamples` cover the conversion, and further tests cover the chart geometry, the number format, and an empty page. The targets `make lint-ctl-report-html test-ctl-report-html` run the package alone, and `lint-ctl` and `test-ctl` cover it too.

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

The scenario catalog reads `expected_bytes` from the module manifests (CTL-47). QA-05 found that `internal/wiring/adapters.go` never set `compiler.Scenario.BytesPerSession` and that `registry.Manifest` had no such field, so the `bytes` basis and the `throughput_bps` objective failed with `ErrNoByteEstimate` on every real controller. `registry.Manifest.ExpectedBytes` now holds the field (scenario name to bytes, from the manifest schema with a minimum of 1), and `Catalog.Scenarios` sets `BytesPerSession` of each scenario from the entry with the scenario name. A scenario without an entry keeps the value 0, and the compiler fails with `ErrNoByteEstimate` where the test needs an estimate, which is the case for the `bytes` basis and for the `throughput_bps` objective. The `sessions` basis with a session or concurrency objective needs none. The catalog does not check that every key of `expected_bytes` names a scenario of the module, because the registry refuses such a key at install (CTL-50), so a misspelled key cannot reach the catalog from an installed module. `TestCatalogSetsBytesPerSessionFromManifests`, `TestCatalogCompilesBytesBasisAndThroughputFromManifests`, and `TestCatalogWithoutEstimateFailsWithErrNoByteEstimate` in `internal/wiring/bytes_test.go` check the catalog values against the example manifests, compile the enterprise example (basis `bytes`, 10 Gbit/s) and a `sessions` basis variant from them, and compile the example with a copy of `dns` whose manifest lacks the key. Module authors measure and declare the estimate as [Module Development](Module-Development.md) describes.

The package `internal/runs/calibrate/` (CTL-21) applies decision D12 item 10. The compiler turns a `throughput_bps` objective into session rates with the `expected_bytes` estimate of the module manifest, so a session rate is `objective / (8 * mean bytes per session)` (see `splitRates` in `internal/compiler/`). When the estimate is wrong, the run misses the objective by the same ratio. The calibration measures the real bytes per session during the run and returns a `Factor` that scales every compiled session rate. The package uses only the standard library, reads no clock, and uses no random source. CTL-24 wired it into the run loop, which [Throughput calibration in the run loop](#throughput-calibration-in-the-run-loop) describes.

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

### What the orchestrator does

[Throughput calibration in the run loop](#throughput-calibration-in-the-run-loop) describes how the orchestrator calls the package. Concurrency objectives and `sessions_per_second` objectives need no calibration.

### Tests

`calibrate_test.go` simulates the closed loop with noisy sessions (a standard deviation of 40 percent per session and 5 percent per window) for estimates of 0.25, 0.5, 1, 2, and 4 times the true bytes per session, over 40 seeds each. Every run stays within 5 percent of the objective from update 25 on, every step stays within the bound, settled steps stay within 5 percent, and direction reversals stay few. A noise free run moves monotonically, a step test checks the exact 1.5x sequence, and further tests cover mixes of entries with unnormalized weights, the factor bound, the dead band, the minimum session count, immutability of the state, and every validation error.

## Throughput calibration in the run loop

CTL-24 wired `calibrate` into `internal/runs/` (`calibration.go`). A run whose objective is `throughput_bps` corrects its session rates while it runs. Runs with any other objective never start the loop.

### Flow

| Step | What happens |
|---|---|
| Create | `Orchestrator.Create` keeps the scenario catalog that it passed to the compiler. For a `throughput_bps` plan it builds `calibrate.State` with the objective (the sum of `load.objective_value` over the plans), each mix entry of the plan with its normalized weight, and the `BytesPerSession` of its scenario, which is the manifest `expected_bytes` that the compiler used |
| Running | After every agent started, the loop ticks once per window. It stops before the run leaves `running`, because an agent accepts `UpdateRates` only while it runs |
| Each window | The loop sums the newest client side counters of every agent. A mix entry has no label of its own in the metrics catalog, so the entry maps to the `scenario` label, and the `side` label must be `client`, so each payload byte counts once. Bytes are `bytes.tx` plus `bytes.rx`, and sessions are `sessions.succeeded`. The sample of the window is the difference to the last counted window. Two mix entries that run one scenario share its counters, and the window splits by their weights, which keeps the bytes per session. A scenario without a completed session leaves its bytes for the next window that has one, and a counter that fell, as after an agent restart, starts the difference again |
| Update | `State.Update` returns the factor. A change is a `CalibrationStep` in `Run.Calibration` (time, factor, unbounded target, implied total session rate, bytes per session of each entry, at most 256 steps) and one log line `throughput calibration step` |
| Send | Every running agent whose applied factor differs from the current one gets `UpdateRates`. The command holds, for each mix entry, the whole agent total, which is the sum of the compiled `sessions_per_second` of its workers times the factor. The agent divides it among its workers, so the compiled split between workers stays. An agent without a client rate (a `--side server` agent) gets nothing |

An agent that fails to apply the command is tried again in the next window, and the log reports the failure once per factor, so a lost command costs one window. Agents apply the whole command or nothing.

### Agent versions

The orchestrator reads `engine_api` from the capabilities of the agent when it sends. An agent below 0.2.0 never receives the command, and the log line `agent has no UpdateRates, so its rates stay uncalibrated` names it once. The run still starts and ends as usual, and that agent keeps its compiled rates, so the total throughput of a mixed fleet is off by the share of the old agents.

### Configuration

| Field | Meaning |
|---|---|
| `runs.Config.Calibration.Window` | Length of a window in real time. Zero means 5 s (`DefaultCalibrationWindow`), and a negative value turns the calibration off. Decision D12 asks for 5 to 10 s |
| `runs.Config.Calibration.Tuning` | `MaxStep`, `DeadBand`, `MinSessions`, `Forget`, and `MaxFactor` of `calibrate.Config`. Zero fields take the defaults of the package |
| `runs.RateUpdater` | The optional interface `UpdateRates(ctx, agentID, *agentpb.UpdateRatesCommand)` that `Config.Agents` must also implement. Without it, the log says `throughput calibration is off: the agent manager cannot send UpdateRates` and the run keeps its compiled rates |

`agents.Manager.UpdateRates` (CTL-30, `internal/agents/updaterates.go`) sends `SessionResponse_UpdateRates` through `command`, like `Start` does, and returns the `UpdateRatesResult`. The manager therefore implements `runs.RateUpdater`, and `internal/wiring` passes it to the orchestrator as `Config.Agents`, so the calibration runs on a real controller. A refusal of the agent comes back as a `*agents.CommandError`. `Run.Calibration` is not part of the REST API, because the OpenAPI document of the run has no field for it.

### Tests

`calibration_test.go` is the end to end test. The orchestrator runs on the real `agents.Manager`, and fake agents with `fakeagent.Options.Traffic` run the compiled rate in simulated time at 100 times the real speed, report the counters of sessions that carry 40 kB each against a manifest estimate of 10 kB, and take `UpdateRates` into their own rate, so the loop between orchestrator, manager, and agent closes. It checks that an 8 Mbit/s objective, which the compiled rate overshoots fourfold, ends within 5 percent after the factor falls to 0.25 in steps of at most 1.5x, that the log and `Run.Calibration` agree, and that no command arrives after the run ended. Further tests check that an agent with engine 0.1.0 gets no command and is logged once, that a `sessions_per_second` objective and a switched off calibration send nothing, and that a manager without `RateUpdater` leaves the rates alone. `calibration_internal_test.go` covers the window arithmetic, the command builder, and the version check.  `internal/agents/updaterates_test.go` checks the manager method against the fake agent, including the all or nothing rule and the running state requirement. `make lint-ctl-runs test-ctl-runs` runs the package alone.

## Byte weighted mix control

CTL-12 keeps the byte share of every mix entry at its weight when the test sets `profile.basis: bytes`. The test configuration schema already has this setting (`sessions` or `bytes`), so the feature needs no contract change and no extra setting: the configuration asks for it, and a `sessions` basis is untouched.

Since CTL-34 the compiler (CTL-05) turns the byte weights of a `bytes` basis into session shares with the manifest estimates (plan rule 1), so a run starts at the byte shares that the estimates predict. The estimates are only manifest guesses, and a scenario whose real sessions are heavier than its estimate still takes more than its share of the bytes. The package `internal/mixcontrol/` measures the bytes per session of each entry and returns one multiplier per entry. The compiled session rate of the entry times its multiplier gives the entry its byte share.

### Composition with the throughput calibration

A run has exactly one loop that sends `UpdateRates`, the CTL-24 calibration loop in `internal/runs/`. Two loops would overwrite each other's totals. `mixcontrol` therefore sends nothing and reads no clock. The calibration loop feeds the samples of each window to both estimators and sends, for every mix entry, `compiled rate * factor * multiplier` as the whole agent total, in one command per agent and window.

| Piece | Job | Why it composes |
|---|---|---|
| `calibrate` factor `f` | Scales the total to the `throughput_bps` objective | The factor rests on the weighted mean bytes per session of the compiled session weights |
| `mixcontrol` multiplier `m_i` | Moves bytes between entries | Multipliers keep that weighted mean unchanged (`PreserveBytes`): `sum w_i m_i e_i = sum w_i e_i`, so `f` stays right |

The target multiplier is `m_i = (b_i / (w_i e_i)) * sum_j(w_j e_j)`, with `b` the normalized byte weights (the compiled session weight times the estimate, normalized, which `newByteMix` rebuilds), `w` the compiled session weights, and `e` the bytes per session estimates. The entry then carries the byte share `w_i m_i e_i / sum_j(w_j m_j e_j) = b_i`. A `PreserveSessions` mode keeps the total session rate instead, for an objective in sessions per second. The run loop does not use it yet.

### Estimate and control law

| Part | Value |
|---|---|
| Estimate | Pooled byte and session sums per entry, decayed by 0.95 per window. An entry with 20 pooled sessions replaces its manifest estimate `expected_bytes` with the measured bytes per session. Before that, the estimate shapes the mix |
| Window | One second, as the catalog asks. When `Calibration.Window` is zero, a byte weighted run uses `mixcontrol.DefaultWindow` (1 s) for the whole loop and not the 5 s of the calibration alone. The calibration tolerates this: its estimate is a property of the traffic, and its factor moves by at most 1.5 times per window. A configured window wins |
| Step | Each multiplier moves toward its target by at most a factor of two per window, so an entry with a distant target (a scenario with tiny sessions) does not hold back the entries that carry the bytes |
| Dead band | Multipliers hold while every target stays within 1.5 percent, which costs under one point of byte share and keeps the loop from sending a command every window for noise |

Like the calibration, the control never reacts to the achieved byte shares. It estimates a property of the traffic, so it has no feedback delay and no oscillation with the ramp of the agents. `State.Update(now, samples)` takes the time for its `SettledSince` field, and the package has no other time or random input.

### In the run loop

`internal/runs/bytemix.go` attaches the control. `Create` calls `attachByteMix` after `newCalibrator`. The control starts when `profile.basis` is `bytes`, the objective is `throughput_bps`, the calibration is on, and `CalibrationConfig.ByteMixOff` is false. For a `bytes` basis with another objective the log says `byte weighted mix control is off: it needs a throughput_bps objective and the calibration loop, so the mix keeps its compiled session weights` once, and the run keeps its compiled rates. Each change of a multiplier is a `byte mix step` log line and a `CalibrationStep` in `Run.Calibration`, which now carries the `Multipliers` map. `CalibrationConfig.MixTuning` sets `MaxStep`, `DeadBand`, `MinSessions`, and `Forget`.

`internal/fakeagent` got `Traffic.BytesByScenario` and `Agent.ScenarioBytes()` for this package, so one fake agent can run scenarios with very different bytes per session.

### Tests

`mixcontrol_test.go` simulates the closed loop without a clock: noisy sessions (40 percent per session), estimates that are off by factors of four in both directions, 20 seeds each. The byte shares stay within 2 points from window 30 on, for 300 windows. Further tests cover both preserve modes, the dead band, few moves when settled, the manifest estimate before measurements, immutability, determinism, and every validation error. Coverage is 100 percent.

`bytemix_test.go` is the acceptance test on the real agent manager and a fake agent. `TestByteMixExactEstimatesStartOnTarget` (CTL-34) gives the scenarios exact estimates and requires every byte share to be within 2 points of its weight from the first window of 2 simulated seconds on, every rate command (if any) to keep the compiled ratio, and the throughput to stay within 5 percent. `TestByteMixConvergesAndStays` keeps the wrong estimates case below. The mix has the byte weights 60, 39.9, and 0.1 over scenarios whose sessions carry 20 kB, 200 kB, and 200 B, against manifest estimates of 40 kB, 50 kB, and 1 kB, at 80 Mbit/s. At the compiled session shares the heavy scenario takes 84 percent of the bytes. The test runs 120 simulated seconds (the fake agent runs 20 simulated seconds per real second, and a window of 50 ms is one simulated second) and requires every share to stay within 2 points of its weight from 30 simulated seconds on, the throughput to stay within 5 percent of the objective, and every command to carry all entries. Two more tests check that a `sessions` basis and `ByteMixOff` leave the compiled session ratios alone, and that a `bytes` basis with a `sessions_per_second` objective logs and sends nothing. `make lint-ctl-mixcontrol test-ctl-mixcontrol test-ctl-bytemix` runs them.

### Limits

- Only `throughput_bps` objectives are controlled in the run loop. A `bytes` basis with `sessions_per_second` or concurrency objectives needs a loop without the throughput factor (the `PreserveSessions` mode exists), and concurrency objectives also need `concurrent_sessions` targets in `UpdateRates`.
- The mix entry is matched to the `scenario` label of the counters, as in CTL-24.
- The bytes of failed sessions count in the byte counters but not in the completed sessions, so a high failure ratio biases the estimate, as in CTL-24.

## Mutual TLS

CTL-15 added the package `internal/pki`. It gives the control channel between the controller and its agents mutual TLS, and it uses only the standard library (`crypto/x509`, `crypto/tls`, `crypto/ecdsa`). CTL-27 wired the package into the controller, which [Mutual TLS in the controller](#mutual-tls-in-the-controller) describes. The Rust agent speaks mutual TLS and enrolls itself since AGT-08 (see [Agent](Agent.md)); [What the agent must change](#what-the-agent-must-change) is the list it implemented.

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

The controller server certificate comes from the same CA. The authority creates it in memory for the host names and addresses that agents dial, which become its subject alternative names, lets it live for 30 days, and replaces it when a third of its life remains. Both listeners send the chain `[leaf, CA]` in the handshake (CTL-31), so an agent that pins the SHA-256 fingerprint of the CA finds the CA in the presented chain and can authenticate the enrollment endpoint before it sends its token. An agent that trusts a CA file verifies the same chain as before, because the extra certificate is the root that it already trusts.

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

### Mutual TLS in the controller

CTL-27 did the wiring of the controller. The owner chose the HTTPS enrollment endpoint, so the contracts did not change.

`wiring.New` opens the authority from `pki_dir`, which defaults to `data_dir/pki`, and builds the two server configurations for the names of the controller certificate. Without `pki_dir` and `data_dir` the CA lives in memory and ends with the process, which the controller logs as a warning. `Controller.Run` serves `ControllerService` with `grpc.Creds(credentials.NewTLS(...))` from `ServerTLSConfig`, and `agents.Config.Identity` stays at its default, `PeerCertIdentity`, so the identity of an agent is the common name of its verified certificate and `Register` answers `UNAUTHENTICATED` when it differs from `agent_id`. `Controller.RunPKI` serves `pa.Handler()` behind `EnrollTLSConfig` on the second listener, and it serves the admin socket described below. `Controller.PKI` exposes the authority.

| Key or flag | Default | Meaning |
|---|---|---|
| `pki_dir`, `-pki-dir dir` | `data_dir/pki` | Directory of the CA and the PKI state. The controller must be its only writer |
| `enroll_listen`, `-enroll-listen address` | `localhost:8445` | TCP address of the HTTPS service for `POST /enroll` and `POST /renew` |
| `tls_names`, `-tls-names a,b` | see below | Host names and IP addresses in the controller certificate, which agents check against the address that they dial |
| `admin_socket`, `-admin-socket path` | `pki_dir/admin.sock` | Unix socket of the admin API. None without a PKI directory |
| `insecure_plain_grpc`, `-insecure-plain-grpc` | false | Serves the gRPC service without TLS, see [Plain mode](#plain-mode) |

When `tls_names` is empty the certificate carries `localhost`, `127.0.0.1`, `::1`, and the host of `grpc_listen` and of `enroll_listen` when it names one address. A wildcard such as `0.0.0.0` adds nothing, so a controller that listens on every interface sets `tls_names` to the names that its agents dial. The controller logs `certificate authority open` with the names and the SHA-256 fingerprint of the CA, which an operator can compare with the file that the agent trusts. The socket path must stay below 100 bytes, which the limit of unix sockets forces.

#### Operators, tokens, and revocation

The OpenAPI contract has no operation for tokens and revocation, so an admin REST endpoint is out of scope. The controller also must stay the only writer of `state.json`. We chose the admin socket, which is simpler and safer than a command that opens the state on its own, because a second process would need a lock that the PKI package does not have, and it could not end the open session. The controller serves a small HTTP API on a unix socket, and two subcommands of the binary call it.

```bash
fps-controller token  -agent-id edge-1 -ttl 30m -config /etc/fps/controller.yaml > edge-1.token
fps-controller revoke -agent-id edge-1 -config /etc/fps/controller.yaml
```

The subcommands find the socket from `-admin-socket`, `-pki-dir`, `-data-dir`, or the `-config` file, with the same layers as the controller. `token` prints the token alone on standard output so that a script can capture it, writes the expiry to standard error, and exits with 1 when no controller answers on the socket. The token is a secret that is valid once. Hand it to the agent through a file or the environment of the agent and never through a command line. `revoke` revokes every certificate and unused token of the identity and ends its open session at once. A token that the operator mints also adds the identity to the inventory when it is missing, because minting a token admits the agent. Revoking leaves the inventory entry, which `DELETE /v1/agents/{id}` removes.

The socket has mode 0600 inside the PKI directory, which has mode 0700, so the file system is the credential, and the controller refuses a socket directory that everyone may write. The admin API has no other authentication and never listens on TCP. At start up the controller connects to an existing socket file. When a process answers, it refuses to start, because two controllers must not share one PKI directory, and when nothing answers, it replaces the stale file of a dead controller. A path that is not a socket stays untouched and fails the start.

#### agent_auto_enroll with certificates

A certificate proves who an agent is, and the inventory decides whether the controller accepts it. A valid certificate for an identity that the inventory does not list gets `PERMISSION_DENIED` unless `agent_auto_enroll` is true, in which case the agent joins the inventory. The setting never admits an agent without a valid certificate, and it never replaces one. Minting a token enrolls the identity, so the normal path needs no `POST /v1/agents`. The inventory lives in memory and the PKI state on disk, so CTL-29 rebuilds the inventory at start: `wiring.New` calls `Authority.ValidIdentities`, which lists every identity that holds a certificate that the CA issued, that `Revoke` did not end, and that has not passed its `NotAfter` time, and passes the list to `Manager.Seed`. `Seed` adds each identity as an enrolled agent without labels, offline until it registers, and it leaves an identity that the inventory already holds alone. An agent with a valid certificate therefore registers again after a restart with `agent_auto_enroll: false`, while a revoked identity, an identity whose certificates all expired, and an identity that holds only an unused token are not seeded, so they need a new token. The restart loses what the inventory never persisted, which is the operator labels from `POST /v1/agents` and any reservation, and an operator sets the labels again. `agent_auto_enroll` keeps its meaning: `true` admits any holder of a valid certificate, even one that the PKI state does not list, and `false` admits an agent only when an operator or the PKI state enrolled it. The default stays false.

#### Plain mode

`insecure_plain_grpc` (flag `-insecure-plain-grpc`) serves `ControllerService` on plain TCP, trusts the `agent_id` of each request, opens no authority, and serves neither the enrollment listener nor the admin socket. It exists for development and for tests. The Rust agent has TLS since AGT-08, and the image dropped it in IMG-06, so only `tests/gates/gate1/run.sh` and `TestEndToEnd` still use it. The controller logs `INSECURE` warnings at start up, when the service starts listening, and for every identity that it accepts. The end to end test with the real agent (`engine/fps-agent/tests/controller_e2e.sh`) uses mutual TLS since AGT-08. Never set it on a shared network.

#### What the agent must change

AGT-08 implemented this list; it stays as the reference for other agent implementations.

The Rust agent (`engine/fps-agent`, AGT-08) has to do the following.

1. Build the tonic channel with TLS. This needs the tonic `tls` feature with a rustls provider. The channel trusts the CA from a file or a pinned fingerprint, presents the client certificate and key, uses the `https://` scheme, and sets the server name to a name in the controller certificate. The check for `http://` addresses can stay for local tests.
2. Enroll when it has no certificate. It generates an ECDSA P-256 key, writes it with mode 0600, builds a CSR with the common name set to its `--agent-id`, and sends the token and the CSR to the enrollment endpoint over TLS that trusts the CA. The token comes from a file or an environment variable, never from the command line, and the agent deletes it after the exchange. New flags are needed for the CA file, the token source, the enrollment address, and the directory that holds the key and the certificate.
3. Renew. From `renew_at` on, and before the certificate expires, it calls `POST /renew` with its current certificate and a fresh CSR, writes the new files atomically, and uses them for the next dial. It should redial only while no plan runs, because D12 item 12 drops the plan on a lost session. When the certificate is expired or revoked it logs the reason and enrolls again only if a new token exists.
4. Treat `UNAUTHENTICATED` and a handshake failure as permanent, because redialing cannot fix them, and follow the redial limit of D12 item 11.

#### Contract choice

The owner chose the HTTPS enrollment endpoint, which `pki` serves and which no proto file describes, so no contract changed. The endpoint adds an HTTP client to the agent (for example `hyper-util` or `reqwest` with rustls). The alternative that we did not take is an `Enroll` and a `Renew` RPC in `fps.agent.v1`, which would reuse the tonic stubs and let one listener serve everything. Tokens and revocation need no contract change, because the subcommands serve them.

### Tests

The tests open real TLS connections on the loopback interface. `TestMutualTLSAdmitsOnlyValidCertificates` starts a TLS server with `ServerTLSConfig`, shows that a freshly enrolled certificate is accepted and that the server reads the agent identity from the certificate subject, and shows that these clients are rejected: no client certificate, a certificate of another CA, a certificate that the CA signed without a record, a revoked identity, and an expired certificate. The clock of the authority is settable, so the expiry test moves it past `NotAfter`. `TestTokens` covers reuse, expiry at the exact instant, wrong and unknown and malformed tokens, a CSR for another identity, an RSA key, the lifetime limits, and 16 concurrent exchanges of one token. `TestRotation` renews a certificate directly and checks that an expired, foreign, or revoked certificate cannot renew. `TestHandlerEnrollAndRenewOverTLS` drives both endpoints over TLS. Further tests check the file modes of the CA, the refusal of a loose key, a half CA, a mismatched pair, a damaged state, and an expired CA, that the state never holds a token secret, and that used tokens and revocations survive a restart.

`make test-ctl-pki lint-ctl-pki` runs the package alone, and `make lint-ctl test-ctl` covers it with the rest of the module.

### Tests of the wiring

`make test-ctl-tls lint-ctl-tls` runs the CTL-27 tests, and `make lint-ctl test-ctl` covers them with the rest of the module.

- `TestMutualTLSAdmitsOnlyAgentsWithAValidCertificate` in `internal/wiring` is the acceptance test at the level of the controller. A minted token enrolls an agent over HTTPS, a single use token fails on its second use, and the agent registers over mutual TLS. An agent without a certificate, with a certificate of another CA, with a valid certificate for another identity, or with a valid certificate that the inventory does not list cannot register, and `RevokeIdentity` ends the open session, keeps the agent out, and lets a new token enroll it again.
- `TestRestartRebuildsInventoryFromPKIState` in `internal/agents` is the acceptance test of CTL-29. It enrolls agents, reopens the authority from the same state directory, seeds a new manager with `agent_auto_enroll` off, and shows that the agent registers again while a revoked identity, an unknown identity, and an identity whose certificate expired do not. `TestSeed` and `TestValidIdentities` cover the two methods alone.
- `TestAutoEnrollAdmitsCertificateHolders`, `TestSecureModeRefusesPlainClients`, and `TestPlainModeIsExplicit` cover `agent_auto_enroll` with certificates, a client without TLS, and the plain mode, where the PKI operations answer `ErrNoPKI`.
- `TestExpiredCertificateEndsAnIdleAgentSession` moves the clock past the lifetime of a certificate, and the sweep ends the session of the idle agent. The agent manager tests in `internal/agents` show that a busy agent keeps its session, and that `EndSession` and the expiry give their reasons.
- `TestAdminSocket`, `TestAdminSocketReplacesAStaleFile`, and `TestAdminHandlerValidatesRequests` cover the admin socket: mode 0600, the refusal of a live socket, the replacement of a stale one, and the validation of every request.
- `TestEndToEndMutualTLS` in `cmd/fps-controller` builds the binary in its default mode and runs the path with a Go fake agent. It mints a token with the `token` subcommand, enrolls over HTTPS with the CA file from the PKI directory, registers over mutual TLS, shows that no certificate, a plain client, a certificate of another CA, and another identity cannot register, revokes with the `revoke` subcommand, and enrolls again with a new token. It checks that the token never reaches the logs. `TestEndToEnd` keeps its Go fake agent over the plain mode.
