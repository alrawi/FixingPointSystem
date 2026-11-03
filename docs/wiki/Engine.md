# Engine

This page describes the Rust crates of the agent engine under `engine/`, which the ENG workstream owns. ENG-01 created the first four crates. Later ENG packages fill them and add the scheduler, runtime, extension host, and capture crates that the [Implementation Plan](Implementation-Plan.md#target-repository-layout) lists.

## Crates

The workspace glob in the root `Cargo.toml` picks up every directory under `engine/`, so a new crate needs no edit there. The table lists the crates that exist today.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-io`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-io) | The `SessionIo` trait and the in memory duplex `fps_io::memory::duplex` | ENG-06 adds `DatagramIo` |
| [`fps-flow`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-flow) | The scenario model, the loader, the program compiler, and the template compiler of contract v0.1 | ENG-02 adds the semantic checks, ENG-03 the module functions |
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
3. Rendering takes any `rand::Rng`, and the filler buffer comes from `rand_pcg::Pcg64Mcg`, which gives the same bytes on every platform. The spike used `SmallRng`, whose algorithm may change between platforms and releases.
4. The contract test `tests/contract_examples.rs` compiles every scenario under `contracts/examples/modules/`, so a new example joins the check on its own.

The crate still rejects transport modules, spawned flows, the `spawn` and `raw` actions, and module helper functions with `LoadError::Unsupported` or `TemplateError::ModuleHelper`.

## Known duplication

Spike S1 in `spikes/s1-engine` keeps its own copies of `program.rs` and `template.rs`, so its benchmark harness builds and reproduces the Gate 0 numbers unchanged. The engine crates are the source of truth for new work. A later package may switch the spike to the engine crates or retire it, and until then a fix in one copy does not reach the other.

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
