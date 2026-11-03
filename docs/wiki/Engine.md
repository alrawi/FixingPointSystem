# Engine

This page describes the Rust crates of the agent engine under `engine/`, which the ENG workstream owns. ENG-01 created the first four crates. Later ENG packages fill them and add the scheduler, runtime, extension host, and capture crates that the [Implementation Plan](Implementation-Plan.md#target-repository-layout) lists.

## Crates

The workspace glob in the root `Cargo.toml` picks up every directory under `engine/`, so a new crate needs no edit there. The table lists the crates that exist today.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-io`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-io) | The `SessionIo` trait and the in memory duplex `fps_io::memory::duplex` | ENG-06 adds `DatagramIo` |
| [`fps-flow`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-flow) | The scenario model, the loader, the program compiler, the template compiler of contract v0.2, and the `TemplateFn` dispatch trait | ENG-02 adds the semantic checks |
| [`fps-interp`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-interp) | Empty, with the lint header | ENG-04 ports the interpreter of spike S1 |
| [`fps-stats`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-stats) | The catalog table, per worker shards, HDR histograms, the snapshot merge, and the proto3 JSON mapping of `CounterSnapshot` | ENG-10 publishes one snapshot per second from the worker runtime |
| [`fps-sched`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-sched) | Load profile, session schedule, drain iterator, and injected clock | ENG-10 drives it from the worker runtime |
| [`fps-hosts`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-hosts) | Host pool address mapping, slicing, and seeded host selection, see [Hosts](Hosts.md) | HST-02 adds kernel port setup |
| [`fps-backend-kernel`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-backend-kernel) | `SessionIo` over kernel TCP sockets, see [Transport](Transport.md) | TRN-02 adds UDP |

Every crate carries `#![forbid(unsafe_code)]` and the lint header, except `fps-backend-kernel`, which carries `#![deny(unsafe_code)]` with one allowed `setsockopt` module that [Transport](Transport.md#unsafe-code) explains. Every crate also carries from [Coding Standards](Coding-Standards.md#rust). A `clippy.toml` in each crate with tests lets unit tests call `unwrap`, `expect`, and `panic`, which library code may not. We run the tests of every engine crate with `make test-eng`, and the `rust` job of `ci.yml` runs them on every pull request.

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

## Statistics collection

`fps-stats` collects the counters, gauges, and HDR histograms of `contracts/metrics/catalog.yaml` per worker and turns them into `CounterSnapshot` messages. ENG-09 added it, and ENG-10 wires it into the worker runtime.

We keep the catalog as the static table `fps_stats::CATALOG` with one constant per metric in `fps_stats::metric`, for example `metric::SESSIONS_ATTEMPTED`. The contract test `tests/catalog.rs` reads the YAML file and compares every name, type, unit, label list, and enumerated label value, so a catalog change fails the test until the table follows it.

Each worker owns one `Shard`. The worker registers a series once with `counter`, `gauge`, or `histogram`, which checks the metric type and the labels against the catalog and returns a handle. Recording through a handle with `inc`, `add`, `set`, `adjust`, or `record` indexes a vector that only the owning worker touches, so the hot path takes no lock and uses no atomic instruction. We apply four rules to recorded values.

1. Counters saturate at `u64::MAX`.
2. Histograms keep three significant digits in the catalog unit and record values above one hour in microseconds as that value. A histogram grows when a value exceeds every earlier value of its series, which allocates at most a few dozen times over its life.
3. A shard records only gauges that workers can sum, such as `sessions.concurrent`. Ratios and percentages such as `sessions.failed_ratio` are derived after the merge, and their registration fails with `StatsError::NotAdditive`.
4. A key may omit catalog labels, which aggregates over them. An unknown label, a repeated label, or a value outside the enumerated values fails the registration.

Once per snapshot interval the worker calls `Shard::snapshot` and sends the cumulative `Snapshot` to the collector. The collector keeps the newest snapshot of every worker and merges them with `Snapshot::merge_all`, so a dropped snapshot loses no data. Counters, gauges, and histograms of the same series add up.

`Snapshot::to_proto_json` serializes a snapshot with a `SnapshotMeta` to the proto3 JSON mapping of `CounterSnapshot` in `contracts/proto/fps/agent/v1/agent.proto`. We follow the canonical printer, so field names are lower camel case, 64 bit integers are strings, `hdrV2` is base64 of the compressed HdrHistogram V2 encoding, gauges that JSON numbers cannot hold become `NaN` or `Infinity`, and fields with default values are omitted. An aggregate snapshot sets `aggregate`, and a worker snapshot sets `workerId`. Series appear in catalog order and then in label order, and each histogram is copied into bounds that depend only on its largest value before encoding, so equal values always give equal bytes. The contract test `tests/proto_json.rs` reads the proto file and checks every emitted key against the fields of `CounterSnapshot`, `CounterValue`, and `HistogramValue`.

The component test `tests/concurrency.rs` runs eight worker threads that record 200000 events each and send snapshots to a collector thread while they run. It checks that the aggregate never decreases and that the final aggregate holds every increment and every histogram sample. The property test `tests/merge_props.rs` checks that any split of an event stream over up to eight shards merges to the same JSON as one shard, in either merge order.

## The session scheduler

ENG-08 added the crate [`fps-sched`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-sched), which turns one worker rate of a plan into session starts. A `Schedule` serves one mix entry on one worker, and its scheduled times are a pure function of the configuration, the run seed, and the stream, so a replay reproduces every start.

The crate has four parts, which are listed below.

| Item | Contents |
|---|---|
| `LoadProfile` | The validated `LoadPhase` list with the fraction at a time, its integral, and the inverse of the integral |
| `Schedule` | Session starts for a `SessionsPerSecond` or a `ConcurrentSessions` objective, with a concurrency cap |
| `Drain` | The iterator that `Schedule::drain(&clock, open)` returns for one clock reading |
| `Clock` and `ManualClock` | The injected clock, which reports the time since the run start |

A worker calls `drain` with its clock and its open session count, starts every session that the drain yields, and sleeps until `next_due` returns a time. When `next_due` returns `None`, only a closing session can release the next start. The crate never reads the wall clock, so the runtime of ENG-10 backs the clock with its timer and tests use `ManualClock`.

### Rate objectives

A rate objective places starts with the time change method. The schedule draws the arrivals of a unit rate process in operational time and maps each one back through the integral of the target rate. Constant arrivals place start `k` where the expected count reaches `k + 0.5`, so every phase holds its expected count rounded. Poisson arrivals add exponential gaps with mean 1 in operational time, which gives a nonhomogeneous Poisson process whose intensity follows the phases. A linear ramp from 0 to 100 sessions per second over 10 seconds therefore holds exactly 500 constant starts, of which 125 fall in the first 5 seconds.

At the concurrency cap the default policy `CapPolicy::Defer` holds an arrival until a session closes, and the start keeps its scheduled time so that the caller can measure the delay. The policy `CapPolicy::Skip` drops the arrival and counts it in `skipped`.

### Concurrency objectives

A concurrency objective starts a session whenever the open sessions fall below the target, which is the floor of the steady target times the current fraction, bounded by the cap. Such a start carries the drain time as its scheduled time. `next_due` returns the first time at which the target exceeds the open sessions, or `None` while the target is met.

### Phase shapes

We follow the `PhaseShape` comments of `agent.proto`. A hold phase keeps `start_fraction`, and a linear phase moves from `start_fraction` to `end_fraction`. Step `i` of a step phase with `n` steps, counting from 0, holds `start + (end - start) * i / n`, so the phase begins at the start fraction and the end fraction takes effect where the phase ends. The five step ramp down of the example plan therefore runs at 100, 80, 60, 40, and 20 percent.

### Seeds and streams

Each schedule seeds `Pcg64Mcg` from the run seed and a stream, and `stream_id(worker_id, mix_index)` gives every worker rate its own stream. `Pcg64Mcg` forces the two lowest state bits to 1, so the crate keeps the stream above those bits and mixes the seed into the high half with SplitMix64.

### Tests

The tests of the crate cover the four levels that the ENG workstream requires except performance, which ENG-10 measures with the runtime. They are listed below.

1. Two schedules with the same seed are identical, other seeds and streams differ, and a drain in 1 ms ticks or at every `next_due` time yields the same starts as one drain at the end.
2. A two sided KS test at alpha 0.05 accepts the Poisson inter arrival times at a constant rate and the rescaled gaps during a linear ramp, and it rejects constant arrivals.
3. Phase boundary tests count the starts of ramp up, steady, and ramp down, step ramps up and down, and the concurrency targets at each phase.
4. Property tests check the inverse of the integral, ordering, bounds, reproducibility, and the constant arrival count over generated profiles.
5. Contract tests parse every shape name of `test-config.schema.json` and schedule the phases of `examples/plans/http_dns_smoke.json`.
