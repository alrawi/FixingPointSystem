# Contracts v0.1

These files define the interfaces between FixingPointSystem workstreams. Implementation work must conform to them, and changes follow the process in `CONTRIBUTING.md`.

| File | Defines |
|---|---|
| `openapi/fps-api-v1.yaml` | Controller REST API (OpenAPI 3.1) |
| `schemas/module-manifest.schema.json` | Module manifest |
| `schemas/scenario.schema.json` | Scenarios and flow program actions |
| `schemas/test-config.schema.json` | Test configuration |
| `examples/` | Example modules and tests that every validator run checks |

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

A template is a string with zero or more `{{ expression }}` placeholders. Contract v0.1 defines the core grammar below. Module helper functions such as `http1.request(...)` arrive with WebAssembly extensions in a later contract version.

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

The validator checks schema conformance and the semantic rules that JSON Schema cannot express. These rules cover unique flow ids, `after` and `spawn` references, dependency cycles, branch probabilities, reserved actions, template syntax, scenario references in mixes, host pool sizes, and port references.
