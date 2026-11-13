## [2026-11-12] ingest | The raw action in the engine

ENG-29 (issue #523) makes the engine run the `raw` action of scenario contract v0.6. The compiler of `fps-flow` decodes the hex string at load time and emits `Op::Send` of `Template::literal(bytes)`, so the interpreter writes the bytes on the session as it writes any send, and no template parser reads them. `fps-interp` needed no change. The contract check already refused `raw` in a module that is not of type `strike` or `fuzzer`, and a scenario loaded without a module context, with `LoadError::ReservedAction`. The runnable program no longer fails with `Unsupported` for the action.

The tests are the following.
- `engine/fps-runtime/tests/raw.rs` builds a module of type `strike` in the test, with no dependency on `modules/strike/`. A client runtime writes 30 bytes with `raw`, which hold `{{ x }}`, two NULs, `}}`, and 0xff, to a plain `TcpListener`. Five sessions succeed, `bytes_tx` is 150, and the listener reads the exact bytes of every session. The same scenario in a module of type `app`, `tunnel`, `evasion`, `transport`, `host`, or `content` fails to load with `ReservedAction` and the message `flow 'a': client: action 2: 'raw' is reserved for module types ["fuzzer", "strike"]`.
- Unit tests in `program.rs` cover strike and fuzzer modules in the three loader modes, `raw` inside `loop` and `branch`, every other module type in two modes, a load without a module context, and bad hex.
- `engine/fps-flow/tests/fixtures.rs` loads the existing fixtures `st/scenarios/raw_ok.yaml` and `fx/scenarios/raw_reserved.yaml`, and no longer tolerates `Unsupported` for a raw fixture.

What the package does not cover:
- A UDP flow sends the bytes as one datagram through the same send op. The tests cover TCP end to end and UDP only as far as the compiled program.
- Stateless SYN floods and large spoofed source pools need packet level sending that a write on a session socket does not give. MOD-29 writes down what they would need from the engine.
- A run through the agent was not made, because the strike module belongs to the owner (MOD-27 to MOD-29). The agent loads a scenario with `Scenario::parse_instance` and the context of its module (`engine/fps-agent/src/modules.rs`), the same call that the new tests make.

### Shared page notes

[Module Development](../Module-Development.md) must change, because a module author can now write `raw` and have it run. The page belongs to the MOD workstream and not to this package, so the coordinator applies the three edits below before the merge. Each edit shows the exact replacement text.

1. In the table of what works today, replace the row

   `| Use the types `tunnel`, `strike`, `evasion`, and `fuzzer`, or the `raw` action | Not supported. The schema accepts the types, and the specification reserves them. | `docs/spec/specification.md`, `tools/contracts/validate.py` |`

   with the two rows

   `| Use the types `tunnel`, `evasion`, and `fuzzer` | Not supported. The schema accepts the types, and the specification reserves them. | `docs/spec/specification.md`, `tools/contracts/validate.py` |`

   `| Write bytes without templating with the `raw` action | Works in a module of type `strike` or `fuzzer`, see [Module types](#module-types). Any other module type fails to load. | `engine/fps-flow/src/check.rs`, `engine/fps-flow/src/program.rs` |`

2. In the section Module types, replace the paragraph that starts with `The schema description names` with the following text. The table row `| `tunnel`, `strike`, `evasion`, `fuzzer` | Reserved | Not supported. |` stays as it is, except that `strike` and `fuzzer` leave it and get the row below it.

   Table rows:

   `| `tunnel`, `evasion` | Reserved | Not supported. |`

   `| `strike`, `fuzzer` | Reserved | Write scenarios that use the `raw` action. The engine runs no other behavior for these types. |`

   Paragraph:

   `The schema description names `tunnel`, `strike`, `evasion`, and `fuzzer` as reserved. The schema accepts them and no reader rejects them, so a manifest of such a type validates and installs. The one rule that depends on the type is the `raw` action, which the validator and the contract check of the engine admit only in `strike` and `fuzzer` modules (`tools/contracts/validate.py`, `RESERVED_ACTION_TYPES`, and `engine/fps-flow/src/check.rs`). A module of any other type that uses `raw`, in a loop or a branch arm as well, fails to load with the error `ReservedAction`, which names the action and the two types that may use it, for example `flow 'a': client: action 2: 'raw' is reserved for module types ["fuzzer", "strike"]`. In a `strike` or `fuzzer` module the engine runs the action: it decodes `hex` once at load time and writes the bytes on the session exactly as given (`engine/fps-flow/src/program.rs`, ENG-29). The bytes are not a template, so `{{` and `}}` in them stay as written, and the engine draws nothing from the session generator for them. On a TCP flow the bytes join the stream, and on a UDP flow they form one datagram of at most 65,507 bytes, as with `send`. The action writes on the session socket of the flow and nothing more. It does not send stateless packets such as a SYN flood, and it does not forge source addresses, so a scenario that needs either of them cannot be written yet.`

3. In the table of actions, replace the row of `raw` with

   `| `raw` | `hex` | Only in `strike` and `fuzzer` modules. Writes the decoded bytes without templating. `hex` is an even number of hexadecimal digits with no spaces, and at least one byte. Any other module type fails to load with `ReservedAction`. |`

The page [Engine](../Engine.md) is in this package and already describes the action, the error, and the tests. The line `A strike scenario with `raw` therefore validates and installs, and the agent refuses to run it.` of the old paragraph is gone in the replacement above, and no other page says that the engine refuses `raw`. `contracts/README.md` needs no edit, because it states only the rule on the module type.
