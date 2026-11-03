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
