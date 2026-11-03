# Agent

This page describes the agent of the AGT workstream. The crate [`fps-agent`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-agent) holds the agent binary. AGT-03 added capability detection, which [Implementation Plan](Implementation-Plan.md) describes, and AGT-01 adds the standalone mode, which runs a plan without a controller. AGT-02 will add the client loop that dials the controller.

## Standalone mode

The command below runs one plan file and exits with the verdict of its criteria.

```bash
fps-agent run --plan contracts/examples/plans/http_dns_smoke.json \
  --modules contracts/examples/modules [--side both|client|server] [--linger SECONDS]
```

| Exit code | Meaning |
|---|---|
| 0 | The run completed and every criterion holds |
| 1 | The run completed and at least one criterion fails |
| 2 | A usage error, an invalid plan, a module or scenario that fails its checks, a feature that the mode does not run, or a failed worker |

The run takes eight steps.

1. Read the plan in the canonical JSON mapping of `fps.agent.v1.Plan` from `contracts/README.md`. The reader rejects unknown fields, reads `seed` as a string or a number, reads durations such as `"30s"`, and checks the references between modules, scenarios, mix entries, pools, and workers.
2. Load every module of the plan from `DIR/<name>`. The manifest must carry the plan's name and version, and the tree digest of the directory must equal the plan digest. The digest follows the rule of `contracts/README.md`, so a symbolic link fails the run.
3. Check the digest of every scenario source, compile it in the call scope of its module with `fps-flow`, apply the mix parameters as variable overrides, and fold constants. An integral JSON number binds as an integer.
4. Register the native template functions of decision D10. A module gets the functions of its crate, `fps-module-http1` or `fps-module-dns`, only when its manifest catalog equals the crate catalog in names, parameter order, types, and required parameters. The run reports every module without native functions on stderr, and calls into such a module fail the session.
5. Turn every plan worker into engine workers, as the next section describes.
6. Start the server workers, then the client workers. TCP flows run on `fps-runtime`. UDP flows run on worker threads of the agent with the datagram openers of `fps-backend-kernel`, with the same session ids, generators, and metric series as the TCP workers.
7. Write one counter snapshot per second to stdout as a JSON line, and a final line with `isFinal` when the run ends.
8. Evaluate the criteria and print the totals of each side, every criterion, and the verdict on stderr.

The standalone mode runs `BACKEND_KERNEL` and scenarios with one flow. Other backends, scenarios with several flows, and external servers named by host name end the run with exit code 2.

### From plan workers to engine workers

An engine worker runs one side of one flow, so one plan worker becomes several engine workers. Every rate of a plan worker gives a client worker, and every mix entry whose server pool has a host range on the plan worker gives a server worker. The engine worker id is the plan worker id shifted left by 8 bits, plus the mix index shifted left by 1 bit, plus 1 on the server side. Plans therefore hold at most 256 workers and 128 mix entries. Every engine worker pins to the core of its plan worker, and a pin that fails leaves the thread unpinned.

| Setting | Value |
|---|---|
| Client source addresses | The plan worker's range of the entry's client pool, picked round robin per session |
| Client destinations | The plan worker's range of the entry's server pool, so a session stays on its worker, or the whole slice when the worker owns no range there |
| Client schedule | The rate as `sessions_per_second` or `concurrent_sessions`, the plan phases, Poisson arrivals, and the stream of the plan worker id and mix index |
| Server listen addresses | Every address of the plan worker's range of the server pool, at the constant port of the scenario's `accept` |
| Topology | `TOPOLOGY_APP_SIM` runs both sides, `TOPOLOGY_CLIENT_SIM` sends to the IP addresses of `externalServers`, and `TOPOLOGY_SERVER_SIM` runs only servers |

Two mix entries that listen on the same transport, address, and port fail the run, because the server could not tell their sessions apart.

The agent does not configure addresses. Every pool address that a worker binds must be local to the namespace of the process, for example through a `local` route on the loopback for the whole prefix, which [Hosts](Hosts.md) calls AnyIP. Port setup with `fps-hosts` moves an interface into a namespace of its own, which needs the process to enter that namespace, and we leave that step to a later package.

### Snapshot lines

Each line is the proto3 JSON mapping of an aggregate `CounterSnapshot` with `planId`, `sequence` from 1, `time`, the `phase` at that moment, and the counters and histograms of the reported side. The metric catalog has no side label, and the two sides of one session must not add up, so a process reports the client side when it runs client workers and the server side otherwise. The counters are cumulative from the start of the run.

### Sides and the end of a run

`--side client` and `--side server` restrict a process to one side, so a test can run the two sides in two network namespaces. A process with client workers ends when every schedule ends and every session closes, and it resets client sessions that are still open 30 seconds after the load phases. A process with server workers only stops them when the load phases end plus the linger time, five seconds by default. Server workers then let open sessions finish within two seconds.

### Criteria

The agent evaluates every criterion over the totals of the reported side. Counters and histogram statistics count from the end of the warmup, which the agent marks with a baseline snapshot at the first tick after `warmup`. Gauges take their final value. The derived ratios `sessions.failed_ratio` and `tls.resumption_ratio` divide their counters as the catalog formula says. A ratio over zero sessions is undefined and fails its criterion. A histogram reference needs one statistic, such as `sessions.duration.p99`. A gauge or histogram that the agent does not record ends the run with exit code 2.

## Tests

| Level | Test | What it shows |
|---|---|---|
| Unit | `src/*.rs` | Plan parsing, durations, criteria, tree digests, native function registration, and a UDP client and server over loopback |
| Component | `tests/standalone.rs` | The example plan on loopback pools with high ports passes with sessions of both modules on both sides, a failed criterion gives fail, a server only run reports the server side, and a changed digest, version, or backend is an error |
| End to end | `tests/netns_smoke.sh`, `make e2e-agt` | The example plan, unchanged, runs in the QA-01 harness with the server side in the server namespace and the client side in the client namespace |

The end to end test routes the plan pools `16.0.0.0/24` and `48.0.0.0/24` through the routing namespace and adds a local route for each pool in its namespace. A local run with a debug build completed 38,107 sessions, 34,299 of `http1` and 3,808 of `dns`, on both sides without a failure, which matches the planned load. The workflow [`agt.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/agt.yml) runs it as root on `ubuntu-24.04`.

## Controller session

AGT-02 added the agent side of the control protocol. The agent dials the controller and serves no gRPC service, as CON-06 decided. The command below registers the agent and serves the commands of the controller until SIGINT or SIGTERM.

```bash
fps-agent connect --controller http://127.0.0.1:7443 --agent-id local \
  --modules contracts/examples/modules [--root DIR] [--side both|client|server] [--label site=lab]
```

The binary detects the host capabilities, adds the installed modules of `--modules` with their tree digests and the `--label` pairs, and calls `Register` with its state and plan. It then opens the `Session` stream with the token that `Register` returned, sends a heartbeat at the interval of the response and at once after every state change, and serves `prepare`, `start`, and `stop`. Logs are JSON lines on standard error from `tracing`. The controller speaks plain TCP until CTL-15 adds mutual TLS, so the agent dials `http://` addresses only.

### Generated types

`build.rs` compiles `contracts/proto/fps/agent/v1/agent.proto` with `protox` and `tonic-prost-build`, so the build needs no `protoc`. The module `fps_agent::pb` holds the messages, the client stub, and the server stub, which the tests use for a fake controller. `fps_agent::convert` turns the `Plan` of a `prepare` command into the plan type of the standalone mode and back, and turns an `fps-stats` snapshot into a `CounterSnapshot` through its proto3 JSON mapping. Both modes therefore run the same plan checks and send the same counters.

### State machine

The module `fps_agent::session` keeps the agent state in one structure behind a mutex that the session task and the run driver thread share. No lock is held across an `.await`.

| Command | Allowed in | Effect |
|---|---|---|
| `prepare` | `IDLE`, `STOPPED`, `FAILED` | `PREPARING` at once, then `PREPARED` after the plan checks, the module digests, and the worker assembly on a dedicated thread |
| `start` | `PREPARED` | `RUNNING` at once, and the run driver thread starts the workers at once or at `start_at` |
| `stop` | `PREPARED` | Releases the plan and answers with `IDLE` |
| `stop` | `RUNNING`, `STOPPING` | A ramp down moves to `STOPPING`, an abort resets every session, and the answer carries the final snapshot once the workers end |
| `stop` | `STOPPED` | Answers again with the final snapshot of the run |

Any other pair fails with `ERROR_CODE_FAILED_PRECONDITION`, and a `plan_id` that does not name the loaded plan fails with `ERROR_CODE_NOT_FOUND`. A plan that breaks a rule or names another agent fails with `ERROR_CODE_INVALID_ARGUMENT`, a module digest, version, or tree that does not match fails with `ERROR_CODE_FAILED_PRECONDITION`, a plan with more workers than cores fails with `ERROR_CODE_RESOURCE_EXHAUSTED`, and a backend or flow kind that the run path lacks fails with `ERROR_CODE_UNIMPLEMENTED`. A failed command leaves the state unchanged. A worker that fails moves the agent to `FAILED`, and the next heartbeat carries the reason in `message`. A run that ends on its own moves to `STOPPED` and sends the final snapshot and a heartbeat.

The run driver thread reuses the run path of the standalone mode. `run::prepare` checks and assembles a plan, `Prepared::start` starts the server workers and then the client workers, and `Active` samples, stops, and finishes the run. The standalone command `fps-agent run` uses the same three steps, so the two modes cannot drift.

### Snapshots, reconnection, and the heartbeat timeout

The driver sends one aggregate `CounterSnapshot` per `counter_interval`, numbered from 1 for each run. A snapshot that the agent takes while the connection is down waits in the agent and goes out on the next session, so the sequence continues across sessions and the controller drops duplicates. Command results belong to the session that received the command, and the agent drops them when that session ends.

After a lost connection the agent dials again with exponential backoff from 200 ms up to 10 s and calls `Register` with its current state and plan, so the controller resumes the run. A session that opened resets the backoff, except one that ended with `ABORTED`, which means that another session of the same identity replaced it. A watchdog compares the time since the last message from the controller with the heartbeat timeout of `Register`. When the timeout passes, the agent aborts any run, releases the plan, moves to `IDLE`, ends the session, and dials again.

### Session tests

| Level | Test | What it shows |
|---|---|---|
| Unit | `src/convert.rs` | The example plan survives the round trip through the generated types, and unknown enum numbers, negative durations, and list parameters are errors |
| Component | `tests/session.rs` | An in process fake controller drives prepare, start, and stop over loopback and receives numbered cumulative snapshots and a final snapshot, out of order calls fail with their codes, a bad plan leaves the state unchanged, a run continues across a reconnection and stops on its own, and a silent controller makes the agent release the plan |
| End to end | `tests/controller_e2e.sh FPS_CONTROLLER FPS_AGENT` | The real `fps-controller` enrolls the real agent, installs the example modules, and drives a loopback run over REST to `completed` with a passed verdict |

A local run of the end to end script with debug builds completed its run with 148 succeeded `http1` sessions in three seconds at 40 sessions per second. The controller checks port locations against the agent capabilities, which leave out loopback, so the script names the first detected interface while the loopback pools carry the traffic.

### Limits

- `per_worker` snapshots are not implemented. The agent logs a warning and sends aggregate snapshots.
- The agent does not bind ports during `prepare`. The workers bind their sockets when the run starts.
- The backoff has no jitter.
- Mutual TLS waits for CTL-15 on the controller side.
