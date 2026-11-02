# Contracts v0.1

These files define the interfaces between FixingPointSystem workstreams. Implementation work must conform to them, and changes follow the process in `CONTRIBUTING.md`.

| File | Defines |
|---|---|
| `openapi/fps-api-v1.yaml` | Controller REST API (OpenAPI 3.1) |
| `schemas/module-manifest.schema.json` | Module manifest |
| `schemas/scenario.schema.json` | Scenarios and flow program actions |
| `schemas/test-config.schema.json` | Test configuration |
| `extension/fps-extension.wit` | Extension world `fps:extension@0.1.0` with the five hooks, the template function catalog, and the engine calls. `extension/README.md` describes the semantics and the mapping to the Rust trait. |
| `examples/` | Example modules and tests that every validator run checks |
| `proto/fps/agent/v1/agent.proto` | Agent control protocol between the controller and agents (gRPC, proto3) |
| `proto/buf.yaml` | buf module, lint rules, and breaking change rules for `proto/` |
| `examples/` | Example modules, tests, and agent plans that every validator run checks |

## Flow program semantics

Each flow runs one session. The client side and the server side each execute their action list in order. A flow fails when any action fails or times out.

| Action | Meaning |
|---|---|
| `connect` | Open a session to `host:port`. Without `host`, the engine picks an address from the server pool. |
| `accept` | Listen on `port` and accept one session. |
| `send` | Render the template and write the bytes. |
| `expect` | Read with one of three stop rules. `pattern` consumes through the first match. `bytes` consumes exactly N bytes. `min_bytes` waits for at least N bytes and consumes everything buffered. The default timeout is 10,000 ms. |
| `delay` | Wait a fixed time (`ms`) or a sample from `exponential`, `uniform`, `lognormal`, or `pareto`. Lognormal parameters apply to the natural log of the delay in `unit`, which defaults to seconds. |
| `set` | Bind scenario variables from templates. |
| `loop` | Repeat `do` `count` times. |
| `branch` | Choose one `do` block with probability `p`. Probabilities must sum to 1. |
| `spawn` | Start the named flow, which must declare `spawned: true`. |
| `close` | `fin` shuts down the write side and drains until the peer closes. `rst` aborts. `half` shuts down the write side and continues. |
| `raw` | Write hex bytes without templating. Only `strike` and `fuzzer` modules may use it. |

For UDP flows, `connect` binds the peer address, `accept` binds the port and waits for the first datagram, and `close` releases the socket.

## Template grammar

A template is a string with zero or more `{{ expression }}` placeholders. Contract v0.1 defines the core grammar below. Module helper functions such as `http1.request(...)` are template functions that a module extension exports through `extension/fps-extension.wit`. The template syntax for calling them, including keyword arguments, arrives in contract v0.2 (CON-04).

```
expression := reference | call | literal
reference  := ident ("." ident)*          # vars.host, session.id
call       := ident "(" [arg ("," arg)*] ")"
arg        := reference | literal
literal    := integer | 'single quoted string'
```

| Name | Value |
|---|---|
| `vars.<name>` | Scenario variable, after test parameter overrides |
| `session.id` | Session sequence number within the run |
| `session.flow` | Flow id |
| `fill(n)` | n printable bytes from a seeded generator |
| `hex(s)` | Bytes decoded from the hexadecimal string s |
| `rand_int(a, b)` | Uniform integer in [a, b] from the seeded generator |

## Validation

```bash
python3 -m pip install -r tools/contracts/requirements.txt
python3 tools/contracts/validate.py
```

```bash
cargo install wasm-tools --locked
wasm-tools component wit contracts/extension/fps-extension.wit
```

The validator checks schema conformance and the semantic rules that JSON Schema cannot express. These rules cover unique flow ids, `after` and `spawn` references, dependency cycles, branch probabilities, reserved actions, template syntax, scenario references in mixes, host pool sizes, and port references.

The plan checker validates the agent plans in `examples/plans/`. It needs the protobuf packages in `tools/contracts/requirements-proto.txt`, and CI runs it in the `proto` job together with `buf lint` and `buf breaking`.

```bash
python3 -m pip install -r tools/contracts/requirements-proto.txt
python3 tools/contracts/check_plans.py
python3 -m pytest tools/contracts/test_plans.py
buf lint contracts/proto
```

## Agent control protocol

The controller compiles each test configuration into one `Plan` per agent and drives every agent through `AgentService`. The agent serves the service over gRPC with mutual TLS, and the controller is its only client. The controller learns the agent address from `POST /v1/agents` and then calls `Register`.

| RPC | Allowed in state | Result |
|---|---|---|
| `Register` | any, when unbound or bound to the same controller | Binds the agent and returns `Capabilities` |
| `Heartbeat` | any, from the bound controller | Renews the binding and reports state, plan, phase, and the agent clock |
| `Prepare` | `IDLE`, `STOPPED`, `FAILED` | Verifies module digests, loads scenarios, binds ports, and moves to `PREPARED` |
| `Start` | `PREPARED` | Moves to `RUNNING` at once or at `start_at` |
| `Stop` | `PREPARED`, `RUNNING`, `STOPPING`, `STOPPED` | Ramps down or aborts and moves to `STOPPED`, or releases an unstarted plan and moves to `IDLE` |
| `StreamCounters` | `PREPARED` to `STOPPED` | Streams one snapshot per interval and ends after the final snapshot |

The agent moves to `STOPPED` on its own when the last load phase ends. When no heartbeat arrives within the heartbeat timeout, the agent aborts the run, releases the plan, and drops the binding, so that no traffic runs without a controller.

The agent reports errors with gRPC status codes.

| Code | Meaning |
|---|---|
| `INVALID_ARGUMENT` | The plan breaks a rule of this contract |
| `FAILED_PRECONDITION` | The call is out of order, a module digest does not match, or another controller holds the binding |
| `NOT_FOUND` | The `plan_id` does not name the loaded plan |
| `RESOURCE_EXHAUSTED` | The plan needs more workers, cores, or ports than the agent has |
| `UNIMPLEMENTED` | The plan needs a backend or module type that the agent does not support |

### Plan rules

A plan carries everything that an agent needs except the module packages, which the agent already holds and checks by digest. The checker enforces the rules below.

1. Each module digest is `sha256:` followed by 64 lowercase hex digits. The digest covers the module tree. Every regular file under the module directory contributes the line `<sha256 hex of the file>  <relative POSIX path>` with a final newline, lines sort by the UTF-8 bytes of the path, and the digest is the SHA-256 of their concatenation. Symbolic links are not allowed. `check_plans.py --print-digests` prints the digests of a module directory.
2. Each scenario source is the scenario file in YAML, and its digest is the SHA-256 of the UTF-8 bytes of the source. The source validates against `schemas/scenario.schema.json`.
3. Mix weights are normalized within the basis and sum to 1. Each worker rate names a mix entry. Rate objectives (`SESSIONS_PER_SECOND`, `THROUGHPUT_BPS`) compile to `sessions_per_second`, and concurrency objectives (`CONCURRENT_SESSIONS`, `SIMULATED_USERS`) compile to `concurrent_sessions`.
4. Worker rates are steady state targets. Each load phase scales every target from `start_fraction` to `end_fraction` with its shape. A hold phase keeps one fraction, and a step phase moves in `steps` equal steps.
5. Host index i of a pool maps to the i-th address of the pool prefix, counting from the first address after the network address, skipping the gateway, and for IPv4 skipping the broadcast address. Host i uses the MAC address `mac_base` plus i. A host pool slice covers the pool indices from `offset` to `offset + count - 1`, and the host ranges of the workers lie inside the slice and never overlap.
6. Each worker derives its generators from the plan seed, its worker id, and the session id, so a replay reproduces every session.
7. Counter, histogram, and criterion metric names come from the counter catalog in `metrics/catalog.yaml`, which CON-02 defines.

### Canonical JSON mapping

The standalone agent mode reads a plan file in the proto3 JSON mapping of `fps.agent.v1.Plan`. A canonical plan file uses lowerCamelCase field names, writes enum values by name, writes 64 bit integers such as `seed` as strings, writes durations as strings such as `"30s"`, and leaves out fields that hold their default value. A field that is absent therefore means zero, an empty string, or false. Numbers in `params` are JSON numbers, and the interpreter treats an integral number as an integer. The checker rejects unknown fields and any file that differs from the canonical form of its own content. `examples/plans/http_dns_smoke.json` shows a two worker plan for the `http1` and `dns` example modules.
