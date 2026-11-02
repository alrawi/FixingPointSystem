# Spike S1: Engine benchmark

S1 is a Rust prototype of the agent's flow program interpreter on the kernel socket backend. It answers how many sessions per second and how much throughput one agent core sustains, and how much the interpreter costs compared with hand written code.

## Design

- `template.rs` compiles contract v0.1 templates once per scenario. Rendering appends to a reused buffer, and `fill()` copies from a 1 MiB seeded buffer at a random offset, so payloads differ between sessions.
- `program.rs` compiles each side of a flow into flat operations with jumps. Loops and branches run without recursion or per call allocation.
- `interp.rs` executes operations over any async byte stream, which lets unit tests use in memory pipes.
- `engine.rs` runs emulated servers and closed loop clients on Tokio. Clients bind source addresses across `127.1.0.0/24` with `IP_BIND_ADDRESS_NO_PORT`, which mimics a host pool and spreads ephemeral ports.
- The `baseline` mode replays `http1/web_get` with pre rendered bytes and no interpreter, which isolates interpreter cost.

Out of scope for this spike are UDP flows, transport modules, `spawn`, `raw`, explicit connect hosts, and module helper functions.

## Usage

```bash
cargo run --release -p fps-spike-engine -- check --scenario contracts/examples/modules/http1/scenarios/web_get.yaml
cargo run --release -p fps-spike-engine -- bench \
  --scenario contracts/examples/modules/http1/scenarios/web_get.yaml \
  --concurrency 64 --duration 15 --var body_size=65536 --json out.json
./spikes/s1-engine/run-bench.sh   # full suite, writes results/<host>-<date>/
```

## Results

See `RESULTS.md`.
