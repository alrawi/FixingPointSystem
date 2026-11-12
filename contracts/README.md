# Contracts

Each contract carries its own version, and the table names it: the REST API 0.12.0, the agent protocol 0.2.4, the module manifest v0.5, the scenario contract v0.6, the test configuration v0.3, the extension interface 0.1.0, and the metric catalog 0.8.0.

These files define the interfaces between FixingPointSystem workstreams. Implementation work must conform to them, and changes follow the process in `CONTRIBUTING.md`.

| File | Defines |
|---|---|
| `openapi/fps-api-v1.yaml` | Controller REST API (OpenAPI 3.1, version 0.12.0) |
| `schemas/module-manifest.schema.json` | Module manifest (v0.5), including the template function catalog |
| `schemas/scenario.schema.json` | Scenarios and flow program actions (scenario contract v0.6) |
| `schemas/test-config.schema.json` | Test configuration (v0.3) |
| `extension/fps-extension.wit` | Extension world `fps:extension@0.1.0` with the five hooks, the template function catalog, and the engine calls. `extension/README.md` describes the semantics and the mapping to the Rust trait. |
| `metrics/catalog.yaml` | Every counter, gauge, and histogram with its type, unit, level, and labels (catalog 0.8.0) |
| `metrics/catalog.schema.json` | Format of the metric catalog |
| `proto/fps/agent/v1/agent.proto` | Agent control protocol 0.2.4 between the controller and agents (gRPC, proto3). Agents dial the controller. |
| `proto/buf.yaml` | buf module, lint rules, and breaking change rules for `proto/` |
| `examples/` | Example modules, tests, and agent plans that every validator run checks |

## Flow program semantics

Each flow runs one session. The client side and the server side each execute their action list in order. A flow fails when any action fails or times out.

| Action | Meaning |
|---|---|
| `connect` | Open a session to `host:port`. Without `host`, the engine picks an address from the server pool. |
| `accept` | Listen on `port` and accept one session. |
| `send` | Render the template and write the bytes. |
| `expect` | Read with one of three stop rules. `pattern` consumes through the first match. `bytes` consumes exactly N bytes. `min_bytes` waits for at least N bytes and consumes everything buffered. The default timeout is 10,000 ms. The consumed bytes become `last`. UDP flows follow the datagram rules below. |
| `delay` | Wait a fixed time (`ms`) or a sample from `exponential`, `uniform`, `lognormal`, or `pareto`. Lognormal parameters apply to the natural log of the delay in `unit`, which defaults to seconds. |
| `set` | Bind scenario variables from templates. |
| `loop` | Repeat `do` `count` times. |
| `branch` | Choose one `do` block with probability `p`. Probabilities must sum to 1. |
| `spawn` | Start the named flow, which must declare `spawned: true`. |
| `tls_upgrade` | Wrap the transport of the running session in TLS in place. The side's role starts or accepts the handshake, and every later action runs over TLS. TCP flows only. |
| `close` | `fin` shuts down the write side and drains until the peer closes. `rst` aborts. `half` shuts down the write side and continues. |
| `raw` | Write hex bytes without templating. Only `strike` and `fuzzer` modules may use it. |

A flow may also carry the keyword `transport: {module: tls, profile, server_name?}`, which is sugar for a `tls_upgrade` right after the opener of each side the flow declares. The client side upgrades as the client with `server_name`, the server side upgrades as the server, and the profile names a TLS profile of the run for both sides. The loader desugars it, so `transport: {module: tls, profile: p, server_name: n}` means `tls_upgrade: {profile: p, server_name: n}` after `connect` and `tls_upgrade: {profile: p}` after `accept`. It composes with explicit `tls_upgrade` actions, which run after it, and it applies only to TCP flows. A flow whose sides need different profiles uses `tls_upgrade` on each side. `module` takes only `tls`, and `profile` is required (scenario contract v0.5).

For UDP flows, `connect` binds the peer address, `accept` binds the port and waits for the first datagram, and `close` releases the socket. The datagram that `accept` waits for stays queued for the first `expect`.

### Datagram rules for UDP

A UDP flow keeps datagram boundaries. The engine applies six rules.

1. Each `send` writes its rendered bytes as one datagram. A rendering longer than 65,507 bytes fails the session.
2. Each `expect` reads exactly one datagram and consumes all of it. The engine never joins two datagrams and never leaves part of a datagram for the next `expect`.
3. The stop rule checks that one datagram. `bytes: N` requires a datagram of exactly N bytes, `min_bytes: N` requires at least N bytes, and `pattern` requires the pattern somewhere in the datagram. A datagram that breaks the rule fails the session with reason `pattern_mismatch`.
4. `max_datagram` sets the largest datagram that the `expect` accepts, from 1 to 65,507 bytes with a default of 65,507. A longer datagram fails the session with reason `pattern_mismatch`, and the engine never truncates it silently. The validator rejects `max_datagram` on TCP flows, and on UDP flows it rejects a fixed `bytes`, `min_bytes`, or pattern length that exceeds the limit.
5. `timeout_ms` bounds the wait for that one datagram. A protocol that answers with several datagrams reads them with one `expect` each, for example inside a `loop`.
6. A session receives only datagrams from its peer. The client socket connects to the peer address, and the server session binds to the source address of the first datagram, so datagrams from other sources belong to other sessions. The engine does not reorder, deduplicate, or retransmit datagrams.

The engine reads datagrams into one scratch buffer per worker and copies a datagram into the session only when a template of that side refers to `last`, so the limit costs no memory per session.

## Metric catalog

The catalog in `metrics/catalog.yaml` names every metric once. The agent records metrics under these names, the controller derives and serves them, and test criteria refer to them. A metric that the catalog lacks cannot appear in a criterion or in the OpenAPI document.

Each entry carries six fields and an optional formula.

| Field | Meaning |
|---|---|
| `name` | Dotted lower case name, for example `sessions.failed` |
| `type` | `counter` only increases during a run, `gauge` is a value at one instant, and `histogram` is an HDR distribution |
| `unit` | One unit from the schema, for example `sessions`, `bytes`, or `us` |
| `level` | `application`, `scenario`, `flow_action`, `transport`, or `port_host` |
| `labels` | Labels the metric carries, each declared in the `labels` section |
| `description` | What the metric counts or measures |
| `formula` | For a derived gauge, the arithmetic over other catalog metrics that the controller evaluates |

The levels follow the counter table of the specification. Its application and scenario row splits into two levels. The scenario level holds the session, transaction, and byte counters labeled by application and scenario, and the application view of the API sums them over the scenario label. The application level holds the mix and realism gauges.

A metric reference names a counter or a gauge directly. A reference to a histogram appends one statistic, which is `p50`, `p90`, `p99`, `p999`, `min`, `max`, `mean`, or `count`, as in `sessions.response_time.p99`. A criterion carries no labels and applies to the whole run, so the controller aggregates the metric over every label before it compares the value.

The OpenAPI document names the metric behind a response field with the `x-fps-metric` keyword. Every property of `RunSummary` carries one. Prometheus exposition maps a catalog name to `fps_` followed by the name with dots replaced by underscores, and adds `_total` to counters.

The label `application` takes the value `all` for a series that totals every application of the run, and the label `property` takes the value `both` for the payload sizes of the two directions pooled (metric catalog 0.3.0). The realism metrics `realism.ks_statistic` and `realism.ks_p_value` use them for comparisons that no single module owns. Metric catalog 0.5.0 (CON-23) gives every check kind of the realism report a series: `realism.check_failures` (label `check`) for all of them, and `realism.parser_errors`, `realism.decrypted_ratio`, `realism.ja4_match_ratio`, `realism.duplicate_ratio`, `realism.structure_failures`, and `realism.transport_deviations` for the kinds that judge such a thing. The change entry `changes/CON-23.md` gives the target and the outcome of each, and `examples/realism/check-metrics.yaml` holds a passing and a failing case for every kind. No module may be named `all`, and the validator rejects the name. An `all` series sits next to the per application series, so a criterion on such a gauge selects it with `where: {application: all}`, and a reader that sums an `application` label must leave the `all` series out.

## Arrival and size distributions

The test configuration may declare the distribution that the traffic of a run is meant to follow (test configuration v0.3). Both fields are optional, and a configuration without them keeps its meaning.

| Field | Where | Describes |
|---|---|---|
| `arrival` | `profile` or one `profile.mix` entry | The gaps between the starts of consecutive sessions, for the whole run or for one mix entry |
| `size` | `profile` or one `profile.mix` entry | The payload sizes in bytes, by direction, with the keys `request` (client to server), `response` (server to client), and `both` (the two pooled) |

An entry overrides the profile. A mix entry that has its own `arrival` replaces `profile.arrival` for its sessions, and a mix entry that names a direction of `size` replaces that direction of `profile.size`. Each distribution is an object with `dist` and the parameters of the table. An arrival uses the parameter names of a scenario `delay`, in milliseconds unless `unit` says otherwise, and a size uses bytes.

| `dist` | Arrival parameters | Size parameters |
|---|---|---|
| `constant` | `ms`, optional `tolerance_ms` | `value`, optional `tolerance` |
| `exponential` | `mean_ms` | `mean` |
| `uniform` | `min_ms`, `max_ms` | `min`, `max` |
| `lognormal` | `mu`, `sigma`, optional `unit` (`s` or `ms`, default `s`) | `mu`, `sigma` |
| `pareto` | `alpha`, `scale_ms` | `alpha`, `scale` |

`mu` is the natural logarithm of the value in the `unit` (arrivals) or in bytes (sizes), and a Pareto distribution has `scale` as its smallest value, as the scenario `delay` and the realism check define them. The validator rejects a uniform distribution whose minimum is above its maximum.

The fields are declarations. They add no command for an agent in this version, so the arrivals of a run still come from the load objective and its phases, and the sizes still come from the scenarios and their `params`. The realism runner takes its targets from them, with these mappings.

| Declaration | Realism comparison |
|---|---|
| `profile.arrival` | `inter_arrival` with `application` set to `all` |
| `profile.mix[].arrival` | `inter_arrival` with `application` set to the module of the entry's scenario |
| `size.request`, `size.response` | `request_bytes` and `response_bytes`, with `application` set to `all` at the profile level and to the module of the entry in a mix entry |
| `size.both` | `both`, with `application` as above |

A declaration should agree with the rest of the configuration. An exponential `arrival` in a steady phase has `mean_ms` of 1000 divided by the `sessions_per_second` objective, and a configuration whose declaration disagrees with its objective fails the realism check and not the validator, which does not compare the two.

## Template grammar

A template is a string with zero or more `{{ expression }}` placeholders. The engine copies text outside placeholders byte for byte. Scenario contract v0.2 extends the v0.1 grammar with module function calls, keyword arguments, nested calls, float and boolean literals, and the reference `last`. Every v0.1 template keeps its meaning.

```
template   := { text | "{{" expression "}}" }
expression := literal | reference | call
reference  := "last" | "vars" "." ident | "session" "." ident | "profiles" "." ident
call       := name "(" [ arguments ] ")"
name       := ident | module "." ident | "dist" "." ident
arguments  := positional { "," positional } { "," keyword } | keyword { "," keyword }
positional := expression
keyword    := ident "=" expression
literal    := integer | float | string | "true" | "false"
integer    := [ "-" ] digit { digit }
float      := [ "-" ] digit { digit } "." digit { digit }
string     := "'" { any character except "'" } "'"
ident      := ( letter | "_" ) { letter | digit | "_" }
```

The grammar has five further rules.

1. A placeholder ends at the first `}}`, so a string literal cannot contain `}}` or a single quote. Strings have no escape sequences. Binary content comes from `hex(s)`.
2. Positional arguments come before keyword arguments, and a keyword names each parameter at most once. A trailing comma is a syntax error.
3. Arguments may be calls, so `http1.response(200, body=fill(vars.size))` is valid. Calls nest at most 8 levels deep.
4. A function name with a module prefix calls a template function of that module. The module must be the module that ships the scenario or a module in its `requires` list. Core functions have no prefix.
5. The module names `vars`, `session`, `last`, `profiles`, `dist`, `true`, and `false` are reserved, because a template could not tell them apart from references and literals. The name `all` is reserved too (see the metric catalog). The validator rejects all eight. The controller registry rejects `vars`, `session`, `last`, `true`, and `false` today, so a module named `profiles`, `dist`, or `all` fails the validator and would still install; do not rely on that.

| Name | Type | Value |
|---|---|---|
| `vars.<name>` | type of the variable | Scenario variable, after test parameter overrides |
| `session.id` | int | Session sequence number within the run |
| `session.flow` | text | Flow id |
| `last` | bytes | The bytes that the previous `expect` of the same side consumed |
| `profiles.<name>` | text | The entries of a profile of the scenario, each rendered as `Name: value` and CRLF in declaration order (scenario contract v0.3) |
| `dist.<name>(...)` | int | A draw from `pareto`, `lognormal`, `exponential`, or `uniform` with the run's seeded generator, limited by `cap` (scenario contract v0.3) |
| `fill(n)` | text | n printable bytes from a seeded generator |
| `hex(s)` | bytes | Bytes decoded from the hexadecimal string s |
| `rand_int(a, b)` | int | Uniform integer in [a, b] from the seeded generator |
| `int_bytes(value, width, endianness)` | bytes | The unsigned integer `value` as exactly `width` bytes, 1 to 8, in `'big'` (the default) or `'little'` byte order (scenario contract v0.6) |
| `now(format)` | text or int | The wall clock at the moment of the call, as `'http_date'` (text), `'epoch_s'` (int), or `'epoch_ms'` (int) (scenario contract v0.6) |
| `<module>.<name>(...)` | bytes | The bytes that the module's `render` hook returns |

### Module calls

A call such as `dns.query(name=vars.host, qtype='A')` renders through the `render` hook of the module extension, which `extension/fps-extension.wit` defines. The engine passes the function name without the module prefix, the positional arguments in call order, and the keyword arguments in call order. The rendered bytes replace the placeholder in a `send` template. A `set` action parses them as text with the rules of a command line override.

Core functions accept keyword arguments with the parameter names in the table above, so `fill(n=16)` equals `fill(16)`.

### The reference `last`

`last` holds every byte that the most recent `expect` of the same side consumed. A `pattern` expect includes the match, a `bytes` expect holds exactly N bytes, a `min_bytes` expect holds everything it consumed, and on UDP `last` holds the whole datagram. Each `expect` replaces `last`. A side never sees the `last` of the other side, and `last` does not carry into a spawned flow or a flow that runs `after` this one. A `set` action copies a value out of `last` into a variable that later flows read.

A template may refer to `last` only when an `expect` always runs earlier on the same side. The validator checks this rule in document order. An `expect` inside a `loop` counts after the loop only when `count` is a positive integer literal, and an `expect` inside a `branch` counts after the branch only when every arm contains one.

### The functions `int_bytes` and `now`

Scenario contract v0.6 adds two core functions. Both have no module prefix, accept keyword arguments with the parameter names of the table above, and draw nothing from the seeded generator, so a template that uses them leaves every other drawn value of the session unchanged.

`int_bytes(value, width, endianness='big')` writes an integer into a binary field, such as a length prefix or a transaction identifier.

- `width` is the number of bytes, from 1 to 8. `endianness` is `'big'` (network order) or `'little'`, and a template that leaves it out gets `'big'`.
- `value` is an unsigned integer that fits the width. `int_bytes(258, 2)` renders the bytes `01 02`, and `int_bytes(258, 2, 'little')` renders `02 01`. The largest `value` of an `int` is 2^63 - 1, so a width of 8 takes the values from 0 to 2^63 - 1.
- A negative value, a value that does not fit, a width outside 1 to 8, and another endianness are errors. The validator reports a literal argument at load time. A value that comes from a variable or another call fails the session at render time with the reason `template_error`, and the engine never truncates or wraps a value.
- The result is `bytes`, and its length is exactly `width`. A modeled byte count therefore does not depend on `value`.

`now(format)` reads the clock for headers and timestamps. The three formats render as follows.

| `format` | Result | Rendering |
|---|---|---|
| `'http_date'` | text | The IMF fixdate of RFC 9110 section 5.6.7 in UTC, always 29 bytes, for example `Tue, 10 Nov 2026 06:15:04 GMT`. Day and month names are the English abbreviations, and the day and the time have two digits. |
| `'epoch_s'` | int | Whole seconds since 1970-01-01T00:00:00Z, truncated toward zero. |
| `'epoch_ms'` | int | Whole milliseconds since 1970-01-01T00:00:00Z, truncated toward zero. |

An unknown format is an error, reported by the validator for a literal and by `template_error` at render time otherwise. The validator types the result by a literal format and gives `any` to a format that comes from a variable.

`now` is the one core function that is not a function of the run seed and the session, so the rule for replays and goldens is part of the contract.

1. Each evaluation reads the clock, so two calls in one template may straddle a second boundary. A scenario that needs one instant in several places binds it once with `set`. A variable is not re-evaluated.
2. The engine reads the clock through the same seam as its random source. A production run reads the wall clock of the agent that renders the template, in UTC. The agents of a run have no common clock, so a client and a server that both call `now` can differ by the skew of their hosts.
3. A test harness, a golden test, or a replay that needs equal bytes pins the clock. A pinned clock returns one fixed instant, and it may advance by a fixed step per call. The instant is a parameter of the harness, and a golden file must record it. A golden that does not pin the clock may not contain output of `now`.
4. The length of the output does not depend on the instant, within the years 2001 to 2286. `http_date` is 29 bytes, `epoch_s` is 10 digits, and `epoch_ms` is 13 digits. A replay therefore reproduces the byte counts of a scenario without pinning the clock, and only the content of the date and epoch fields differs.
5. The compiler never folds a call of `now`, as it never folds `fill` and `rand_int`, because its value differs between sessions.

### Variables across `after`

A variable that a `set` action binds lives in the scenario instance, which is the set of flows that one start of the scenario runs. The rule has five parts.

1. A flow that runs `after` another flow sees every variable that the earlier flow bound, directly or through a chain of `after` entries. The flow reads the value as it stood when the flow started, so `after` carries variables as well as order.
2. A variable that a flow binds is visible to its own later actions, to every flow ordered after it by `after`, and to the flows that it starts with `spawn`, which see the bindings of the parent side up to the `spawn` action.
3. Two flows that `after` does not order have no defined order of completion. A flow must not read a variable that only such a flow binds, and when two flows bind the same name, the flow that completes last wins for the flows that read it later.
4. The two roles of a flow run on different agents and share no variables during the run. A variable that the client side binds is visible to the client sides of later flows, and a variable of the server side to the server sides. A value that both roles need is bound by each role from its own `last`, as the generators of CAP-02 do for tokens that a request carries and the response echoes. An engine that runs both roles in one process may show a role the bindings of the other, and a scenario must not depend on that.
5. Each scenario instance starts with the values of `vars`, after the overrides of the test parameters, and the bindings of one instance never reach another. A `set` replaces the value of the name in its instance only.

A `set` parses the rendered value with the rules of a command line override, so the validator types a variable that only a `set` binds as `any`. The validator checks that a variable is bound somewhere in the scenario, and it does not check the order of the flows.

### Types

Values carry one of the types `int`, `float`, `boolean`, `text`, and `bytes`, and a parameter may also take `any`. These names match the `value-type` enum of the extension interface. A variable takes the type of its value under `vars`. A variable that only a `set` action binds has type `any`, because its value depends on the rendered text. An argument binds a parameter of the same type, a `float` parameter also takes an `int`, a `bytes` parameter also takes `text`, and `any` on either side matches everything. A module call result is `bytes`, so it reaches a `text` or `int` parameter only through a variable that a `set` action binds.

### Template function catalog

The validator checks module calls without running extension code. Each module manifest lists its template functions under `functions`, with the same names, parameters, types, and defaults as the `functions` export of its extension. The validator reads that list and reports an unknown function, a missing required parameter, an unknown or repeated keyword, too many positional arguments, and an argument whose type cannot bind its parameter. When the manifest of a required module is not available, the validator reports a warning and leaves the call unchecked.

```yaml
functions:
  - name: query
    summary: Query message in RFC 1035 wire format with a transaction id from the session generator.
    params:
      - { name: name, type: text }
      - { name: qtype, type: text, default: A }
```

A parameter without `default` is required. A `bytes` default is a string of hexadecimal digit pairs. A manifest that declares `functions` together with an `extension` must list the `render` hook. When the engine loads a module, it compares the manifest catalog with the result of `functions` and rejects the module when the two differ in any function name, parameter name, parameter order, type, or default. The manifest is therefore the single catalog that scenario authors, the validator, and the controller read, and the extension only confirms it.

## Validation

```bash
python3 -m pip install -r tools/contracts/requirements.txt
python3 tools/contracts/validate.py
```

```bash
cargo install wasm-tools --locked
wasm-tools component wit contracts/extension/fps-extension.wit
```

The validator checks schema conformance and the semantic rules that JSON Schema cannot express. These rules cover unique flow ids, `after` and `spawn` references, dependency cycles, branch probabilities, reserved actions, template syntax and argument types, module calls against the manifest function catalog, the precedence rule of `last`, the UDP datagram rules, `tls_upgrade` and `transport` only in TCP flows, the literal arguments of `int_bytes` and `now`, the bounds of uniform arrival and size distributions, the reserved module names, scenario references in mixes, host pool sizes, and port references. The metric rules check that catalog names and labels are unique and declared, that formulas refer to catalog counters and gauges, and that every metric in test criteria and in `x-fps-metric` annotations exists in the catalog.

## Module rules of the registry

The controller registry refuses a module at install for reasons that the manifest schema cannot express. `tools/contracts/validate.py` applies the same rules (CON-30), so a module that validates installs. The rules are the license, the file names, the links, and the files that the manifest names.

- The `license` is an SPDX expression that every license and exception of which is on the allow list of `controller/internal/registry/license.go`, and that is compatible with GPL-3.0: every `AND` operand is compatible and at least one operand of each `OR` is. Operators are upper case. The schema checks the length only.
- Every file and directory of the module tree, not only the named files, has a path of letters, digits, `_`, `.`, `-`, and `/` and no `.` or `..` segment.
- The tree holds no symbolic link and no special file.
- Every path that the manifest names is a regular file of the tree.
- The `extension.wasm` path is the constant `extension.wasm`. The agent loads only that file at the module root (`engine/fps-agent/src/modules.rs`), and the agent does not read the path, so the schema accepts no other value (CON-30). A module that ships the file needs no `extension` key for the agent to load it, and the key declares the hooks.
- Each key of `expected_bytes` is the name of a scenario in `scenarios`. Only the validator checks this today.

The module types `tunnel`, `strike`, `evasion`, and `fuzzer` are reserved. The schema accepts them, and no reader rejects them yet. The one rule that depends on the type is that the `raw` action needs a `strike` or `fuzzer` module.

The plan checker validates the agent plans in `examples/plans/`. It needs the protobuf packages in `tools/contracts/requirements-proto.txt`, and CI runs it in the `proto` job together with `buf lint` and `buf breaking`.

```bash
python3 -m pip install -r tools/contracts/requirements-proto.txt
python3 tools/contracts/check_plans.py
python3 -m pytest tools/contracts/test_plans.py
buf lint contracts/proto
```

## Agent control protocol

The controller compiles each test configuration into one `Plan` per agent. Each agent dials the controller and registers itself, so an agent behind NAT or in a cloud needs no inbound port. The controller serves `ControllerService` over gRPC with mutual TLS, and every agent is a client. The agent serves no gRPC service.

| RPC | Kind | Purpose |
|---|---|---|
| `Register` | unary | The agent announces its identity, `Capabilities`, state, and loaded plan. The controller returns its `controller_id`, a session token, and the heartbeat interval and timeout. |
| `Session` | bidirectional stream | Carries all control traffic of one registered agent until the connection ends |

### Registration and identity

The agent identity is the identity in its client certificate, and `RegisterRequest.agent_id` must equal it. The controller rejects an identity that it does not accept with `PERMISSION_DENIED`, and a request whose `agent_id` differs from the certificate with `UNAUTHENTICATED`. Operators may enroll an identity ahead of time with `POST /v1/agents`, which sets its labels. A repeated `Register` for the same identity is idempotent, returns a new session token, and revokes the previous one. The controller rejects an agent whose `Capabilities.engine_api` it cannot drive with `FAILED_PRECONDITION`.

### Session stream

The agent opens `Session` right after `Register`. The agent sends `SessionRequest` messages and the controller sends `SessionResponse` messages.

| Direction | Message | When |
|---|---|---|
| agent to controller | `open` | First message, with the agent identity and the session token. Any other first message, a repeated `open`, or a revoked token ends the stream with `UNAUTHENTICATED`. |
| agent to controller | `heartbeat` | Once per heartbeat interval, and at once after every state change. It carries the state, plan, phase, and agent clock. |
| controller to agent | `heartbeat_ack` | Once per heartbeat, with `command_id` zero, the echoed sequence, and the controller clock |
| controller to agent | `prepare` | Carries the plan, the counter interval, and the per worker option |
| controller to agent | `start` | Starts the prepared plan at once or at `start_at` |
| controller to agent | `stop` | Ramps down or aborts a run, or releases an unstarted plan |
| agent to controller | `result` | Exactly one per command, with the echoed `command_id` and either a result or a `CommandError` |
| agent to controller | `snapshot` | One counter snapshot per counter interval from the start of the run until the final snapshot |

The controller sends at most one command at a time and waits for its result before the next, except that `stop` may follow any command without waiting. At most one session per agent is open, and a new session for the same identity ends the older one with `ABORTED`.

### Agent state machine

| Command | Allowed in state | Result |
|---|---|---|
| `prepare` | `IDLE`, `STOPPED`, `FAILED` | Moves through `PREPARING` and verifies module digests, loads scenarios, binds ports, and moves to `PREPARED` |
| `start` | `PREPARED` | Moves to `RUNNING` at once or at `start_at` and begins the counter snapshots |
| `stop` | `PREPARED`, `RUNNING`, `STOPPING`, `STOPPED` | Ramps down through `STOPPING` or aborts and moves to `STOPPED` with a final snapshot, or releases an unstarted plan and moves to `IDLE` |

The agent moves to `STOPPED` on its own when the last load phase ends, and sends a final snapshot with `is_final` and a heartbeat. An internal error moves the agent to `FAILED`, and the next heartbeat explains it in `message`.

### Connection loss

Both sides watch the same heartbeat timeout. When the controller receives no heartbeat within the timeout, it marks the agent offline and marks a run on that agent degraded. When the agent receives nothing from the controller within the timeout, it aborts any run, releases the plan, and moves to `IDLE`, so that no traffic runs without a controller.

After a lost connection, the agent dials again with exponential backoff, calls `Register` with its current state and `plan_id`, and opens a new session. When it returns within the heartbeat timeout, the run continues and the controller resumes it from the reported state. Counter snapshots keep their sequence numbers across sessions, and the counters are cumulative, so the controller drops duplicates and loses no data.

### Error codes

A failed command returns a `CommandError` in its `result`. The numbers of `ErrorCode` equal the gRPC status codes of the same name. A failed command leaves the agent state unchanged, except for `ERROR_CODE_INTERNAL`, which moves the agent to `FAILED`.

| Code | Meaning |
|---|---|
| `ERROR_CODE_INVALID_ARGUMENT` | The plan or the command breaks a rule of this contract |
| `ERROR_CODE_NOT_FOUND` | The `plan_id` does not name the loaded plan |
| `ERROR_CODE_RESOURCE_EXHAUSTED` | The plan needs more workers, cores, or ports than the agent has |
| `ERROR_CODE_FAILED_PRECONDITION` | The command is out of order for the agent state, or a module digest does not match |
| `ERROR_CODE_UNIMPLEMENTED` | The plan needs a backend or module type that the agent does not support |
| `ERROR_CODE_INTERNAL` | The agent failed for a reason of its own |

The `Register` call and the `Session` stream end with gRPC status codes.

| Code | Meaning |
|---|---|
| `UNAUTHENTICATED` | The `agent_id` differs from the certificate identity, or the session token is missing, revoked, or not the first message |
| `PERMISSION_DENIED` | The controller does not accept this agent identity |
| `FAILED_PRECONDITION` | The controller cannot drive the contract version that the agent reports |
| `ABORTED` | A newer session for the same agent replaced this one |
| `UNAVAILABLE` | The controller shuts down, and the agent dials again |

### Plan rules

A plan carries everything that an agent needs except the module packages, which the agent already holds and checks by digest. The checker enforces the rules below.

1. Each module digest is `sha256:` followed by 64 lowercase hex digits. The digest covers the module tree. Every regular file under the module directory contributes the line `<sha256 hex of the file>  <relative POSIX path>` with a final newline, lines sort by the UTF-8 bytes of the path, and the digest is the SHA-256 of their concatenation. Symbolic links are not allowed. `check_plans.py --print-digests` prints the digests of a module directory.
2. Each scenario source is the scenario file in YAML, and its digest is the SHA-256 of the UTF-8 bytes of the source. The source validates against `schemas/scenario.schema.json`.
3. Mix weights are session shares between 0 and 1 and sum to 1. The compiler converts the byte weights of the bytes basis into session shares. Each worker rate names a mix entry. Rate objectives (`SESSIONS_PER_SECOND`, `THROUGHPUT_BPS`) compile to `sessions_per_second`, and concurrency objectives (`CONCURRENT_SESSIONS`, `SIMULATED_USERS`) compile to `concurrent_sessions`.
4. Worker rates are steady state targets. Each load phase scales every target from `start_fraction` to `end_fraction` with its shape. A hold phase keeps one fraction, and a step phase moves in `steps` equal steps.
5. Host index i of a pool maps to the i-th address of the pool prefix, counting from the first address after the network address, skipping the gateway, and for IPv4 skipping the broadcast address. Host i uses the MAC address `mac_base` plus i. A host pool slice covers the pool indices from `offset` to `offset + count - 1`, and the host ranges of the workers lie inside the slice and never overlap.
6. Each worker derives its generators from the plan seed, its worker id, and the session id, so a replay reproduces every session.
7. Counter, histogram, and criterion metric names come from the counter catalog in `metrics/catalog.yaml`, which CON-02 defines.

### Canonical JSON mapping

The standalone agent mode reads a plan file in the proto3 JSON mapping of `fps.agent.v1.Plan`. A canonical plan file uses lowerCamelCase field names, writes enum values by name, writes 64 bit integers such as `seed` as strings, writes durations as strings such as `"30s"`, and leaves out fields that hold their default value. A field that is absent therefore means zero, an empty string, or false. Numbers in `params` are JSON numbers, and the interpreter treats an integral number as an integer. The checker rejects unknown fields and any file that differs from the canonical form of its own content. `examples/plans/http_dns_smoke.json` shows a two worker plan for the `http1` and `dns` example modules. It is the `plan` field of a `prepare` command, and CON-06 left the `Plan` message unchanged, so the example stays valid.
