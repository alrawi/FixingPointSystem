# Engine

This page describes the Rust crates of the agent engine under `engine/`, which the ENG workstream owns. ENG-01 created the first four crates. Later ENG packages fill them and add the scheduler, runtime, extension host, and capture crates that the [Implementation Plan](Implementation-Plan.md#target-repository-layout) lists.

## Crates

The workspace glob in the root `Cargo.toml` picks up every directory under `engine/`, so a new crate needs no edit there. The table lists the crates that exist today.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-io`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-io) | The `SessionIo` trait and the in memory duplex `fps_io::memory::duplex` | ENG-06 adds `DatagramIo` |
| [`fps-flow`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-flow) | The scenario model, the loader, the program compiler, the template compiler of contract v0.2, and the `TemplateFn` dispatch trait | ENG-02 adds the semantic checks |
| [`fps-interp`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-interp) | Empty, with the lint header | ENG-04 ports the interpreter of spike S1 |
| [`fps-stats`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-stats) | Empty, with the lint header | ENG-09 adds statistics collection |

Every crate carries `#![forbid(unsafe_code)]` and the lint header from [Coding Standards](Coding-Standards.md#rust). A `clippy.toml` in each crate with tests lets unit tests call `unwrap`, `expect`, and `panic`, which library code may not. We run the tests of every engine crate with `make test-eng`, and the `rust` job of `ci.yml` runs them on every pull request.

## The SessionIo seam

`SessionIo` is the byte stream seam between the interpreter and the transport backends of the TRN workstream. It extends `AsyncRead`, `AsyncWrite`, `Unpin`, and `Send` with one method, `abort`. We define the semantics that every implementation follows in four rules.

1. `poll_shutdown` closes the write side, which a TCP transport sends as FIN. The peer reads the buffered bytes and then end of stream.
2. `abort` ends the session with a reset. The peer's next read or write fails with `ConnectionReset`, and bytes that the peer had not read yet are lost.
3. After `abort`, every operation on the aborting side fails with `NotConnected`, and a second `abort` has no effect.
4. Dropping a stream without `abort` behaves like a close, so the peer reads end of stream.

The kernel TCP backend implements `abort` with `SO_LINGER` set to zero followed by close, as spike S1 did. That implementation belongs to the TRN backend crate, so `fps-io` depends only on Tokio. `Box<dyn SessionIo>` also implements the trait, so a runtime can hold sessions of several backends in one collection.

Tests use `fps_io::memory::duplex(capacity)`, which returns two connected `MemoryIo` ends that follow all four rules. A capacity of 1 delivers every byte in its own read, which exercises patterns that span reads. [Testing](Testing.md#writing-a-modular-test) shows the duplex in an interpreter test.

## The flow crate

`fps-flow` holds the program model and the template parser of spike S1 with their tests. We changed four things in the port.

1. Errors are the typed enums `LoadError` and `TemplateError` from `thiserror`, in place of `anyhow`. Each `LoadError` names its location from the file inward, for example `web_get.yaml: flow 'page': client: action 3: loop: action 1`. ENG-02 replaces the location string with structured fields.
2. `Flow` records `l4`, so the DNS example over UDP compiles. The interpreter of ENG-04 runs TCP flows only, and ENG-06 adds datagram sessions for UDP flows.
3. Rendering takes any sized `rand::RngCore`, and the filler buffer comes from `rand_pcg::Pcg64Mcg`, which gives the same bytes on every platform. The spike used `SmallRng`, whose algorithm may change between platforms and releases.
4. The contract test `tests/contract_examples.rs` compiles every scenario under `contracts/examples/modules/`, so a new example joins the check on its own.

The crate still rejects transport modules, spawned flows, and the `spawn` and `raw` actions with `LoadError::Unsupported`.

## Templates

ENG-03 replaced the v0.1 template parser with a compiler for the full v0.2 grammar of `contracts/README.md`. The tokenizer and the recursive descent parser in `template/parse.rs` follow `tools/contracts/validate.py` rule for rule, so the engine and the validator accept the same templates. The compiler covers literals of all four kinds, the references `vars.<name>`, `session.id`, `session.flow`, and `last`, the core functions `fill`, `hex`, and `rand_int` with positional and keyword arguments, and module calls with keyword arguments and nested calls up to 8 levels.

A compiled template is a list of segments. A segment is either literal bytes or an expression slot. The compiler binds the arguments of core functions to their parameters and checks their static types with the rules of the contract section on types, so `fill('x')` fails at load time. Module calls keep their positional and keyword arguments in call order, because the extension binds them against its own catalog.

### Constant folding

We fold in two passes, and neither pass changes the rendered bytes or the order of random draws.

1. `Template::compile` folds literals and `hex()` of a literal into the neighboring literal segments. A template without placeholders, or with only such parts, is a single literal and `Template::as_static` returns its bytes.
2. `Scenario::fold_constants` runs once per run after `Scenario::override_vars`. It substitutes every scenario variable that no `set` action binds and folds again. In the example `web_get` the whole client request becomes one literal of 132 bytes.

`fill`, `rand_int`, `last`, the session fields, and module calls never fold, because they differ between sessions or draw from the session generator.

### Module function dispatch

Decision D10 lets first party modules implement template functions natively. The trait `fps_flow::TemplateFn` is the seam between the renderer and module code. `RenderCtx` carries one `&mut dyn TemplateFn` per session, and the renderer calls it for every module call with a `FnCall` that holds the module name, the function name, and `CallArgs`. `ArgValue`, `NamedArg`, and `CallArgs` mirror the `value` variant and the `call-args` record of `contracts/extension/fps-extension.wit`, and `FnErrorKind` mirrors `hook-error-kind`.

```rust
pub trait TemplateFn {
    fn render(&mut self, call: &FnCall<'_>, rng: &mut dyn RngCore, out: &mut Vec<u8>)
        -> Result<(), FnError>;
}
```

The renderer passes the session generator, so module output is reproducible from the run seed and the session id. The interpreter of ENG-04 implements the trait over the session module table, which forwards each call to `ExtensionSession::render` of the named module, and the WebAssembly adapter of ENG-11 plugs in behind the same table. `NoFunctions` serves sessions whose templates call no module. The renderer evaluates the arguments of a call in call order, renders a nested module call into a temporary buffer, and passes it as `bytes`. `Template::module_calls` lists every call site, so the engine can check calls against the function catalog before a run starts.

### Program dump

`Template::dump` and `Scenario::dump` print compiled programs in a stable text form for golden tests. Each segment takes one line, and arguments follow on deeper lines under their parameter names or as `#<index>` for positional module arguments.

```text
1 send
  dynamic, 1 segment
  [0] call http1.response
      #0 = int 200
      body = fill
          n = vars.page_size
```

### Tests

| Test | What it checks |
|---|---|
| `tests/grammar.rs` | Every grammar rule of contract v0.2, with the dump or the error and the bytes that one session renders with seed 42, against `tests/testdata/grammar.seed42.txt` |
| `tests/contract_examples.rs` | Every example scenario compiles, and its dump before and after folding matches `tests/testdata/examples/<module>.<scenario>.dump` |
| `tests/templates.rs` | Property tests that rendering is deterministic for a fixed seed and session id, module calls included, and that folding preserves the rendering |

We rewrite the golden files with `UPDATE_GOLDEN=1 cargo test -p fps-flow` and review every changed file before a commit.

## Known duplication

Spike S1 in `spikes/s1-engine` keeps its own copies of `program.rs` and `template.rs`, so its benchmark harness builds and reproduces the Gate 0 numbers unchanged. The engine crates are the source of truth for new work. A later package may switch the spike to the engine crates or retire it, and until then a fix in one copy does not reach the other.
