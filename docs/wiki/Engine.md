# Engine

This page describes the Rust crates of the agent engine under `engine/`, which the ENG workstream owns. ENG-01 created the first four crates. Later ENG packages fill them and add the scheduler, runtime, extension host, and capture crates that the [Implementation Plan](Implementation-Plan.md#target-repository-layout) lists.

## Crates

The workspace glob in the root `Cargo.toml` picks up every directory under `engine/`, so a new crate needs no edit there. The table lists the crates that exist today.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-io`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-io) | The `SessionIo` trait, the in memory duplex `fps_io::memory::duplex`, and the `DatagramIo` seam with the in memory `datagram_pair`, see [UDP datagram sessions](#udp-datagram-sessions) | Kernel UDP since TRN-02 |
| [`fps-flow`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-flow) | The scenario model, the loader, the program compiler, the template compiler of contract v0.3, and the `TemplateFn` dispatch trait | ENG-02 adds the semantic checks |
| [`fps-interp`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-interp) | The session interpreter, the `Opener` seam, `session_rng`, delay sampling, and failure reasons, see [The flow program interpreter](#the-flow-program-interpreter) | UDP sessions since ENG-06 and vectored fill writes since ENG-05, see [Vectored writes for fill bodies](#vectored-writes-for-fill-bodies). ENG-07 adds multi flow instances |
| [`fps-stats`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-stats) | The catalog table, per worker shards, HDR histograms, the snapshot merge, and the proto3 JSON mapping of `CounterSnapshot`, plus the typed `SessionCounters` and `TransportCounters` sets with the `side` label and the failure reasons of ENG-14 | ENG-10 publishes one snapshot per second from the worker runtime |
| [`fps-runtime`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-runtime) | Worker threads, optional core pinning, the Tokio clock of the scheduler, statistics snapshots, stop and abort, the `RateControl` of the client workers, see [Run time rates](#run-time-rates), and scenario instances with several flows, see [Multi flow scenario instances](#multi-flow-scenario-instances) | AGT-01 builds a `RuntimeConfig` from the agent plan, and AGT-05 drives `RateControl` |
| [`fps-sched`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-sched) | Load profile, session schedule, drain iterator, and injected clock | The worker runtime drives it, and AGT-05 routes `UpdateRates` to the `RateControl` that `RuntimeHandle::rate_control` returns |
| [`fps-hosts`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-hosts) | Host pool address mapping, slicing, and seeded host selection, see [Hosts](Hosts.md) | HST-02 adds kernel port setup |
| [`fps-backend-kernel`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-backend-kernel) | `SessionIo` over kernel TCP sockets, see [Transport](Transport.md) | TRN-02 adds UDP |
| [`fps-wasm`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-wasm) | The WebAssembly extension host, with fuel and memory limits, see [The WebAssembly extension host](#the-webassembly-extension-host) | `fps-runtime` loads it into sessions through an `ExtensionSet`, see [WebAssembly extensions in sessions](#webassembly-extensions-in-sessions) (ENG-17) |
| [`fps-capture`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-capture) | The seeded session sampler, the tap wrappers, frame synthesis, and the bounded pcapng ring with TLS key logs, see [Session sampling and capture](#session-sampling-and-capture) | AGT-05 serves `StartCapture` and `StopCapture` with it, and `fps-runtime` taps its TCP sessions through the `CaptureCell` of the crate (ENG-19) |

Every crate carries `#![forbid(unsafe_code)]` and the lint header, except `fps-backend-kernel`, which carries `#![deny(unsafe_code)]` with one allowed socket module that [Transport](Transport.md#unsafe-code) explains. The lint header comes from [Coding Standards](Coding-Standards.md#rust). A `clippy.toml` in each crate with tests lets unit tests call `unwrap`, `expect`, and `panic`, which library code may not. We run the tests of every engine crate with `make test-eng`, and the `rust` job of `ci.yml` runs them on every pull request.

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

1. Errors are the typed enums `LoadError` and `TemplateError` from `thiserror`, in place of `anyhow`. Each `LoadError` carries a typed `Location`, which [The scenario loader](#the-scenario-loader) describes.
2. `Flow` records `l4`, so the DNS example over UDP compiles. The interpreter of ENG-04 runs TCP flows only, and ENG-06 adds datagram sessions for UDP flows.
3. Rendering takes any sized `rand::RngCore`, and the filler buffer comes from `rand_pcg::Pcg64Mcg`, which gives the same bytes on every platform. The spike used `SmallRng`, whose algorithm may change between platforms and releases.
4. The contract test `tests/contract_examples.rs` compiles every scenario under `contracts/examples/modules/`, so a new example joins the check on its own.

`Scenario::load` desugars the flow keyword `transport: {module: tls, ...}` into a leading `tls_upgrade` on each side since CON-18, and still rejects spawned flows and the `spawn` and `raw` actions with `LoadError::Unsupported` (`Scenario::parse_instance` accepts spawn for the multi flow instances of ENG-23), while `Scenario::check` accepts them when the contract allows them. The action `tls_upgrade` of ENG-20 compiles in every mode to `Op::UpgradeTls`, see [TLS upgrade of a running session](#tls-upgrade-of-a-running-session).

## Templates

ENG-03 replaced the v0.1 template parser with a compiler for the full v0.2 grammar of `contracts/README.md`, and ENG-16 extended it to v0.3, see [Profiles and distributions](#profiles-and-distributions). The tokenizer and the recursive descent parser in `template/parse.rs` follow `tools/contracts/validate.py` rule for rule, so the engine and the validator accept the same templates. The compiler covers literals of all four kinds, the references `vars.<name>`, `session.id`, `session.flow`, and `last`, the core functions `fill`, `hex`, `rand_int`, and since ENG-25 `int_bytes` and `now` with positional and keyword arguments, and module calls with keyword arguments and nested calls up to 8 levels.

A compiled template is a list of segments. A segment is either literal bytes or an expression slot. The compiler binds the arguments of core functions to their parameters and checks their static types with the rules of the contract section on types, so `fill('x')` fails at load time. Module calls keep their positional and keyword arguments in call order, because the extension binds them against its own catalog.

### Constant folding

We fold in two passes, and neither pass changes the rendered bytes or the order of random draws.

1. `Template::compile` folds literals and `hex()` of a literal into the neighboring literal segments. A template without placeholders, or with only such parts, is a single literal and `Template::as_static` returns its bytes.
2. `Scenario::fold_constants` runs once per run after `Scenario::override_vars`. It substitutes every scenario variable that no `set` action binds and folds again. In the example `web_get` the whole client request becomes one literal of 132 bytes.

`fill`, `rand_int`, `now`, `last`, the session fields, and module calls never fold, because they differ between sessions, draw from the session generator, or read the clock. `int_bytes` folds when its arguments are known and valid, and an invalid known argument stays in the template and fails when rendered.

### Profiles and distributions

ENG-16 added the two roots of scenario contract v0.3 (CON-10) to the parser, the checker, and the renderer.

`profiles.<name>` has type `text`. The loader reads the top level `profiles` object of the scenario into `fps_flow::Profiles` and folds each reference into the text that it renders, so a compiled scenario carries no profile lookup and the interpreter needs no change. The text holds each entry as `Name: value` and CRLF in declaration order, with a number or boolean in its `Display` form. `Template::fold_profiles` does the substitution, `Scenario::parse` calls it for every template of every flow after the flows compile, and a template that `Template::compile` built on its own and that nobody folded fails to render with `UndefinedProfile`. The loader rejects a profile without entries, an entry name outside `[A-Za-z0-9][A-Za-z0-9_-]*`, a nested value, and a string with CR or LF, as the schema does, and the check reports a reference to a profile that the scenario does not declare.

`dist.pareto(alpha, scale, cap)`, `dist.lognormal(mu, sigma, cap)`, `dist.exponential(mean, cap)`, and `dist.uniform(low, high)` return an `int`. The code lives in `template/dist.rs` and uses no distribution crate.

| Function | Draw | Generator words |
|---|---|---|
| `pareto` | `scale * u^(-1/alpha)` with `u` in (0, 1], the inverse transform | 1 |
| `lognormal` | `exp(mu + sigma * z)`, where `z` comes from the Box-Muller transform and the second normal is discarded | 2 |
| `exponential` | `-mean * ln(u)` | 1 |
| `uniform` | `low + (high - low) * r` with `r` in [0, 1) | 1 |

The draw is rounded to the nearest integer and limited to `cap`, which defaults to 2,147,483,647 (`DEFAULT_CAP`). `uniform` has no `cap`, and its draw never leaves `[low, high]` before rounding. An infinite draw, which a tiny `alpha` can cause, lands on the cap. Every call consumes the same number of generator words whatever its arguments, so a seeded run repeats and two runs that differ only in the arguments stay in step. The parameters take `float` and bind an `int` too, and a variable of type `text` that parses as a number binds as well, as `fill` already accepts a numeric string.

The ranges of the contract hold at two points. A literal argument is checked when the template compiles (`DistParameter`, `UniformRange`), so the loader rejects `dist.exponential(0)`. An argument that a variable or a call computes is checked on each render, and the session fails with the same errors. `alpha`, `scale`, `sigma`, `mean`, and `cap` must be positive, `low` and `high` must not be negative, `low` must not exceed `high`, and `mu` must be finite. The existing object form `delay: { dist: lognormal, ... }` stays as it was, and `delay: { ms: "{{ dist.lognormal(3.0, 0.8) }}" }` is the template form.

### The functions `int_bytes` and `now`

ENG-25 added the two core functions of scenario contract v0.6 (CON-19). Both draw nothing from the session generator, so a template that adds one leaves every other drawn value of the session unchanged.

`int_bytes(value, width, endianness='big')` returns exactly `width` bytes. A literal argument that breaks a rule is a template error at load time, as the validator reports it: a width outside 1 to 8, an endianness other than `'big'` or `'little'`, and a negative value or one that does not fit the width. An argument from a variable or another call fails the session when rendered, with reason `template_error`, and the engine never truncates, wraps, or encodes a negative value. A width of 8 takes 0 to 2^63 - 1.

`now(format)` takes `'http_date'` (text, the 29 byte IMF fixdate in UTC), `'epoch_s'` (int, truncated toward zero), or `'epoch_ms'` (int). A literal format fixes the result type, so `fill(now('epoch_s'))` compiles and `fill(now('http_date'))` does not, and a format from a variable gives type `any`. `http_date` needs a year from 1 to 9999, and the clock outside that range fails the session when rendered. Each evaluation reads the clock, so a scenario that needs one instant in several places binds it with `set`.

The clock is a seam next to the random source. `fps_flow::Clock` has one method, `now_ms`, and `RenderCtx` carries a `&dyn Clock`. `SystemClock` reads the wall clock of the agent in UTC and is the default. `PinnedClock::new(ms)` returns one fixed instant, and `.with_step_ms(step)` advances it by a fixed step per call. A session takes a clock with `Session::set_clock`, and an instance gives the same clock to all of its sides with `ScenarioInstance::set_clock`, which a rerun keeps. `SessionCtx` and `InstanceConfig` did not change, so the crates that build them (`fps-runtime`, `fps-agent`, `fps-backend-kernel`) compile as before and run on the wall clock. A harness, golden, or replay that needs equal bytes pins the clock and records the instant, and a golden that does not pin it must not contain output of `now`.

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
| `tests/profiles.rs` | `web_profile.yaml` loads and checks, a loaded scenario renders its profile and draws capped delays, and the loader rejects undeclared profiles, bad `dist` calls, and malformed profiles |
| `src/template/tests_dist.rs` | Each `dist` function (sample mean or median over 200,000 draws within 3%, minimum and bounds), the cap and the default cap, seeded repeatability, fixed generator words per call, argument binding and range errors at compile and render time, profile rendering order and CRLF, and the dump |
| `tests/templates.rs` | Property tests that rendering is deterministic for a fixed seed and session id, module calls included, and that folding preserves the rendering |
| `src/template/tests_core.rs`, `src/clock.rs` | `int_bytes` for every width from 1 to 8 in both byte orders (zero, one, an asymmetric pattern, and the largest value of the width), the 2^63 - 1 limit of width 8, the error cases (width, endianness, negative value, value that does not fit, wrong types, call shape) at compile time for literals and at render time otherwise, folding, and a round trip property; `now` in the three formats under a pinned clock, truncation toward zero, the output lengths from 2001 to 2286, the year range, one clock read per evaluation, the formats and the result types, and that `now` never folds; the date arithmetic of `http_date` |

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

### Side label and failure reasons

Catalog 0.2.0 (CON-08) gives every scenario level and transport metric the `side` label with the values `client` and `server`, and it adds the failure reasons `datagram_rejected`, `template_error`, and `extension_error`. ENG-14 follows it in `fps-stats` with three additions in `src/series.rs`.

1. `Side` and `FailureReason` name the label values as types, each with `ALL` in catalog order and `as_str`. The contract test `tests/series.rs` compares both with `contracts/metrics/catalog.yaml`.
2. `SessionCounters::register(shard, application, scenario, side)` registers `sessions.attempted`, `sessions.established`, `sessions.succeeded`, `bytes.tx`, `bytes.rx`, one `sessions.failed` counter for each of the 12 reasons, the `sessions.concurrent` gauge, and the `sessions.duration` histogram, all with the `side` label. Its methods `attempted`, `established`, `succeeded`, `failed(reason)`, and `add_bytes` record through the handles. ENG-15 added `started`, which counts an attempt and raises the gauge, and `ended(duration_us)`, which lowers the gauge and records the duration.
3. `TransportCounters::register(shard, port, side)` registers the plain `tcp.*` and `tls.*` counters with the `side` label. The alert counters carry the extra `alert` label, so a caller registers them with `Shard::counter`.

The `TransportCounters` set covers counters only. A total over both sides counts each session twice, so a reader that sums `sessions.failed` without the `side` label gets double counts when both sides run in one plan. The unit tests in `tests/series.rs` record each new reason on both sides, and they check the merged snapshot and the proto3 JSON.

ENG-15 closed two of the follow ups of ENG-14. `FailureKind::reason` in `fps-interp` now returns the three new reasons, and `fps-runtime` registers its series through `SessionCounters`. The UDP path of `fps-agent` still has its own registration without `side`, which AGT-07 changes.

## The session scheduler

ENG-08 added the crate [`fps-sched`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-sched), which turns one worker rate of a plan into session starts. A `Schedule` serves one mix entry on one worker, and its scheduled times are a pure function of the configuration, the run seed, and the stream, so a replay reproduces every start.

The crate has six parts, which are listed below.

| Item | Contents |
|---|---|
| `LoadProfile` | The validated `LoadPhase` list with the fraction at a time, its integral, and the inverse of the integral |
| `Schedule` | Session starts for a `SessionsPerSecond` or a `ConcurrentSessions` objective, with a concurrency cap |
| `Drain` | The iterator that `Schedule::drain(&clock, open)` returns for one clock reading |
| `Clock` and `ManualClock` | The injected clock, which reports the time since the run start |
| `RateHandle` | A thread safe handle on the target of one schedule, which a control thread sets while the worker runs (ENG-13) |
| `RateControl` | The registry that maps a scenario id to the handles of its workers, with all or nothing updates (ENG-13) |

A worker calls `drain` with its clock and its open session count, starts every session that the drain yields, and sleeps until `next_due` returns a time. When `next_due` returns `None`, only a closing session can release the next start. The crate never reads the wall clock, so the runtime of ENG-10 backs the clock with its timer and tests use `ManualClock`.

### Rate objectives

A rate objective places starts with the time change method. The schedule draws the arrivals of a unit rate process in operational time and maps each one back through the integral of the target rate. Constant arrivals place start `k` where the expected count reaches `k + 0.5`, so every phase holds its expected count rounded. Poisson arrivals add exponential gaps with mean 1 in operational time, which gives a nonhomogeneous Poisson process whose intensity follows the phases. A linear ramp from 0 to 100 sessions per second over 10 seconds therefore holds exactly 500 constant starts, of which 125 fall in the first 5 seconds.

At the concurrency cap the default policy `CapPolicy::Defer` holds an arrival until a session closes, and the start keeps its scheduled time so that the caller can measure the delay. The policy `CapPolicy::Skip` drops the arrival and counts it in `skipped`.

### Concurrency objectives

A concurrency objective starts a session whenever the open sessions fall below the target, which is the floor of the steady target times the current fraction, bounded by the cap. Such a start carries the drain time as its scheduled time. `next_due` returns the first time at which the target exceeds the open sessions, or `None` while the target is met.

### Run time rate adjustment

ENG-13 lets the agent change the rate of a running schedule without restarting a worker. `Schedule::rate_handle()` returns a `RateHandle`, a clone of one atomic cell that holds the target as the bits of an `f64`. Any thread may call `set`, and the worker loads the cell at every `drain`, which is its scheduling tick. The worker takes no lock, and a single atomic word cannot tear. A handle keeps the kind of its objective, so a rate handle refuses a concurrency target and the reverse with `KindMismatch`, and `set` refuses a rate that is negative or not finite with `InvalidRate`.

The change applies at the first tick that sees it, and `next_due` returns a time that is not after the clock while a change waits, so a worker that re reads it ticks at once. A worker that sleeps with no due time, which happens while the schedule is paused or at the cap, has to wake by itself to see a change, and ENG-10 and AGT-05 should bound that sleep to a tick of about 100 ms.

The arrival process stays correct across a change because the schedule keeps the operational time of the pending arrival and changes only the map from operational time to time. An arrival at operational time `op` lies where the integral of the profile reaches `mass_base + (op - op_base) / rate`. A change at time `t` sets `op_base` to the expected count at `t` under the old rate and `mass_base` to the integral at `t`, so the expected count is continuous. The next start keeps its place in the count and moves to where the new rate reaches it. Constant arrivals therefore keep every gap between the old and the new spacing, which leaves no burst that catches up and no gap, and Poisson arrivals stay a Poisson process with the new intensity.

Four behaviours are defined.

| Case | Behaviour |
|---|---|
| Rate 0 | The schedule pauses. It yields no start, `is_finished` stays false, and `next_due` is `None`. The expected count stands still, so a resume does not owe the paused time. A concurrency target of 0 starts nothing and leaves the open sessions to the worker |
| Arrival already due | An arrival that was due before the tick under the old rate goes out first, and the new rate applies from the first tick at which none is due. Under `CapPolicy::Defer` at the cap, a change therefore waits until the deferred arrivals have started |
| Rate raised near the end | The profile ends at a fixed time. A raised rate can bring an arrival inside the profile that the old rate left beyond it, and a lowered rate can push one out |
| Unknown scenario id | `RateControl::update` and `scale` return `UnknownScenario` and change nothing |

`RateControl` is the registry for the agent. The AGT-05 handler of `UpdateRates` registers the handle of every worker rate under its mix entry name and calls `update`. `update` takes the new target of the whole agent per scenario, as CON-05 defines it, and divides it among the workers in proportion to their current targets, or equally when all are 0. A concurrency total divides into whole sessions that add up to the total. `update` is applied completely or not at all, because it checks every id, kind, and value before it stores anything, and it rejects an id that it names twice. A scenario that it does not name keeps its targets. The workers apply their shares on their own ticks, so one update can take effect on slightly different ticks on different workers. `scale` takes one factor per scenario, as the calibration of CTL-21 produces them, and sets every handle to its registered target times the factor. The factor applies to the registered target and not to the current one, so a repeated factor changes nothing and a factor of 1 restores the compiled rate.

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
6. `tests/rate.rs` covers the rate adjustment on a simulated clock without a sleep. A change takes effect at the next tick. Two changes realize the new rates within 5% for Poisson arrivals, over windows of 400 seconds, and within 1% for constant arrivals. Constant arrivals keep every gap between the old and the new spacing, a KS test accepts the rescaled Poisson gaps across two changes, and a property test checks the count and the gaps over generated rates. Pause and resume, a raised rate near the end, the concurrency target, an unknown id, an invalid update, and the proportional split have their own tests. Four threads that call `set` while a worker drains, and four threads that call `update` while a reader checks the registry, prove the thread safety.

## The scenario loader

ENG-02 brought the loader of `fps-flow` to parity with the semantic rules of `tools/contracts/validate.py`. Loading a scenario now runs two passes. The first pass in `check.rs` walks the YAML document and applies the rules of `check_scenario` in the same order, and the second pass compiles the programs as before. Both validators therefore give every scenario the same verdict, and a shared fixture set proves it.

### Entry points

| Function | Module calls | Features the engine does not run yet |
|---|---|---|
| `Scenario::load(path)` and `Scenario::parse(text)` | Unchecked | Rejected with `LoadError::Unsupported` |
| `Scenario::load_with(path, ctx)` and `Scenario::parse_with(text, ctx)` | Checked when `ctx` is given | Rejected with `LoadError::Unsupported` |
| `Scenario::check(text, ctx)` and `Scenario::check_file(path, ctx)` | Checked when `ctx` is given | Accepted, and the warnings are returned |

The context is a `ModuleContext`, which `ModuleSet::load_dir(root).context(name)` builds from the manifests under a modules directory. It holds the module that ships the scenario, its type, and the function catalog of the module itself and of every required module. A required module without an installed manifest keeps its calls unchecked, and the check reports a warning, as the Python validator does.

### Typed locations

Every `LoadError` variant except `Read` carries a boxed `Location` with the file, the variable, the flow index and id, the side, the path of action steps through loops and branch arms, and the field. `Location::pointer` renders the JSON pointer that the Python validator prints, for example `/flows/0/client/1/loop/do/0/spawn`, and the `Display` form counts actions from 1, for example `last_after_loop.yaml: flow 'a': client: action 3`. `LoadError::kind` names the variant, so tests compare error kinds without matching on messages.

### Rules

The loader rejects a scenario for each rule below. The rules on templates use the declared variable types, where a variable that only a `set` action binds has type `any`.

| Rule | Variant |
|---|---|
| Two flows share an id | `DuplicateFlow` |
| An `after` entry names the flow itself, an unknown flow, or closes a cycle | `SelfDependency`, `UnknownFlow`, `Cycle` |
| A flow module is neither the owner nor a required module | `ForeignFlowModule` |
| A side does not open with `connect` or `accept` | `MissingOpener` |
| A `spawn` names an unknown flow or a flow without `spawned: true` | `UnknownSpawnTarget`, `NotSpawned` |
| Branch probabilities do not sum to 1 | `BranchSum` |
| A uniform delay has `min_ms` above `max_ms` | `UniformRange` |
| `raw` appears outside a strike or fuzzer module | `ReservedAction` |
| `max_datagram` on TCP, or a UDP expect that cannot fit one datagram | `MaxDatagramOnTcp`, `DatagramLimit` |
| `tls_upgrade` in a UDP flow, at any depth of `loop` and `branch` | `TlsUpgradeOnUdp` |
| `last` where no `expect` always ran earlier on the same side | `LastWithoutExpect` |
| A call names a foreign module or a function that the manifest lacks | `ForeignModule`, `UnknownModuleFunction` |
| An undefined variable or profile, or a call with a wrong arity, parameter, or argument type, or a `dist` literal out of range | `Template` with the matching `TemplateError` |
| A profile with no entry, a bad entry name, a nested value, or a line break in a string | `Invalid` |

The rule on `last` follows the Python validator. A loop body counts only when its count is a positive integer literal, and a branch counts only when every arm runs an `expect`. `Template::check` holds the typed template rules and reads the catalogs through `CheckScope`, and `manifest.rs` reads the `functions` catalog of a manifest.

### Shared fixtures

The fixtures live in `engine/fps-flow/tests/fixtures/`. Four fixture modules hold 61 scenarios, 16 accepted and 45 rejected, and `verdicts.yaml` lists each scenario with its verdict, the Rust error kind, and the JSON pointer of the first error. The Rust test `tests/fixtures.rs` checks every verdict, kind, and pointer with `Scenario::check_file`. The Python test `tests/test_fixtures.py` runs the module and scenario checks of `validate.py` over the same modules and checks every verdict and pointer. The workflow `eng.yml` runs the Python half, and the rust job of `ci.yml` runs the Rust half. A new rule therefore needs one fixture and one line in `verdicts.yaml`, and both tests pick it up.

### Limits

The Rust loader does not run the JSON Schema step of the Python validator. Its compiler catches most structural errors, but some schema rules, such as identifier patterns and a `set` value that is not a string, still pass in Rust. The fixtures therefore cover only schema valid scenarios. The Rust loader stops at the first error, while the Python validator reports every finding.

## The flow program interpreter

ENG-04 filled the crate [`fps-interp`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-interp), which runs one side of one TCP session from a compiled `fps_flow::program::Program`. A `Session` holds the read, write, and scratch buffers of the side, its generator, its `set` overlay, and its `last` buffer, and it reuses them for every action. Spike S1 was the prototype, and we kept its two buffer lessons. Reads grow the read buffer in 16 KiB steps, and `expect bytes` discards payload through a 64 KiB scratch buffer, so a bulk body never grows the read buffer.

### Running a side

A side runs in two steps, which `Session::run_program` combines.

1. `Session::open` renders the port and the optional host of `connect` or `accept` and passes an `OpenRequest` to an `Opener`. A transport backend implements `Opener` over its sockets, and tests hand out ends of `fps_io::memory::duplex`.
2. `Session::run` executes `send`, the three `expect` stop rules, `delay`, `set`, `loop`, `branch`, and the three `close` modes over the open `SessionIo`.

Both steps take a `stop` future. When it completes first, the session aborts the transport with a reset and fails with reason `aborted`, so a stopping run ends every session at once. `send` flushes after each write, because a TLS layer sends buffered records only on flush. `close: fin` shuts down the write side and drains until the peer closes, and a reset after our FIN still counts as a clean close. `Limits` holds the values that a flow program does not set, which are the open timeout of 5 seconds, the drain timeout of 10 seconds, the pattern buffer limit of 16 MiB, and the `last` limit of 16 MiB.

The session copies consumed bytes into `last` only when a template of the program refers to `last`, which `Template::uses_last` tells us. `RenderCtx` receives that buffer and the session's `&mut dyn TemplateFn`, and `fps_flow::function::NoFunctions` serves sessions without modules.

### Seeds and time

`session_rng(run_seed, session_id, side)` returns the `Pcg64Mcg` generator of one side. `Pcg64Mcg::new` forces the two lowest state bits to 1, so we put a SplitMix64 mix of the session id and the seed into the high half and a mix of the seed and a side constant into the low half. Distinct session ids of one run therefore never share a stream, which fixes the collision of bug #90 for the interpreter, and the regression test checks 100000 consecutive ids. Templates, branches, and delays all draw from this one generator in program order.

Delays and timeouts use the Tokio clock, so tests pause it with `start_paused` and run multi second programs in milliseconds. `fps_interp::delay::sample` draws fixed, uniform, exponential, lognormal, and Pareto think times with `rand_distr` and caps every sample at one hour. Parameters that a distribution rejects fail the session with reason `internal`.

### Failure reasons

Every failure is a `Failure` with the flow id, the side, the catalog action index, the action type, a `Reason`, and a `FailureKind` that keeps the cause. The action index counts actions in document order with the opening action at 0, as the `action_index` label of the catalog defines it, so the compiled `LoopEnd` and `Jump` ops do not shift it. The table maps each reason to its trigger.

| Reason | Trigger |
|---|---|
| `connect_timeout` | The opener did not finish within the open timeout, or it reported `TimedOut` |
| `connect_refused` | The opener reported `ConnectionRefused` |
| `expect_timeout` | An `expect` did not finish within its `timeout_ms`, or the peer did not close within the drain timeout of `close: fin` |
| `pattern_mismatch` | `expect pattern` buffered more unmatched bytes than the pattern buffer limit |
| `datagram_rejected` | A received UDP datagram is longer than the datagram limit (`FailureKind::DatagramTooLarge`) or breaks the stop rule of its `expect` (`FailureKind::DatagramMismatch`) |
| `unexpected_reset` | A read or write met `ConnectionReset` or `ConnectionAborted` outside the drain of `close: fin` |
| `peer_closed` | An `expect` read end of stream, or a write met `BrokenPipe` |
| `tls_failure` | The transport reported `InvalidData`, which is how `tokio-rustls` reports TLS errors |
| `aborted` | The `stop` future completed |
| `template_error` | A template failed to render for a cause of its own, such as `fill()` with a negative size or an undefined variable |
| `extension_error` | A module function failed during a render, whatever its `FnErrorKind` and whether it is native or runs in the WebAssembly host of `fps-wasm`, which returns its failures to the renderer as `FnError` |
| `internal` | A rendered count, port, or delay was out of range, or the transport reported another error |

`FailureKind::reason` reads the cause of `FailureKind::Render`. A `TemplateError::Function` means `extension_error`, and every other `TemplateError` means `template_error`. A failed module call also keeps its kind, and `Failure::extension_label` returns the label `extension_<kind>` that `contracts/extension/README.md` names. The contract test `tests/catalog.rs` reads `contracts/metrics/catalog.yaml` and checks that `Reason::ALL` equals the reason values in catalog order.

### TLS upgrade of a running session

A flow binds its transport once, at `connect` or `accept`, so STARTTLS in SMTP, IMAP, POP3, FTP, and LDAP could not be written (finding of MOD-07). ENG-20 adds the action `tls_upgrade`, which wraps the transport of the running session in TLS in place, so every later action runs over TLS.

```yaml
- tls_upgrade: { profile: mail, server_name: mx.example.test }
```

| Field | Meaning |
|---|---|
| `profile` | Required. The name of a TLS profile of the run, which the engine resolves, see [The worker runtime](#tls-upgrade-in-the-runtime) |
| `role` | `client` or `server`. The default is the role of the side, so the client side starts the handshake and the server side accepts it |
| `server_name` | The name that a client handshake verifies and sends as SNI. The engine chooses when it is missing |

`fps-flow` compiles the action to `Op::UpgradeTls(TlsUpgrade)`. The interpreter has no TLS code. It calls a `TlsUpgrader`, a seam in [`engine/fps-interp/src/upgrade.rs`](https://github.com/alrawi/FixingPointSystem/blob/main/engine/fps-interp/src/upgrade.rs) that a runner sets with `Session::set_tls_upgrader`, like the spawn queue. A program that holds a `tls_upgrade` runs over a `Swappable` transport, a boxed `SessionIo` that the upgrade replaces by the secured transport, and every other program runs over its stream as before, so it pays nothing. The upgrader consumes the old transport and returns the new one with the old one below it. A capture tap on the old transport therefore keeps recording what crosses the wire, which is now encrypted.

The interpreter checks three things before it calls the upgrader. The session must have an upgrader, and the read buffer must be empty, because bytes that the peer sent before the handshake would be plaintext in front of a ClientHello, which fails the session rather than hiding them. The flow must be a TCP flow, because the engine has no DTLS. The handshake runs under the open timeout of the session and under the `stop` future, which drops the transport. The failures map to the existing reasons.

| Cause | Reason |
|---|---|
| The handshake fails, the peer sends an alert, the transport breaks during the handshake, or the handshake times out | `tls_failure` |
| The session has no upgrader, the upgrader has no profile of that name for the role, bytes are buffered, or the flow is UDP | `internal` |

The action type of both is `tls_upgrade`, and its action index is the position of the action in the document as for every action.

`Scenario::parse` and `Scenario::parse_instance` accept the action in every mode. CON-17 added the action to the scenario schema, and ENG-22 made the semantic check agree with `tools/contracts/validate.py`: `check_action` raises `LoadError::TlsUpgradeOnUdp` ("tls_upgrade applies only to TCP flows") for the action in a UDP flow, at any depth of `loop` and `branch`, so a scenario that the contract refuses never reaches the runtime, where the flow would fail with `internal`. The fixtures `tls_upgrade_tcp`, `tls_upgrade_udp`, and `tls_upgrade_udp_nested` in `engine/fps-flow/tests/fixtures/` carry the verdicts for both validators.

### Tests

The tests run client and server programs against each other over the in memory duplex. They are listed below.

| Test | What it checks |
|---|---|
| `tests/sessions.rs` | Programs that complete, with byte counters, the opener requests, patterns split over one byte reads, the three stop rules and `last`, bytes past an `expect bytes` count, loops, branches, `set`, the five delay distributions on the paused clock, module functions, and the three close modes |
| `tests/failures.rs` | One test for each of the twelve reasons except the datagram one, and a second trigger for `connect_timeout`, `expect_timeout`, `peer_closed`, and `internal`, each checking the reason, action index, and action type of both sides |
| `tests/catalog.rs` | The reasons, sides, and action types against the metric catalog |
| `tests/upgrade.rs` | `tls_upgrade` with a stand in upgrader that masks the bytes: the first half crosses in the clear and the second masked, each side takes the role of its side and the client passes the server name, a failed handshake is `tls_failure` at the action, an unknown profile, a missing upgrader, and bytes buffered before the handshake are `internal` |
| `tests/core_functions.rs` | `int_bytes` and `now` in running sessions: the bytes on the wire under a pinned and a stepped clock, one instant bound with `set`, the wall clock by default, and every render time error as `template_error` with nothing sent |
| `tests/instances.rs` (variables) | A later flow and a flow two steps later read what an earlier flow bound with `set`, the pinned clock reaches every side of an instance and survives a rerun, and a client instance and a server instance each see only their own bindings (scenario contract v0.6, "Variables across `after`") |
| unit tests | `session_rng` streams and bug #90, delay sample means and supports, and the action index mapping |

## The worker runtime

The crate [`fps-runtime`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-runtime) of ENG-10 runs the workers of one agent. Each worker is one operating system thread with its own single threaded Tokio runtime and a `LocalSet`, so session tasks hold `Rc` and `RefCell` values and never take a lock. `Runtime::start` takes a `RuntimeConfig` with the run seed, the snapshot interval, and one `WorkerConfig` per worker, and returns a `RuntimeHandle` once every worker is ready.

### Workers and pinning

A worker config names the worker id, an optional core, and a role. A worker with a core pins its thread with the `core_affinity` crate, which calls `sched_setaffinity` on Linux, so the crate keeps `#![forbid(unsafe_code)]`. A failed pin does not stop the worker, and `WorkerExit::pinned` reports the result. We chose `core_affinity` under the MIT and Apache 2.0 licenses because the standard library offers no affinity call and `fps-backend-kernel` keeps its only unsafe module for socket options.

Workers share nothing mutable. Each worker owns its sessions, its timers, its `fps_stats::Shard`, its one MiB `Filler`, and either its schedule and connector or its listeners. The compiled `Scenario` sits behind an `Arc` and is only read. `global_session_id` puts the worker id in the top 16 bits of every session id, so sessions of different workers never share a generator stream from `session_rng`.

| Role | What the worker does |
|---|---|
| `Role::Client` | Drains its `fps_sched::Schedule` with a `TokioClock`, runs the client side of every start over a kernel `Connector`, and takes each source address from its range of a host pool through an `fps_hosts::HostSelector` |
| `Role::Server` | Binds one `KernelListener` per listen address, accepts sessions, and runs the server side of each one. `RuntimeHandle::listen_addrs` reports the bound addresses, which matters for port 0 |

A client session dials the rendered `connect` host when the program sets one, which must be an IP address, and otherwise server address `session mod len` of `ClientConfig::servers`. `ClientConfig::port` replaces the rendered port when it is set.

### Clock

`TokioClock` implements the `Clock` trait of `fps-sched` over the Tokio timer of the worker. The client loop drains the schedule, sleeps until `next_due`, and wakes early when a session closes, because a closed session can release a deferred start. The clock follows a paused Tokio clock, so unit tests advance the schedule without waiting.

### Run time rates

ENG-18 connects the runtime to the rate adjustment of ENG-13. A client worker builds its `Schedule` before it reports ready, and reports the `RateHandle` of the schedule with the name of its mix entry. `Runtime::start` registers every handle in one `fps_sched::RateControl` before it returns, and `RuntimeHandle::rate_control()` returns that registry as an `Arc<RateControl>`. The name is `FlowRef::entry` when it is set and the scenario name otherwise. Set `entry` to the mix entry id when two entries run the same scenario, because `UpdateRates` names entries. A schedule that is invalid now fails the start, as a bind failure does, and no longer ends the worker after the start. Server workers register nothing, so a mix entry that only a server worker serves is not in the registry. The UDP workers of `fps-agent` run their own schedules outside this runtime and register no handles, so UDP mix entries are not in the registry until AGT-05 or a later package adds them.

The agent drives the rates with the registry. AGT-05 keeps the `Arc` from `rate_control()`, which stays valid after `join`, and handles `UpdateRates` with `control.update(&[(entry, Objective::SessionsPerSecond(total))])`, where the total is the rate of the whole agent for the entry that CON-05 names, or with `control.scale(&[(entry, factor)])` for the calibration factors of CTL-21. A scenario is paused with a total of 0 and resumed with a positive one. An unknown entry changes nothing and returns `UnknownScenario`.

A worker wakes at least every 100 ms (`RATE_POLL`) to read its handle. The loop sleeps until the earlier of `next_due` and the poll time, so a worker whose `next_due` is `None`, because its rate is 0 or its sessions sit at the cap, sees a new target within 100 ms, and a worker that waits for a distant start at a low rate does not keep the old spacing after a rise. A wake without a change drains an empty schedule, which costs about ten cheap loop passes per second per worker. The snapshot tick also wakes the loop, so the poll matters when the snapshot interval is longer than 100 ms, as it is by default.

### Capture of TCP sessions

ENG-19 lets the runtime tap its TCP sessions for `fps-capture`. `RuntimeConfig::capture` is a `fps_capture::CaptureCell`, empty by default, and `RuntimeConfig::with_capture` shares one cell that the owner already holds. `RuntimeHandle::capture_cell()` returns a clone, so the agent starts a capture in the middle of the run with `cell.set(capture)` and ends it with `cell.clear()`. A worker wraps the opener of every session in a `TapOpener`, which reads the cell once, when the session opens, and not again. While the cell is empty that is one atomic load and a `Tapped::Plain` wrapper, which forwards every read and write after one `match`, and the worker asks the socket for no address. While a capture runs, the opener takes the local and peer address of the opened `KernelStream`, orders them into client and server by the side of the worker, and calls `Capture::begin` with the run wide session id, so the sampler of the capture decides. A session that is already open when `set` runs is not recorded, and one that began with a recorder keeps it after `clear`.

Both worker roles tap. A client session records the bytes of the client end and a server session those of the accepted end, so a run that holds both sides in one process and gives them one `Capture` records each connection twice, under two session ids. The agent has one role per process, so it does not meet this. The tap sits on the kernel stream, below any TLS that a module layers on top, so a capture holds what a packet capture would hold.

### TLS upgrade in the runtime

`RuntimeConfig::tls` holds the `TlsProfiles` of the run, shared by every worker, and `RuntimeConfig::with_tls` sets it. The registry maps names to `fps_module_tls::TlsConnector` values for the client role and to `TlsAcceptor` values for the server role, so one name may exist for both. It implements `TlsUpgrader`, and a worker passes it to every session with `Session::set_tls_upgrader`. A client upgrade calls `TlsConnector::connect` with the server name of the action, the name set with `TlsProfiles::with_server_name`, or `localhost`, in that order, and a server upgrade calls `TlsAcceptor::accept`. Both wrap the transport as `Box<dyn SessionIo>`, so the capture tap of ENG-19, which wraps the kernel stream in `TapOpener`, stays below the TLS layer and records the encrypted records. The session ends as before, and `close: fin` sends `close_notify` before the FIN.

The key log is part of the profile. Give the client and server profiles of `fps-module-tls` the same `NssKeyLog` in `key_log`, and it receives the secrets of every handshake that runs through them. A capture of the run decrypts with that file, where a tool such as Wireshark reads it through `tls.keylog_file`.

#### Connectors per session (ENG-24)

A `TlsConnector` owns a resumption store, so sessions that share one resume when the server issues tickets, and the ClientHello of a resumed TLS 1.3 session carries `pre_shared_key`, which changes its JA4 (finding of MOD-04). `TlsProfiles::with_client(name, connector)` keeps that shared form. `TlsProfiles::with_client_factory(name, || TlsConnector::new(&profile, roots.clone()))` registers a factory instead, and the registry calls it for every session, so each session has a fresh store, runs a full handshake, and sends the ClientHello of its profile. The factory keeps the resumption setting of the profile (`ClientResumption::InMemory`) and does not switch it to `Disabled`, because a TLS 1.2 ClientHello without the `session_ticket` extension has another JA4 than the profile documents. A build costs one rustls `ClientConfig` and a small store, and the key log `Arc` stays shared, so the capture still decrypts. A factory that returns an error fails the session with reason `internal`. The agent builds its client profiles with `with_client` today (`fps_agent::tls`), so its sessions still resume against a server that issues tickets until it switches to the factory.

#### TLS in scenario instances (ENG-24)

`fps_interp::FlowEnv::tls_upgrader(flow, side)` returns the `TlsUpgrader` of one TCP flow side, `None` by default, and the instance asks for it for every side it starts, including the flows that `spawn` starts, and hands it to `Session::set_tls_upgrader`. `InstanceEnv` returns one `SessionTls` for all sides of the instance, so a `tls_upgrade` in a spawned flow works as in a single session flow. A worker creates an upgrader only when the run has profiles and a flow of the scenario holds a `tls_upgrade`, so other sessions allocate nothing for it.

#### The tls metrics (ENG-24)

`SessionTls` wraps the registry and fills a sink of handshake events while the session runs, and the worker drains the sink when the session or the instance ends and records it in its shard. The upgrader holds the events because it is `Sync` and the shard is not. A completed handshake adds one to `tls.handshakes`, one to `tls.resumptions` when `HandshakeReport::resumed` is true, and a sample of `HandshakeReport::elapsed` in microseconds to `tls.handshake_time`. A failed handshake adds one to `tls.handshake_failures` and one to `tls.alerts.tx` and `tls.alerts.rx` for the alerts that the `TlsError` names (`alert_sent`, `alert_received`). A handshake that the open timeout cancels counts as a failure without alerts, through a drop guard in the upgrade future. A profile that is missing or does not build is `internal` and counts no handshake. `tls.resumption_ratio` is not recorded, because it is a derived gauge that readers compute from the two counters, as `sessions.failed_ratio` is.

The series carry the `side` of the handshake (the role of the `tls_upgrade`, which is the side of the worker unless the action sets a role) and the `port` label, which the catalog defines as the test port name. The runtime knows no port names, so `FlowRef::port` carries it (`None` gives `default`), and the agent sets it from the plan. Series register with the first event of their kind, so a run without `tls_upgrade` has no `tls.*` series. `fps-stats` has no `tls.*` helper for alerts and the histogram, so the runtime registers those with `Shard::counter` and `Shard::histogram` directly.

### WebAssembly extensions in sessions

ENG-17 loads the extensions into the runtime. `fps_runtime::ExtensionSet` holds one compiled `fps_wasm::WasmExtension` per module name, behind an `Arc`. The owner builds it before `Runtime::start`: `ExtensionSet::load(module, bytes, limits, declared)` compiles a component, and `load_module_dir(&manifest, dir, limits)` reads `extension.wasm` from the module directory and returns `Ok(false)` when the module ships none. ENG-21 added `load_module_bytes(&manifest, bytes, limits)`, which compiles bytes that the caller already holds and checks the catalog against the manifest. A caller that digests the module tree reads `extension.wasm` once, digests those bytes, and passes the same bytes to `load_module_bytes`, so a writer cannot swap the file between the digest and the compile. `load_module_dir` reads the file again and now calls `load_module_bytes`, so existing callers keep working. Compilation happens once per module and never on a worker thread. `FlowRef::extensions` carries the `Arc<ExtensionSet>` to every worker, so all workers share the compiled component, and each session that calls the module instantiates it through `WasmExtension::open`, which compiles nothing.

The catalog is checked at load. With a manifest `Catalog` given, the function names, the parameter names and order, the types, and the required flags must equal what the extension returns from `functions`, as `contracts/README.md` demands, and the load fails with `ExtensionError::Catalog` otherwise. The manifest keeps no default values, so those are not compared.

A session whose flow has an extension set runs its module functions through `SessionExtensions`, which wraps the native `FunctionFactory` output of the flow. A call to a module of the set goes to the extension, and any other call goes to the native functions, so a scenario can mix native and WebAssembly modules. A flow without an extension set takes no wrapper, and the worker pays one `Option` check per session. The extension instance opens on the first call that names its module, so a session that never calls the module never instantiates it. The worker ends the instances with `finish`, which runs `on-close` with the close reason that the outcome of the session maps to (`finished`, `reset`, `peer-closed`, `timeout`, or `failed`). ENG-21 exports that mapping as `fps_runtime::close_reason`, so a runner outside the crate, such as the UDP loop of the agent, uses the same function and keeps no copy.

#### Failures

A hook error, an out of fuel trap, a memory limit, or another trap ends the calling session and no other, because every session owns its instance. The `WasmFailure` becomes an `FnError` for the renderer, which wraps it in `TemplateError::Function`, and `FailureKind::reason` returns `extension_error` for every such failure, so `sessions.failed` counts it under that reason. `FailureKind::reason` needed no change for this: ENG-15 already maps a function failure of any kind to `extension_error`, and the test below confirms it end to end. `SessionExtensions::take_failure` keeps the `WasmFailure` behind the error, for the finer label. A looping extension stops at `Limits::fuel_per_call`. The hook runs on the worker thread, so the other sessions of that worker wait for the fuel budget (20 million instructions by default) and then continue.

#### Randomness

Every extension session gets its own `Host`, whose `random_u64` and `random_bytes` draw from `fps_runtime::extension_rng(seed, module, session_id, side)`. It is `fps_interp::session_rng` over the run seed xor a domain constant for extensions xor an FNV-1a hash of the module name, so a replay with the same seed reproduces every draw, the order of sessions does not matter, and the stream is separate from the template stream of the session and from the streams of other modules. The session id is the run wide id of `global_session_id`, which has the worker id in its high bits.

#### Limits of this step

The interpreter has no seam yet for transport writes, timers, or received bytes, so the runtime calls the `render` and `on-close` hooks only. `send` and `set-timer` return `not-allowed` to the guest, `on-data` and `on-timer` are never called, and `on-open` runs at the first template call and not after `connect` or `accept`. The agent still has to call `ExtensionSet::load_module_dir` for each plan module and set `FlowRef::extensions`. A failure in `on-close` is returned by `finish` and dropped by the worker, because the session outcome is recorded by then. `SessionKey::transport` carries the transport of the session, and `open` passes it to the extension as `SessionInfo::transport`. The worker of `fps-runtime` handles stream sessions only and sets `Transport::Tcp`. A runner of datagram sessions, such as the UDP loop of the agent, sets `Transport::Udp`. `fps_runtime::Transport` re-exports `fps_wasm::Transport`.

#### Tests

`tests/extensions.rs` reuses the WAT component of `fps-wasm` through a `#[path]` module.

| Test | What it checks |
|---|---|
| `a_looping_extension_fails_only_its_own_sessions` | Two client workers and two server workers share one compiled extension with a 200,000 fuel limit. One scenario calls `t.echo` on both sides, the other renders `t.spin()`. All 40 looping client sessions fail with reason `extension_error`, and the 40 good sessions on the same threads succeed on both sides, with no failure |
| `extension_randomness_derives_from_the_seed_and_the_session_id` | The same seed and session id give the same bytes in a new instance, a different id, seed, or side gives other bytes, and the first draw equals `extension_rng` |
| `a_fuel_limit_fails_the_session_and_leaves_other_sessions_alone` | `OutOfFuel` with reason `extension_error`, a bystander instance that opened earlier keeps working, a hook error keeps its kind, an unknown function is `Unsupported` |
| `modules_outside_the_set_use_the_native_functions` | A call to a module that the set lacks reaches the fallback functions |
| `the_catalog_must_match_the_manifest`, `an_extension_loads_from_the_module_directory` | Catalog mismatch, missing function, invalid component, duplicate module, and loading from a module directory with and without `extension.wasm` |
| `the_extension_sees_the_transport_of_its_session` | A test component whose `on-open` refuses transport `udp`: a session with `Transport::Tcp` renders, and one with `Transport::Udp` fails at open with an extension failure (ENG-21) |
| `the_compiled_extension_is_the_bytes_the_caller_passes` | With a different file on disk, `load_module_bytes` compiles the passed bytes. Bad bytes fail although the file is good, the catalog check still applies, and `load_module_dir` still loads (ENG-21) |
| `close_reason_maps_outcomes_for_every_runner` | The exported `close_reason` maps `Ok` to `finished`, `aborted` to `reset`, `peer_closed` to `peer-closed`, an expect timeout to `timeout`, and any other reason to `failed` (ENG-21) |

### Multi flow scenario instances

ENG-23 lets a worker run a scenario with more than one flow, such as the ftp scenarios whose control flow spawns one data flow per transfer. `FlowRef::flow` then names the entry flow, which must not be spawned. A worker whose scenario has several flows runs one `fps_interp::ScenarioInstance` for every session start, through `instance.rs`. A scenario with one flow never enters that code: its worker keeps the single session path and registers no extra series, and `tests/loopback.rs` and `tests/capture.rs` run unchanged on it.

A client worker starts an instance with `Sides::Client` at each start of its schedule, and a server worker starts one with `Sides::Server` for each connection that its listener accepts, so a start or an accept counts as one session for the schedule, the concurrency cap, and the ramp down, as before. The instance orders the flows by `after`, starts the same side of a spawned flow at each `spawn`, and passes the variables between flows, see [The flow program interpreter](#the-flow-program-interpreter). The worker gives it an `InstanceEnv`, a `FlowEnv` over the kernel backend that hands out, for every flow side:

| Part | What it does |
|---|---|
| Opener, client | The entry flow dials as the single session path does, including the port override of `ClientConfig::port`. Every other flow dials the host that its `connect` renders, or the server of the session, on the rendered port, which the override never replaces. It retries a refused connection with a doubling wait of 2 to 50 ms for at most two seconds (`OPEN_GRACE`), because the server side of a spawned flow may not have bound its port yet |
| Opener, server | The entry flow takes the connection that the listener accepted. Every other flow binds the rendered port on the address that accepted the entry connection, accepts one connection, and releases the port. It retries a bind that finds the port in use for the same two seconds, because another instance may still hold the port |
| Capture | Every opener is wrapped in a `TapOpener` with its own session id, so each flow session is a separate connection in the capture |
| Functions | Every side gets its own native functions from `FlowRef::functions`, and its own `SessionExtensions` when the flow has an extension set. The worker ends the extension instances with `finish` when the instance returns, using the close reason of the side |
| Session ids | An instance owns a block of 65 536 ids, which starts at `global_session_id(worker, start << 16)`. The instance gives the n-th side that it starts the id `first + n`, and the environment counts its own starts to know the id before the side opens. The worker checks the count against the `SideRecord` of every finished side, and a debug build fails when they differ |
| Series | Every flow side counts as a session in the series of its flow, labelled `application` with the module of the flow, `scenario` with the scenario name, and `side` with the worker side, as the single flow series are. Flows of one module share their series, so `mixed_session` of the ftp module counts four sessions per instance, one control and three data connections, under `application=ftp`. A side counts as started when the instance starts it. It counts as ended when the instance returns, with the byte counters and failure reason of its `SideRecord` and the duration from its start to the drop of its opener |

A failure of any side fails the instance, which aborts the other sides with reason `aborted`. Each side counts under its own reason, so one failed data flow shows up as the failure of that side plus an `aborted` count for each side it cut short. `sessions.concurrent` of a side that ended before its instance returns falls only when the instance returns, because the interpreter reports the records of an instance at its end.

`RuntimeConfig` refuses, before any thread starts, a scenario with several flows whose entry flow is spawned or that has a UDP flow, because the worker runs the flows of an instance over TCP only. The SIP module needs the datagram opener that the UDP workers have, which a later package adds. `tls_upgrade` runs in every flow of an instance since ENG-24, see [TLS in scenario instances](#tls-in-scenario-instances-eng-24).

The agent runs a multi flow plan through the same `RuntimeConfig`. It must load the scenarios with `Scenario::parse_instance` or `load_instance`, build the `FlowRef` with flow index 0 as the entry flow, and read the listen port of a server worker from the entry flow only, which is what `assemble` does for a single flow now. AGT-16 lifts its check that a scenario has exactly one flow.

### Statistics

Every worker registers its session series once and records `sessions.attempted`, `sessions.established`, `sessions.succeeded`, `sessions.failed` by reason, `bytes.tx`, `bytes.rx`, `sessions.concurrent`, and `sessions.duration`, with the `application` label from the flow module, the `scenario` label from the scenario name, and the `side` label of the worker. A worker registers them through `fps_stats::SessionCounters`, which `metrics.rs` calls with the worker side and the interpreter reason mapped to `fps_stats::FailureReason`, so the runtime has no loop over the reasons of its own. Once per snapshot interval, one second by default, and once more when it ends, the worker sends a cumulative `WorkerSnapshot` over a channel. The `Collector` keeps the newest snapshot of every worker and merges them per side, so the two sides of one session never add up, although every series now also carries its `side` label. `RuntimeHandle::collect` reads the snapshots that arrived so far, and `RuntimeHandle::join` returns a `RunReport` with the collector and the exit of every worker.

### Stop and abort

`RuntimeHandle::stop` takes the ramp down time. Every worker then starts no new session, a server worker closes its listeners, and open sessions run on. Sessions still open when the ramp down time ends are aborted, which resets their transports and counts them under reason `aborted`. `RuntimeHandle::abort` aborts every open session at once. A later stop can only shorten a ramp down, and an abort always wins. A client worker also ends on its own when its schedule finishes and its last session closes. Dropping the handle aborts every worker.

### Tests

The tests run over loopback addresses with the kernel backend and are listed below.

| Test | What it checks |
|---|---|
| `tests/loopback.rs` web_get | Two client workers drive the http1 `web_get` example at 50 sessions per second for two seconds against two server workers. Both sides report 200 attempted and 200 succeeded sessions, every session series of each side carries its `side` label with 200 duration samples, client `bytes.tx` equals server `bytes.rx` and the reverse, and every worker sent one snapshot per second and a final one |
| `tests/loopback.rs` ramp down | A stop during a run lets every open `web_get` session finish, and the run ends within the ramp down time plus one second |
| `tests/loopback.rs` stop and abort | Sessions that wait 20 seconds for an answer are aborted at the end of a 700 ms ramp down, the run ends within the ramp down time plus one second, and an abort ends the server workers within one second |
| `tests/loopback.rs` rate control | Two client workers drive `web_get` at 60 sessions per second under the entry name `browse`, which differs from the scenario name. Through `rate_control()` the test updates an unknown name (refused, nothing changes), pauses to 0 (no start for one second), resumes to 60, raises to 120, and lowers to 36. Every window of 1.2 seconds realizes the new rate within 15 percent, measured between two snapshots with the same staleness. The runtime runs on wall time, so the test cannot use a simulated clock. The 15 percent are looser than the 1 percent for constant arrivals that ENG-13 holds on a simulated clock, and cover the 100 ms poll after a change, scheduling on a shared two CPU host, and the 20 ms snapshot grain |
| `tests/loopback.rs` paused wake | With a snapshot interval of 30 seconds, so that no snapshot tick wakes the worker, a worker paused at rate 0 opens its first connection to a plain listener within 300 ms of a resume, twice in a row. Without the bounded sleep the test fails, because the worker never wakes |
| `tests/loopback.rs` setup | Pinning to an allowed core, a bind failure that fails the start, and the configuration checks |
| `tests/capture.rs` mid run | A client runtime starts 20 `web_get` sessions per second for three seconds. One second in, the test sets a capture that samples every session into the cell of the handle. The ring holds only sessions that opened after that moment, between 10 and the number that started after it, and each is a complete `web_get` session that the independent reader reassembles: handshake, four requests and four responses, and two FINs. Where tshark is installed, it also finds every request and response, one conversation per session, and no checksum, malformed, retransmission, loss, or reordering finding. The tshark part skips without the tool and fails when `FPS_REQUIRE_CAPTURE_TOOLS` is `1` |
| `tests/capture.rs` server and empty cell | A server worker whose cell holds a capture records all 20 sessions it accepts. A cell that was set and cleared before the start records no session |
| `tests/tls_upgrade.rs` STARTTLS | An inline SMTP scenario runs EHLO, STARTTLS, `220`, a TLS 1.3 handshake, a second EHLO, `MAIL FROM`, and QUIT between a client runtime and a server runtime, and a capture records two sessions. On the wire, the client starts with the plaintext `EHLO` and `STARTTLS`, the server with its greeting, `250 STARTTLS`, and `220 2.0.0 Ready to start TLS`, and each direction continues with TLS records that begin with a handshake record. The mail transaction (`MAIL FROM`, the address, `AUTH PLAIN`, the replies, `QUIT`) appears nowhere on the wire. A decrypter of the test, built on `ring` and independent of the module, derives the record keys from the exported `NssKeyLog` and recovers exactly the second half of both directions, and a key log with flipped digits decrypts nothing |
| `tests/tls_upgrade.rs` failures | A client that trusts another authority fails every session with `tls_failure`, and so does the server that receives the alert. A runtime without profiles fails the client with `internal` |
| `tests/tls_sessions.rs` | Three sessions of the Chrome profile against a ticket issuing server, through a relay that records the ClientHello of each connection: with a connector factory all three have the `expected_ja4` of the profile (computed from the bytes with `realism/ja4.py`), and with one shared connector the later two resume, differ in JA4, and count in `tls.resumptions`. A `tls_upgrade` in a spawned flow succeeds on both sides. The `tls.handshakes`, `tls.resumptions`, `tls.handshake_failures`, `tls.alerts.tx`, `tls.alerts.rx`, and `tls.handshake_time` series have the `port` and `side` labels, and appear in the proto JSON of the run snapshot |
| `tests/instances.rs` ftp | `mixed_session` of the ftp module, a listing over EPSV, a download over PASV, and an upload over EPSV on one control connection, runs between a client runtime and a server runtime over loopback, eight instances in one second. Both workers count 32 sessions, four per instance, and every one succeeds. A capture of the client sessions holds 32 connections, and each of the eight source hosts dialed the control port once and a port of the data range three times, with no reset, so each transfer has its own data connection |
| `tests/instances.rs` order and failure | A data connect that comes 150 ms before the server binds is retried and succeeds, a flow with `after` starts when its predecessor is done and dials its own port, and a data flow that times out fails its instance, aborts the control flow, and leaves every concurrent gauge at zero |
| `tests/instances.rs` extensions and config | An extension component is called from the control flow and the spawned flow, a looping call in the spawned flow fails with `extension_error`, and the configuration check refuses a UDP flow and a spawned entry flow |
| `tests/properties.rs` | Session ids stay unique across workers, and the collector total equals the sum of the newest snapshot of every worker in any arrival order |
| unit tests | The clock on a paused Tokio clock, the bounded wake time, the stop state order, opener address selection, the collector, and the instance openers: a bind that waits for a port in use, a used stream, and the end of a side |
## UDP datagram sessions

ENG-06 added UDP flows to the interpreter under the datagram rules of [scenario contract v0.2](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md#datagram-rules-for-udp). A UDP flow keeps datagram boundaries, so it runs over its own seam in place of the `SessionIo` byte stream.

### The DatagramIo seam

The crate [`fps-io`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-io) defines the trait `DatagramIo`, a connected datagram socket that carries one session. It has three methods.

| Method | Behavior |
|---|---|
| `poll_send` | Sends the bytes as one datagram, and fails with `InvalidInput` above `MAX_DATAGRAM`, which is 65,507 bytes |
| `poll_recv` | Consumes one whole datagram and returns its full length, and when the buffer is smaller it keeps the first bytes and still returns the full length, as `recv` with `MSG_TRUNC` does on Linux |
| `close` | Releases the socket, after which every operation on this side fails with `NotConnected`, while the peer observes nothing |

`DatagramIoExt` adds the async helpers `send_datagram` and `recv_datagram`. Returning the full length from `poll_recv` lets the interpreter detect an oversized datagram without a second buffer, and a kernel backend gets the same behavior from `MSG_TRUNC`. TRN-02 implements the trait over kernel UDP sockets, see [Kernel UDP backend](Transport.md#kernel-udp-backend).

`fps_io::memory::datagram_pair(queue)` returns two connected `MemoryDatagram` ends for tests. Each direction queues at most `queue` datagrams, and a datagram sent into a full queue or to a closed end is dropped without an error and counted by `dropped`, as UDP drops it. A queue of 0 therefore models a path that loses every datagram. `MemoryDatagram::readable` waits for a datagram without consuming it, which a test opener uses to implement `accept`.

### Running a UDP side

`Session::run_datagram_program` opens a UDP side with a `DatagramOpener` and runs its body, and `Session::open_datagram` and `Session::run_datagram` split the two steps, as their TCP counterparts do. `DatagramOpener` takes the same `OpenRequest` as `Opener`. For `connect` it returns a socket connected to the peer, and for `accept` it binds the port, waits for the first datagram, and returns a socket on which that datagram stays queued for the first `expect`. The open timeout of `Limits` bounds both.

The TCP and UDP paths share one loop over the compiled ops, so `delay`, `set`, `loop`, `branch`, and the generator behave the same. Only three actions differ.

1. `send` renders the template and writes it as one datagram. A rendering above 65,507 bytes fails the session with reason `internal` before anything is sent.
2. `expect` reads exactly one datagram within its `timeout_ms` and consumes all of it. `bytes: N` requires exactly N bytes, `min_bytes: N` at least N bytes, and `pattern` the pattern somewhere in the datagram. A datagram that breaks the rule fails with reason `datagram_rejected` and `FailureKind::DatagramMismatch`, and an oversized one with `FailureKind::DatagramTooLarge`, and the interpreter never joins two datagrams or keeps part of one for the next `expect`. The whole datagram becomes `last` when a template of the side refers to it.
3. `close` releases the socket in every mode, because UDP has no close handshake.

`Session::set_max_datagram` sets the datagram limit of `expect`, clamped to the range from 1 to 65,507 bytes with 65,507 as the default. A longer datagram fails with reason `pattern_mismatch` and `FailureKind::DatagramTooLarge`, and it still counts in `bytes_rx`. The datagram lands in the 64 KiB scratch buffer of the session and is copied into `last` only when a template needs it. A connected socket reports ICMP port unreachable as `ConnectionRefused`, which fails the session with reason `connect_refused`. When the `stop` future completes first, the session closes the socket and fails with reason `aborted`.

The compiled `Op::Expect` of `fps-flow` does not carry the per action `max_datagram` of the contract yet, so the session limit applies to every `expect` of the side. The validator already rejects a fixed size rule above the action limit, and the ENG-06 log entry records the follow up for `fps-flow`.

### Tests

| Test | What it checks |
|---|---|
| `fps-interp/tests/datagrams.rs` | The DNS lookup example of the contract end to end, one datagram per `send`, `last` as the whole datagram, the three stop rules against one datagram, no joining, the datagram limit at and above its value, the 65,507 byte bound of `send` and `expect`, the clamp of `set_max_datagram`, expect and accept timeouts on the paused clock, lost datagrams, per datagram timeouts in a loop, `connect_refused`, `aborted`, and the three close modes |
| `fps-io/tests/datagram.rs` | Property tests that datagrams keep their boundaries, order, and full length through any buffer size, and that a full queue keeps the oldest datagrams |
| `fps-io` unit tests | Truncation with the full length, the send bound, queue drops, `readable`, and `close` |
## Vectored writes for fill bodies

ENG-05 closed Gate 0 carry over item 1 of the [Gate 0 report](https://github.com/alrawi/FixingPointSystem/blob/main/docs/phase0/gate0-report.md). Spike S1 lost 26.1% of its bulk session rate against hand written code, and the spike suspected the copy of each 1 MiB `fill()` body into the write buffer. A `send` now writes large `fill()` bodies straight from the shared filler of the run, and the bytes on the wire stay identical for the same seed.

### Render parts

`Template::render_parts` in `fps-flow` renders a template as a list of `Part` values. A `fill()` at the top level of the template that renders at least `min_ref` bytes becomes `Part::Filler` ranges of `Filler::as_bytes`, one range per wrap of the 1 MiB filler, and every other byte goes into the output buffer and is covered by `Part::Out` ranges. The parts in order equal the bytes of `render_into`, and the render draws the same random numbers, so the session generator stays in step and a replay reproduces every session. A `fill()` inside a module call argument is still copied, because the module function needs its bytes.

### The send path

`send` renders with `render_parts` and the threshold `VECTORED_FILL_MIN`, which is 16 KiB. A rendering without filler parts goes out with one `write_all` as before. A rendering with filler parts goes out with vectored writes of up to 16 slices per call, which resume inside a slice after a partial write, so a 1 MiB body costs one `writev` of three slices on a kernel socket. A transport without vectored writes receives one part per write, and neither path copies the filler. The session keeps its part list between actions, so the send path allocates nothing per action once the list has grown.

### The bulk benchmark

`engine/fps-interp/benches/bulk.rs` repeats the S1 bulk workload in one process over loopback TCP. One iteration runs 16 concurrent `web_get` sessions of 4 requests with 1 MiB bodies on two worker threads, each with its own runtime, and each session runs both sides. It compares three variants.

| Variant | Code |
|---|---|
| `interp` | Both sides run in the interpreter |
| `handwritten` | Hand written code that sends one pre rendered response, the S1 baseline |
| `handwritten-copy` | Hand written code that renders each response with `render_into`, which is the copy that the interpreter made before ENG-05 |

`cargo bench -p fps-interp --bench bulk` runs the variants under criterion. With `FPS_BENCH_OUT=<file>` the benchmark runs a paired measurement instead, which runs the three variants in rotated order in every round, so that load from other processes falls on all of them alike, and writes the medians of 300 rounds as an fps-bench/v1 result for `bench/compare.py`. The baseline is `bench/baselines/fps-interp-bulk-container.json`.

| Measurement on the 2 vCPU recording host | Interpreter over hand written |
|---|---|
| Copying send path before ENG-05 | 0.773 |
| Hand written code with the copy | 0.69 to 0.72 over 7 runs |
| Vectored send path | 1.09 to 1.17 over 7 runs, 1.172 in the baseline |

The interpreter now meets the Gate 0 criterion of at most 25% overhead with room to spare, and the copy explains the S1 gap, because hand written code that makes the same copy loses about 30%. The recording host ran other agents during the runs, so absolute rates moved by up to a factor of two between runs, while the paired ratios moved by at most 7%. The baseline therefore gives the rates a 60% tolerance and the ratios a 10% tolerance, and the ratios are the metrics to watch. Loopback hides NIC and virtualization cost, so carry over item 3 still repeats the run on a 4 vCPU KVM guest.

| Test | What it checks |
|---|---|
| `fps-flow` template unit tests | `render_parts` joins to the bytes of `render_into` and leaves the generator in step for thresholds from 0 to none, a template without `fill()` is one part, and a negative size fails |
| `fps-interp/tests/vectored.rs` | The wire bytes of large bodies equal a copying render, one vectored write per response, partial writes of 1,000 bytes, a 20 MiB body across several writes, a small body copied into one write, a transport without vectored writes, and a property test over body sizes, write limits, and seeds |
## Scenario instances

ENG-07 added `ScenarioInstance` to `fps-interp`, which runs every flow of one scenario together with `after` ordering, `spawn`, and shared scenario variables. The source is [`engine/fps-interp/src/instance.rs`](https://github.com/alrawi/FixingPointSystem/blob/main/engine/fps-interp/src/instance.rs).

### Loading

`Scenario::parse_instance` and `Scenario::load_instance` in `fps-flow` compile a scenario for an instance runner. They accept flows with `spawned: true` and compile `spawn` to the new op `Op::Spawn`, while `Scenario::parse` and `Scenario::load` keep rejecting both with `LoadError::Unsupported`, so a runner that executes flows one by one never meets them. Every `Flow` now carries its `after` list and its `spawned` flag. A `Session` without a spawn queue fails at a `spawn` with reason `internal` and action type `spawn`.

### Rules

| Topic | Rule |
|---|---|
| Sides | `InstanceConfig::sides` selects the client sides, the server sides, or both. A flow without a local side completes at once |
| `after` | A flow that is not spawned starts once every flow of its `after` list is done. Flows without `after` start together |
| `spawn` | A side that runs `spawn` starts the same side of the target flow at once. A side may spawn a flow many times, and the k-th spawn of the client pairs with the k-th spawn of the server |
| Variables | A side reads the shared variables as they stood when it started. A spawned side also reads every `set` of its parent side before the `spawn`. When a side completes, its `set` bindings join the shared variables |
| Session ids | Flow sessions take ids from `first_session_id` in start order, and both sides of one flow session share an id |
| Failure | The first failed side fails the instance. The instance aborts every running side with reason `aborted`, starts nothing further, and marks the flows that never started as skipped |
| Spawned failure | A failed spawned flow also fails its parent and every flow up its spawn chain. `InstanceError::spawned_by` lists the chain from the direct parent |
| Stop | When the stop future completes, the instance aborts every running side, and the first aborted side fails it |

A flow that depends through `after` on a spawned flow waits until that flow ran and completed, and it is skipped when no side spawned it. The contract gives no rule for the instance after a failure, so we chose to fail fast, because the peer of a failed side cannot finish its exchange either.

### The engine seam

`FlowEnv` hands the instance a fresh `Opener` or `DatagramOpener` and fresh module functions for every flow side that it starts, so sides run concurrently. The instance polls all sides on the task that awaits `ScenarioInstance::run`, so the future is not `Send`, like the worker sessions of `fps-runtime`, which run on a current thread runtime. `records` lists each finished side with its session id, byte counters, and failure reason in completion order, `state` gives the `FlowState` of each flow, and `later_failures` keeps the failures after the first one.

| Test | What it checks |
|---|---|
| `fps-interp/tests/instances.rs` | The web_browse scenario of the specification runs its lookup flow before its page flow and dials the address that the lookup bound, one sided client and server instances order their own sides, a spawned data flow reads the port that its parent parsed, a flow with `after` on a spawned flow waits for it, a spawned flow failure fails its parent and the instance and skips the dependents, a failed flow skips its dependents, stop aborts the running flows, and a single session rejects `spawn` |
| `fps-flow` unit tests | `parse_instance` compiles `spawn` and keeps `after` and `spawned`, `parse` still rejects them, and `raw` stays unsupported |

The web_browse test uses plain HTTP for the page flow and test doubles of the `dns` functions, because the engine runs no transport modules yet and the specification example is illustrative.

## The WebAssembly extension host

ENG-11 added [`fps-wasm`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-wasm), which runs module extensions that target the `extension` world of `contracts/extension/fps-extension.wit` (decision D3). It uses wasmtime 48 with the component model, fuel metering, and the Cranelift compiler, and no compilation cache, parallel compilation, async support, or GC proposals. SIMD, relaxed SIMD, multiple memories, and 64 bit memories are off, so a component built with them fails to load. The wasmtime license is Apache-2.0 with the LLVM exception, which `deny.toml` allows.

### Pieces

| Item | Contents |
|---|---|
| `WasmEngine` | One wasmtime engine for the process, with fuel metering on |
| `WasmExtension` | A compiled component with its linked engine calls and its function catalog. `load` reads the catalog once, and `open` starts one session |
| `WasmSession` | The hooks of one session: `on_data`, `on_timer`, `render`, and `on_close`, with the session instance behind them |
| `WasmFunctions` | The module table of one session. It implements `fps_flow::TemplateFn`, the trait that native extensions use, and forwards each call to the `render` hook of the named module |
| `Host` | The `engine` interface as a Rust trait: `send`, `set_timer`, `cancel_timer`, `log`, `random_u64`, and `random_bytes`. The runtime implements it per session |
| `Limits` | `fuel_per_call`, `max_memory_bytes`, and `max_table_elements`, with defaults of 20 million fuel, 16 MiB, and 10,000 elements |
| `WasmFailure` | Out of fuel, out of memory, another trap, a hook error, a failed session, or an invalid component |

The engine has no `Extension` or `ExtensionSession` trait yet, although the extension README names them, so `TemplateFn` is the seam that both kinds of extension share today and the other hooks are methods of `WasmSession`. A native extension follows the same method shapes, and a later package can lift them into a trait when the interpreter calls the hooks.

### One instance per session

The contract says the engine instantiates the component once per worker. We instantiate once per session instead, from a pre linked `InstancePre`, because a trap leaves a component instance unusable. With one instance per worker, a looping extension would fail every session on that worker. A session instance has its own store, its own fuel, and its own memory, so a failure ends that session and no other, and a memory limit is a per session limit. The cost is one instantiation per session, which compiles nothing.

### Limits

Every hook call, `on-open` and the catalog call included, starts with a fresh fuel budget of `fuel_per_call`, and a call that uses it up traps with `WasmFailure::OutOfFuel`. One fuel unit is about one WebAssembly instruction. `WasmSession::fuel_used_by_last_call` reports the use, so a caller can tune the limit.

A store limiter holds the memory limit. Plain wasmtime limits make `memory.grow` return -1, and a guest can carry on after that, so ours turns a refused growth of memory or a table into a trap, which is `WasmFailure::OutOfMemory`. An instance may hold one memory, eight tables, and 32 core instances, so a component with a second memory fails to load. A module whose initial memory is above the limit fails at load with `OutOfMemory`. The host keeps a small reservation per memory and no guard region, so thousands of session instances do not exhaust the address space.

### Failures and phases

Every failure of an extension counts as `sessions.failed` with the reason `extension_error`, which `WasmFailure::reason` returns as `fps_stats::FailureReason::ExtensionError`. `WasmFailure::label` gives the finer label `extension_<kind>` of the extension README, where a hook error uses its own kind and a trap or a limit counts as `internal`. A failed session refuses every later hook with `WasmFailure::SessionFailed`, and `on_close` after a failure skips the guest and succeeds, because the instance cannot run any more. Through `TemplateFn` a failure becomes an `FnError`, and `WasmFunctions::take_failure` returns the `WasmFailure` behind the last one, because `FnErrorKind` has no case for a trap. The renderer can then record `extension_error` and not `internal`.

The engine calls follow the table of the extension README. The hook that runs decides what is valid, and a call that is not valid returns `not-allowed`. The calls `random-u64` and `random-bytes` have no error case, so a call during `on-close` traps the guest, and `random-bytes` traps when it asks for more bytes than the memory limit.

`WasmFunctions` ignores the `rng` argument of `TemplateFn::render`. An extension gets randomness only through the `random-*` calls, which the `Host` of its session serves, so the host derives its generator from the run seed and the session id. `fps-runtime` does so since ENG-17, see [Randomness](#randomness).

### Tests

The test extension is core WAT text in `tests/support/mod.rs`. The tests encode it as a component with `wit-component` and the WIT file of the contract, so its types match the contract by construction. Its hooks react to the first byte of the data, the function name, or the timer id.

| Test | What it checks |
|---|---|
| `an_endless_loop_is_stopped_by_fuel_and_fails_only_its_session` | An `on_data` loop stops with `OutOfFuel` and reason `extension_error`, the session refuses later calls, a second session of the same extension and a session opened afterwards still work |
| `an_endless_render_fails_the_template_call_and_counts_as_extension_error` | The same through `TemplateFn`, as an `Internal` `FnError` with the `WasmFailure` behind it |
| `fuel_is_refilled_for_every_call` | Fifty calls each use less than the budget |
| `growing_past_the_memory_limit_fails_the_session` and `memory_below_the_initial_size_of_the_module_is_refused` | A refused `memory.grow` is `OutOfMemory` and fails only its session, and a limit below the initial memory fails the load |
| `engine_calls_follow_the_hook_rules` | `send` during `render` returns `not-allowed`, and `random-u64` during `on-close` traps |
| `data_hooks_return_their_outcome`, `a_hook_error_fails_the_session`, `on_open_errors_are_reported` | Pass, consume, and replace outcomes, `send` from `on-data` and `on-timer`, and hook errors with their kind and label |
| `template_calls_reach_the_render_hook`, `unknown_modules_and_functions_are_unsupported`, `random_numbers_come_from_the_session_host`, `a_failing_render_reports_the_hook_error` | Argument conversion of text and bytes, catalog checks before the call, host randomness, and error kinds |
| `the_catalog_is_read_at_load`, `a_component_without_the_world_is_rejected` | The catalog with types and defaults, and an invalid or incomplete component |

The conformance tests that decision record 0010 asks for need a native extension to run against, and none exists yet. The WebAssembly side is covered above.

## Session sampling and capture

ENG-12 added `fps-capture`, which mirrors a seeded sample of sessions into a bounded ring of pcapng files, and exports the TLS key log of each sampled session next to its packets. The agent control protocol of [CON-05](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/changes/CON-05.md) names the commands that AGT-05 serves with this crate, which are `StartCapture` and `StopCapture`.

### Sampling

A `Capture` takes a `SamplePolicy`, which is `Off`, `Rate(fraction)`, or `EveryNth(n)`, and the run seed. `Rate` hashes the seed and the session id with SplitMix64 and compares the result with a threshold, so the sample is a binomial draw that a replay reproduces, and no state is shared between workers. A rate of 0.01 over 10,000 sessions gives 100 sessions on average with a standard deviation of 10, and the acceptance band of 70 to 130 sits at three standard deviations. The unit test `one_percent_of_ten_thousand_lands_in_the_acceptance_band` runs 50 seeds through that band, and the component test counts the SYN frames of the written files. `EveryNth` is exact, with an offset that the seed chooses.

### Cost when sampling is off

`Capture::start` returns `Capture::disabled()` for a policy that never samples. That capture owns no thread, no directory, and no channel. Its `begin` call returns `None` after one branch, with no allocation, lock, or clock read, and `tap` then returns the `Plain` variant of `Tapped`, which forwards every call after one `match`. An enabled capture costs a session that the sampler skips one hash and one compare.

### What the capture records

The engine sees session bytes and no packets, so the capture does not use an AF_PACKET ring. `Tapped` wraps a `SessionIo` and `TappedDatagram` wraps a `DatagramIo`. They record the bytes that pass through, the close of the write side, the end of stream, and the reset, with a time from an injected `CaptureClock`. Wrap the transport below TLS, so the capture holds the encrypted bytes that a packet capture would hold. A sampled session copies its bytes into a recorder that is private to the session, and the recorder goes to a writer thread when the last handle drops. The writer thread synthesizes the frames and writes the file, so the worker pays for one copy. The queue holds 256 finished sessions by default, and a session that finds it full is dropped and counted in `sessions_dropped_queue`.

The synthesis is a pure function of the events, the seed, and the session id.

| Layer | What the capture builds |
|---|---|
| Ethernet | Locally administered addresses that derive from the IP addresses |
| IP | IPv4 with the DF bit and a header checksum, or IPv6, from the endpoints of the session, which must share a family |
| TCP | A three way handshake with an MSS option, sequence numbers from the seed and session id, segments of at most MTU minus 40 or 60 bytes with PSH on the last, FIN with ACK in both directions, or RST when a side aborts, and a close of both sides when the session ends without one |
| UDP | One frame per datagram, an empty datagram included, with the checksum that IPv6 requires, and no fragmentation |

A session records at most `max_session_bytes` of payload, 1 MiB by default. A stream keeps its prefix and still closes, a datagram is kept whole or not at all, and the session counts in `sessions_truncated`.

### The ring

`RingConfig` names the directory, a file prefix, `file_bytes`, and `max_files`. The writer appends whole sessions to the newest file `<prefix>-<index>.pcapng` and opens the next file when a session would pass `file_bytes`. When the file count passes `max_files`, the oldest file goes with its key log. A session never spans two files, so each file decrypts alone, and a session that cannot fit into a file of its own counts in `sessions_dropped_oversize`. A new run adopts the files with the same prefix and continues the numbering, so the bound holds across restarts. Each file is a pcapng section with one Ethernet interface and nanosecond timestamps, and the writer flushes after every session, so a reader that opens a file during the run sees whole blocks.

The key log of a session comes from the caller. Give each sampled TLS session its own in memory `NssKeyLog` of the TLS module, and pass its contents to `SessionHandle::add_key_lines` when the handshake is done. `KeyLogMode` chooses where the lines go. `Embedded` writes a decryption secrets block of type `TLSK` ahead of the packets of its session, `Sidecar` writes `<file>.keys` in the NSS format, and `Both`, the default, writes both, so Wireshark decrypts the file alone and other tools read the log. A key log file and a capture with embedded secrets are as sensitive as a private key.

### Tests

The tests read the files back with a pcapng block reader and a packet parser in `tests/common/`, which share no code with the writer. The parser checks every block length, every IP and transport checksum, and every length field, and the reassembly fails on a gap or an overlap.

| Test | What it checks |
|---|---|
| `ring_rotates_and_every_retained_file_parses` | Thirty sessions rotate a ring of three 24 KiB files, each file stays within the limit, every retained session reassembles to its bytes with a handshake and two FINs, and the key log sidecars match the embedded blocks and no stray file remains |
| `one_percent_of_ten_thousand_sessions_captures_between_70_and_130` | The acceptance criterion, counted in the files |
| `every_nth_sampling_is_exact`, `unsampled_sessions_are_not_recorded` | Exact counts and no recording of a session that the sampler skipped |
| `sampling_off_starts_nothing_and_wraps_nothing` | No directory, no thread, `Plain` wrapping, and zero counters |
| `ipv6_udp_and_abort_frames_parse` | IPv6 TCP with a server side reset, IPv4 UDP up to 65,507 bytes with an empty datagram, and IPv6 UDP |
| `long_sessions_are_truncated_at_the_limit`, `a_full_queue_drops_sessions_and_counts_them` | The prefix rule and the drop counters |
| `key_log_modes_place_the_keys` | Where each mode puts the lines |
| `the_same_seed_and_clock_give_identical_files` | Repeatable bytes |
| `tshark_parses_the_synthesized_streams` | tshark reads HTTP over IPv4 and IPv6 and DNS over UDP in both families, finds two TCP conversations, and reports no checksum, malformed, retransmission, loss, or reordering finding |
| `tshark_decrypts_with_embedded_secrets`, `tshark_decrypts_with_the_sidecar_key_log` | TLS 1.2 and 1.3 sessions of the TLS module decrypt from the embedded blocks alone and from the sidecar alone |
| `the_ring_holds_the_nss_lines_of_every_tls_session` | The key lines in the ring and an encrypted stream, without any tool |

The tshark tests skip themselves when tshark is missing and fail instead when `FPS_REQUIRE_CAPTURE_TOOLS` is `1`, as the TLS capture test does. The tshark check of captured runtime sessions lives in `engine/fps-runtime/tests/capture.rs`, which reuses `tests/common/`, and runs with `cargo test -p fps-runtime --test capture`.

### Open points

1. The cell is `fps_capture::CaptureCell` since ENG-19, and `fps-runtime` reads it for TCP sessions, see [Capture of TCP sessions](#capture-of-tcp-sessions). The agent still has its own copy of the cell in `fps_agent::control` for its UDP workers. It switches to the shared type with one `use` and a re-export, so one cell, taken from `RuntimeHandle::capture_cell()` or passed in with `RuntimeConfig::with_capture`, feeds both the TCP and the UDP workers.
2. The capture filters by sampled session, so the pcap-filter expression and the snap length of `StartCaptureCommand` have no counterpart here, and the AF_PACKET ring that the package text mentions is not built. Synthesized frames hold what the sessions sent, and not the real headers, options, retransmissions, or ICMP of the wire.
3. `StopCaptureResult` carries a `sha256:` digest of one file, and the ring holds several, so AGT-05 must decide which file or archive the digest names.
