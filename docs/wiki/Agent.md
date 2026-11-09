# Agent

This page describes the agent of the AGT workstream. The crate [`fps-agent`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-agent) holds the agent binary. AGT-03 added capability detection, which [Implementation Plan](Implementation-Plan.md) describes, and AGT-01 adds the standalone mode, which runs a plan without a controller. AGT-02 added the client loop that dials the controller, which the section [Controller session](#controller-session) describes, and AGT-06 added the kernel requirement and the redial policy, which the sections [Kernel requirement](#kernel-requirement) and [Redial policy](#redial-policy) describe. AGT-05 added the handlers of the v0.2 commands `UpdateRates`, `StartCapture`, and `StopCapture`, which the section [Run time rates and captures](#run-time-rates-and-captures) describes. AGT-08 added mutual TLS, enrollment, and renewal, which the section [Mutual TLS and enrollment](#mutual-tls-and-enrollment) describes. AGT-07 applies module settings and reports every session series with the `side` label, which the sections [Module settings](#module-settings), [Snapshot lines](#snapshot-lines), and [Criteria](#criteria) describe.

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
| 2 | A usage error, an invalid plan, a module or scenario that fails its checks, a feature that the mode does not run, a failed worker, or a kernel before Linux 6.3 |

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

### Module settings

`ModuleRef.settings` (CON-11) holds `modules.<name>.settings` of the test configuration, which the controller validated against the settings schema of the manifest. The agent applies them when it loads the plan, in `modules::NativeFunctions::for_modules`. The `dns` module reads `zone_file` (MOD-19): a relative path resolves against the module directory `<modules dir>/dns`, and the server answers from that zone instead of the shipped one.

A setting path may not leave the module directory. The contract does not allow paths outside it, so the agent refuses a path with a `..` component and any path whose canonical form (symbolic links resolved) lies outside the module directory, absolute paths included. An absolute path that names a file inside the module directory works. The failure ends `prepare` with `InvalidArgument` and a message such as `module dns: settings: zone_file: "../x.zone" has a .. component, which could leave the module directory`. A missing file, a zone with a syntax error, an unknown key, and a value of the wrong type end `prepare` the same way.

The agent refuses settings that it cannot apply, and never skips them in silence, because a run that ignores them measures another test. That covers a module that has no native functions in this agent, such as a module whose manifest catalog differs from its crate, and a module whose native functions take no settings, such as `http1`. A module without settings behaves as before.

### Snapshot lines

Each line is the proto3 JSON mapping of an aggregate `CounterSnapshot` with `planId`, `sequence` from 1, `time`, the `phase` at that moment, and the counters, gauges, and histograms of every side that the process runs. Catalog 0.2.0 gives every scenario and transport series the `side` label, so the client series and the server series of one session sit side by side in the line and do not add up. The UDP workers of the agent register their series through `fps_stats::SessionCounters`, as the TCP workers of `fps-runtime` do, so both transports label their series alike. A process that runs one side only, as with `--side client`, carries that side. A sum over every label of one line counts each session of a two sided process twice, so a reader selects `side` first. The counters are cumulative from the start of the run.

### Sides and the end of a run

`--side client` and `--side server` restrict a process to one side, so a test can run the two sides in two network namespaces. A process with client workers ends when every schedule ends and every session closes, and it resets client sessions that are still open 30 seconds after the load phases. A process with server workers only stops them when the load phases end plus the linger time, five seconds by default. Server workers then let open sessions finish within two seconds.

### Criteria

The agent evaluates every criterion over the series of one side, which is the client side by default, as CON-08 asks, because a total over both sides counts each session twice. A process that runs no client workers has no client series, so it counts the server side. A criterion carries no labels on the wire (CON-11 leaves them out), so it cannot name a side itself yet. A metric without the `side` label counts as it is. Counters and histogram statistics count from the end of the warmup, which the agent marks with a baseline snapshot at the first tick after `warmup`. Gauges take their final value. The derived ratios `sessions.failed_ratio` and `tls.resumption_ratio` divide their counters as the catalog formula says. A ratio over zero sessions is undefined and fails its criterion. A histogram reference needs one statistic, such as `sessions.duration.p99`. A gauge or histogram that the agent does not record ends the run with exit code 2.

## Tests

| Level | Test | What it shows |
|---|---|---|
| Unit | `src/*.rs` | Plan parsing, durations, criteria on both sides of a snapshot, tree digests, native function registration with module settings and the path checks, and a UDP client and server over loopback whose series carry the side label |
| Component | `tests/standalone.rs` | The example plan on loopback pools with high ports passes with sessions of both modules on both sides, and every series of a snapshot line carries the `side` label with the client and the server series next to each other. A failed criterion gives fail, a server only run reports the server side, and the criteria count the client side, which is half of a total over both sides. A changed digest, version, or backend is an error. A plan with the `zone_file` setting of a copy of the dns module passes because the zone answers its name, and the same plan without the setting fails. A setting with `..`, a path outside the module directory, or a missing file ends the run |
| End to end | `tests/netns_smoke.sh`, `make e2e-agt` | The example plan, unchanged, runs in the QA-01 harness with the server side in the server namespace and the client side in the client namespace |

The end to end test routes the plan pools `16.0.0.0/24` and `48.0.0.0/24` through the routing namespace and adds a local route for each pool in its namespace. A local run with a debug build completed 38,107 sessions, 34,299 of `http1` and 3,808 of `dns`, on both sides without a failure, which matches the planned load. The workflow [`agt.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/agt.yml) runs it as root on `ubuntu-24.04`.

## Controller session

AGT-02 added the agent side of the control protocol. The agent dials the controller and serves no gRPC service, as CON-06 decided. The command below registers the agent and serves the commands of the controller until SIGINT or SIGTERM.

```bash
fps-agent connect --controller http://127.0.0.1:7443 --agent-id local \
  --modules contracts/examples/modules [--root DIR] [--side both|client|server] [--label site=lab] \
  [--redial-limit SECONDS] [--capture-dir DIR]
  [--cert-dir DIR [--tls-ca FILE] [--tls-ca-sha256 HEX] [--enroll-address HOST:PORT] \
   [--enroll-token-file FILE | --enroll-token-env NAME]]    # https:// controllers
```

The binary detects the host capabilities, adds the installed modules of `--modules` with their tree digests and the `--label` pairs, and calls `Register` with its state and plan. It then opens the `Session` stream with the token that `Register` returned, sends a heartbeat at the interval of the response and at once after every state change, and serves `prepare`, `start`, `stop`, `update_rates`, `start_capture`, and `stop_capture`. Logs are JSON lines on standard error from `tracing`. Since CTL-27 the controller requires mutual TLS unless it runs with `insecure_plain_grpc`, so the agent dials an `https://` address with a certificate, see [Mutual TLS and enrollment](#mutual-tls-and-enrollment), and an `http://` address only against a controller in that mode.

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
| `update_rates` | `RUNNING` | Sets the target of the whole agent for each named mix entry, see [Run time rates and captures](#run-time-rates-and-captures) |
| `start_capture` | `RUNNING` | Starts a capture of the sessions that begin from now on |
| `stop_capture` | Any state that holds the plan | Ends the capture and answers with the digest of its file |

Any other pair fails with `ERROR_CODE_FAILED_PRECONDITION`, and a `plan_id` that does not name the loaded plan fails with `ERROR_CODE_NOT_FOUND`. A plan that breaks a rule or names another agent fails with `ERROR_CODE_INVALID_ARGUMENT`, a module digest, version, or tree that does not match fails with `ERROR_CODE_FAILED_PRECONDITION`, a plan with more workers than cores fails with `ERROR_CODE_RESOURCE_EXHAUSTED`, and a backend or flow kind that the run path lacks fails with `ERROR_CODE_UNIMPLEMENTED`. A failed command leaves the state unchanged. A worker that fails moves the agent to `FAILED`, and the next heartbeat carries the reason in `message`. A run that ends on its own moves to `STOPPED` and sends the final snapshot and a heartbeat.

The run driver thread reuses the run path of the standalone mode. `run::prepare` checks and assembles a plan, `Prepared::start` starts the server workers and then the client workers, and `Active` samples, stops, and finishes the run. The standalone command `fps-agent run` uses the same three steps, so the two modes cannot drift.

### Snapshots, reconnection, and the heartbeat timeout

The driver sends one aggregate `CounterSnapshot` per `counter_interval`, numbered from 1 for each run. A snapshot that the agent takes while the connection is down waits in the agent and goes out on the next session, so the sequence continues across sessions and the controller drops duplicates. Command results belong to the session that received the command, and the agent drops them when that session ends.

After a lost connection the agent dials again with exponential backoff from 200 ms up to 10 s and calls `Register` with its current state and plan, so the controller resumes the run. The [Redial policy](#redial-policy) section describes the jitter, the reset after an abort, and the limit. A watchdog compares the time since the last message from the controller with the heartbeat timeout of `Register`. When the timeout passes, the agent aborts any run, releases the plan, moves to `IDLE`, ends the session, and dials again.

### Kernel requirement

The agent needs the socket option `IP_LOCAL_PORT_RANGE`, which Linux offers from release 6.3, so that every worker can take source ports from its own range. Decision D12 item 3 lets the agent refuse an older kernel. `fps-agent connect` and `fps-agent run` read `proc/sys/kernel/osrelease` under the same injectable `SystemRoot` as capability detection, before they detect anything else or dial. A release before 6.3 ends the process with exit code 2 and this message on stderr, with the release of the host in place of `5.4.0-150-custom`.

```text
fps-agent: error: kernel 5.4.0-150-custom is too old: the agent needs the IP_LOCAL_PORT_RANGE socket option, which requires Linux 6.3 or newer (decision D12 item 3)
```

An unreadable or malformed release is an error too, and the check never passes on a guess. The command `fps-agent capabilities` still reports any kernel, so an operator can inspect an old host. Distribution kernels that backport the option under an older version number are refused as well, because the check compares versions and does not probe the option.

### Redial policy

Decision D12 item 11 says that an agent keeps redialing after an abort, with a backoff that restarts at 200 ms with jitter and with a limit. The type `fps_agent::session::RedialPolicy` holds the rules, and `serve` applies it after every ended session.

| Rule | Behavior |
|---|---|
| Backoff | The wait doubles from 200 ms up to 10 s |
| Jitter | Every wait is drawn uniformly from the upper half of the step, so a 200 ms step waits 100 to 200 ms and agents that lost the controller together do not dial together. The generator is seeded from the clock at process start |
| Reset | A session that opened and then ended, and every session that ended with `ABORTED`, send the backoff back to 200 ms. `ABORTED` means that another session of the same identity replaced this one |
| Limit | Failed attempts may last for `--redial-limit SECONDS`, which defaults to 300. A session that opened and ended without an abort clears the window. A failed dial and an abort start the window when none runs and leave it running otherwise, so two agents of one identity that replace each other forever still reach the limit. The value 0 means no limit |
| Exit | At the limit the agent aborts any run, releases the plan, logs `redial limit reached, the agent exits`, and exits with code 3 |

| Exit code of `fps-agent connect` | Meaning |
|---|---|
| 0 | SIGINT or SIGTERM stopped the agent |
| 2 | A usage error, an invalid controller address, a kernel before Linux 6.3, a failed capability detection, or a failure that redialing cannot fix, see [Mutual TLS and enrollment](#mutual-tls-and-enrollment) |
| 3 | The agent gave up redialing the controller after the redial limit |

A supervisor such as systemd can restart on code 3 and treat code 2 as a configuration fault.

### Mutual TLS and enrollment

AGT-08 gave the agent the TLS side of the controller (CTL-15 and CTL-27). A controller address with the scheme `https://` selects mutual TLS, and `http://` stays plain TCP for a controller that runs with `insecure_plain_grpc`. The module `fps_agent::identity` holds the code, and the tonic channel uses rustls with the ring provider, the stack of `fps-module-tls`. The owner chose the HTTPS enrollment endpoint, so no contract changed.

```bash
fps-agent connect --controller https://ctl.lab.example:8444 --agent-id edge-1 \
  --modules /var/lib/fps/modules --cert-dir /var/lib/fps-agent/pki \
  --tls-ca /etc/fps/ca.crt --enroll-token-file /etc/fps/enroll.token
```

| Flag | Default | Meaning |
|---|---|---|
| `--cert-dir DIR` | none | The directory of the key and the certificate. An `https://` controller needs it |
| `--tls-ca FILE` | none | A PEM file with the CA certificate of the controller, from `pki_dir/ca.crt` |
| `--tls-ca-sha256 HEX` | none | The SHA-256 fingerprint of the CA certificate, which the controller logs as `ca_sha256`. Colons, upper case, and a `sha256:` prefix are fine |
| `--enroll-address HOST:PORT` | host of `--controller`, port 8445 | The HTTPS listener of the controller for `POST /enroll` and `POST /renew` (`enroll_listen`) |
| `--enroll-token-file FILE` | none | A file that holds the enrollment token |
| `--enroll-token-env NAME` | `FPS_ENROLL_TOKEN` | The environment variable that holds the token when no file is named |

An `https://` controller needs `--cert-dir` and one of `--tls-ca`, `--tls-ca-sha256`, or a `ca.pem` in the directory from an earlier enrollment. An `http://` controller refuses every TLS flag, which keeps a plain connection from passing for a secure one. The token never travels on the command line, and `fps-agent` has no flag that takes it.

#### The files

The directory has mode 0700 when the agent creates it. Every file is written to a temporary name and renamed, so a reader sees the old file or the new one, and the agent refuses a key that group or others can read.

| File | Mode | Content |
|---|---|---|
| `agent.key` | 0600 | The ECDSA P-256 private key in PKCS 8 PEM |
| `agent.crt` | 0644 | The certificate that the controller issued, with the common name set to `--agent-id` |
| `identity.json` | 0644 | `not_after` and `renew_at` of the certificate as Unix seconds |
| `ca.pem` | 0644 | The CA certificate, written only by an enrollment that pinned a fingerprint |

#### Enrollment

At every dial the agent reads the directory. Without a valid certificate, or with a token file that exists, it enrolls first, which takes six steps.

1. It reads the token from the file or the environment variable. Without one it exits with the error `no enrollment token`, which names `fps-controller token` and both sources.
2. It makes an ECDSA P-256 key and a CSR with the common name `--agent-id` and no other name.
3. It connects to the enrollment address over TLS with the CA of the trust rules below, and sends `POST /enroll` with the token and the CSR.
4. It checks that the answer names its own identity, and it parses `not_after` and `renew_at`.
5. It writes the key, `identity.json`, and the certificate.
6. It deletes the token file or removes the environment variable, so the agent keeps no spent secret.

A token file that exists at a dial means that the operator wants a new enrollment, for example after `fps-controller revoke`, so the agent enrolls again over a valid certificate. The agent deletes the file after every enrollment, which makes a leftover file always new. A token in the environment enrolls only when no valid certificate exists, because the variable survives a restart.

#### Trust

The CA is the only trust anchor of both connections, and the agent never reads the system roots. It resolves the CA in this order: the file of `--tls-ca`, the `ca.pem` that an earlier enrollment stored, and the fingerprint of `--tls-ca-sha256`. With a CA file and a fingerprint, the file must hold a certificate with that fingerprint.

A fingerprint alone has a weak spot. The controller sends only its leaf certificate, so the agent cannot verify the enrollment endpoint before it has the CA. The first exchange therefore accepts the certificate of the endpoint, still checking the handshake signature, and the agent keeps the answer only when the `ca_pem` of the answer has the pinned fingerprint and the certificate that the endpoint presented chains to it and names the address that the agent dialed. A rogue endpoint cannot make the agent trust it, but it can read the token, which then fails or succeeds at the real controller as a race. A CA file closes that gap, and the follow up in the log entry of AGT-08 proposes that the controller send its CA in the chain.

#### Renewal

A task checks `identity.json` and calls `POST /renew` when `renew_at` has passed. The request presents the current certificate over mutual TLS and carries a fresh CSR for the same key, because the files change one at a time and a new key would leave a window with a mismatched pair. The renewal runs on its own HTTPS connection and never touches the open session, so the agent never redials just to renew, and decision D12 item 12, which drops the plan with a lost session, holds. A renewal during a run is harmless, and it keeps a long run from outliving its certificate. The next dial reads the new files. A failed renewal retries with a wait from 5 s to 60 s until the certificate expires. When the controller refuses the renewal, the agent logs the reason, and the next dial enrolls again if a new token exists.

#### Permanent failures

Redialing cannot fix a handshake failure, `UNAUTHENTICATED`, a refused token, or a missing trust anchor, so the agent aborts any run, releases the plan, logs `the failure is permanent, the agent exits` with the reason, prints `fps-agent: error:` and the reason, and exits with code 2, which a supervisor reads as a configuration fault.

| Failure | Where it shows | Message starts with |
|---|---|---|
| The CA does not match the controller certificate, or the peer does not speak TLS | The dial | `TLS failure: invalid peer certificate: UnknownIssuer` and similar |
| The controller refuses the certificate of the agent, for example after a revocation | The first call after the dial, as an alert of TLS 1.3 | `TLS failure: received fatal alert: BadCertificate` |
| `Register` answers `UNAUTHENTICATED` | The first call | `the controller answered UNAUTHENTICATED` |
| The token is unknown, used, or expired | `POST /enroll` | `the controller refused the enrollment token (HTTP 401)` |
| The CA of the answer has another fingerprint than the pin | `POST /enroll` | `the CA certificate does not have the pinned fingerprint` |
| No certificate and no token | The dial | `no enrollment token is available` |

A pinned fingerprint that does not match leaves the token spent at the controller, because the agent learns the CA from the answer, and it keeps the token file for a retry with a new token. `PERMISSION_DENIED`, which means that the inventory does not list the agent, and every unreachable controller still redial with the redial limit, because an operator can fix both while the agent runs.

### Run time rates and captures

AGT-05 serves the commands that CON-05 added in protocol 0.2.0. The agent announces `engine_api` 0.2.1, so the controller may send them.

#### UpdateRates

The command is valid in `RUNNING` only, once the workers run. A `plan_id` that does not name the loaded plan fails with `ERROR_CODE_NOT_FOUND`. The handler turns every `RateUpdate` into one `fps_sched::RateControl::update` entry. The target is the rate or the concurrency of the whole agent for the mix entry, and each registry divides it among the workers of the entry in proportion to their current targets. A mix entry that the command does not name keeps its targets, a target of 0 pauses an entry, and a positive one resumes it.

Mix entries run in two places. `fps-runtime` registers the rate handles of the TCP client workers, which `RuntimeHandle::rate_control()` returns, and `assemble.rs` now sets `FlowRef::entry` to the mix entry id, so two entries that share a scenario do not collide. The UDP workers of `src/udp.rs` build their schedules outside `fps-runtime`. They build the schedule before they report ready, register its handle in a registry of their own that `UdpRuntime::rate_control()` returns, and sleep at most 100 ms, so a paused or capped worker sees a new target within that time, as the TCP workers do. We chose to register the UDP handles, so `UpdateRates` accepts UDP entries too. `fps_agent::control::Rates` joins the two registries.

The command applies completely or not at all. `Rates::update` checks every entry, kind, and value across both registries before it stores anything, and each refusal is `ERROR_CODE_INVALID_ARGUMENT` with a message that names the entry.

| Refusal | Message names |
|---|---|
| The entry is not in the plan | `is not a mix entry of the plan` |
| The plan has the entry, but no client worker of this agent runs it, as on a `--side server` agent | `has no client worker with a rate on this agent` |
| One entry twice | `is named twice` |
| `sessions_per_second` for a `concurrent_sessions` entry, or the reverse, or no target | `runs concurrent_sessions` or `runs sessions_per_second` |
| A rate below 0 or not finite | `is not a finite number of at least 0` |

`applied_at` is the agent time at which the registries took the targets. The workers apply them on their next tick, at most 100 ms later.

#### StartCapture

The command is valid in `RUNNING` only. The capture needs a `capture_id` of 1 to 128 letters, digits, dots, underscores, and hyphens that does not start with a dot, because the id names files, and a `capture_id` that is already in use in the plan fails with `ERROR_CODE_FAILED_PRECONDITION`. The agent runs one capture at a time, and a second `StartCapture` while one runs fails with `ERROR_CODE_FAILED_PRECONDITION` as well.

The engine sees session bytes and no packets, so the agent captures sessions and not an interface. `fps-capture` samples every session (`EveryNth(1)`), synthesizes Ethernet, IP, and transport frames around the bytes, and writes them into a bounded ring. A capture that starts in the middle of a run reaches the workers through `fps_agent::control::CaptureCell`, a cell with one atomic flag and the current `Capture`. A worker reads it once per session, when the session opens, so a run without capture pays one atomic load per session, and a session that opens right after `StartCapture` is recorded. A session that was already open is not.

| Field of the command | What the agent does |
|---|---|
| `port` | Empty only. A name that is not a port of the plan is `ERROR_CODE_INVALID_ARGUMENT`. A port of the plan is `ERROR_CODE_UNIMPLEMENTED`, because the engine cannot tell the ports of a plan apart |
| `filter` | Empty only. The agent has no pcap-filter compiler, so any expression is `ERROR_CODE_INVALID_ARGUMENT`, the code that the contract gives for a filter that the agent cannot compile |
| `snap_length` | 0 only. A positive value is `ERROR_CODE_UNIMPLEMENTED`, because the capture keeps whole sessions |
| `max_bytes` | Ends the capture once the packets written hold this many bytes, without framing. 0 means no limit |
| `max_duration` | Ends the capture after this time. Absent or 0 means no limit |

The agent refuses what it cannot honor, so it never records more than the controller asked for. The ring holds at most `file_bytes * max_files` of packets, 64 MiB by 8 files, so the oldest sessions of a long capture leave the file. Sessions that the writer queue cannot take, or that do not fit a ring file, are dropped and counted.

Both sides of a conversation are in one capture when the agent runs client workers, because a client session holds both directions. A process with server workers only, such as `--side server`, captures its server sessions. Key log lines go into the file as decryption secrets blocks, so a capture with TLS sessions is as sensitive as a private key. The agent creates its capture directory with mode 0700.

#### StopCapture

The command names the plan and the capture. It is valid in every state that holds the plan, and it is idempotent: a capture that ended at a limit, at the end of the run, or at an earlier `StopCapture` answers again with the same result. A `plan_id` or `capture_id` that the agent does not know fails with `ERROR_CODE_NOT_FOUND`. A thread ends the capture and answers when the file is ready, so a long join does not hold up the heartbeat. The run end ends every capture too, after the workers ended and every session reached the writer.

**What the digest covers.** `StopCaptureResult` carries one digest, and the ring holds several files. The agent joins the ring files, oldest first, into one pcapng file by concatenating them, which the pcapng format allows because every ring file is a complete section. `bytes` is the size of that file, `digest` is `sha256:` and the lowercase hex SHA-256 of its bytes, and `packets` counts its enhanced packet blocks. The file is the deterministic result of the ring, so the digest does not depend on a tar header or a clock. A capture with no session is a valid file with a section header and an interface description. `dropped` counts sessions that the capture could not record, because the controller needs a number and the agent cannot know how many packets a dropped session held. Sessions that rotated out of a full ring are not in the file and not in `packets`. The ring files are removed once the joined file is written.

**How the controller gets the file.** CON-13 defines the transfer, which the owner chose to run over the agent connection: the controller sends `fetch_capture` with a start offset and a single use transfer id, and the agent opens the client streaming `UploadCapture` call and sends the file in chunks of at most 256 KiB, ending with the size and the digest of `StopCaptureResult` ([Contracts](Contracts.md)). The agent reports `engine_api` 0.2.2 and answers `fetch_capture` with UNIMPLEMENTED until AGT-09 serves it. The file stays on the agent at the path below, and the agent logs the path, the packets, and the digest at `info` level (`capture file ready`). The directory is `--capture-dir`, which defaults to `fps-agent-captures` under the temporary directory of the process.

```text
<capture dir>/<plan id with each other byte as _>-<8 hex digits of sha256(plan id)>/<capture id>.pcapng
```

The function `fps_agent::control::capture_path` computes it. Starting a capture removes the files of an earlier capture with the same plan and capture id, so a reused id never mixes two captures.

A failure to write the file is `ERROR_CODE_RESOURCE_EXHAUSTED`, never `ERROR_CODE_INTERNAL`, because the contract moves an agent to `FAILED` after `ERROR_CODE_INTERNAL`, and a full disk must not end a healthy run.

#### TCP sessions are not captured yet

The TCP workers belong to `fps-runtime`, which builds its openers with the kernel stream type and has no hook for a tap. The cell reaches the UDP workers of the agent only, so a capture of a plan with TCP flows holds its UDP flows, and a capture of a TCP only plan holds no session. A runtime change must read the cell when a client or server session opens and wrap its `KernelStream` with `Capture::tap`. This is the first open point of the Engine page, and the agent code that fills the cell is ready for it.

### Session tests

| Level | Test | What it shows |
|---|---|---|
| Unit | `src/convert.rs` | The example plan survives the round trip through the generated types, and unknown enum numbers, negative durations, and list parameters are errors |
| Component | `tests/session.rs` | An in process fake controller drives prepare, start, and stop over loopback and receives numbered cumulative snapshots and a final snapshot, out of order calls fail with their codes, a bad plan leaves the state unchanged, a run continues across a reconnection and stops on its own, and a silent controller makes the agent release the plan |
| Component | `tests/run_control.rs` | A fake controller changes the rates of a TCP entry and a UDP entry of a running plan, and the new rates show in the snapshots, refused updates change nothing, a pause and a resume work, and the capture commands answer with the codes above. The controller starts a capture in the middle of the run, stops it, locates the file below the capture directory, reads it block by block, and checks the size, the SHA-256, and the packet count of the result. A second capture ends at its time limit with no `StopCapture`, and a third ends with the run and answers after it |
| Unit | `src/control.rs` | An update spans both registries and keeps entries that it does not name, every refusal changes nothing, and plan ids with a separator map to flat, distinct directories |
| Component | `tests/detect.rs`, `tests/redial.rs`, `tests/exit_codes.rs`, `make test-agt-redial` | A 5.4 kernel string gives an error that names `IP_LOCAL_PORT_RANGE`, and 6.3 and newer pass. The backoff doubles, restarts at 200 ms with jitter after an abort, and resets after a session that opened. Failed attempts end `serve` with `RedialLimit` at the limit, aborts alone reach the limit too, and the binary exits with 2 and 3 |
| Component | `tests/mtls.rs` | A fake enrollment endpoint and a fake controller over mutual TLS, both made with rcgen. The agent enrolls with a token from a file, which it deletes, registers with a certificate that names it, and renews at `renew_at` without a new session. A pinned fingerprint replaces the CA file and a stored CA serves the next start. A new token file replaces a certificate. A wrong pin, a refused token, a missing token, a controller of another CA, a certificate that the controller refuses, `UNAUTHENTICATED`, and a peer that does not speak TLS each end the agent with a permanent error, an unreachable controller does not, and the binary exits with code 2 and a TLS message |
| End to end | `tests/controller_e2e.sh FPS_CONTROLLER FPS_AGENT` | The real `fps-controller` mints a token with `fps-controller token`, the real agent enrolls with it over HTTPS and registers over mutual TLS, installs the example `http1` module and the first party `dns` module, and drives a loopback run over REST to `completed` with a passed verdict. A dns test with the `zone_file` module setting passes, and the same test without it fails. An agent that trusts another CA ends with exit code 2 before it spends its token, and after `fps-controller revoke` the running agent ends with exit code 2 and enrolls again with a new token |

A local run of the end to end script with debug builds completed its run with 148 succeeded `http1` sessions in three seconds at 40 sessions per second. A second run follows `contracts/examples/tests/dns_zone.yaml` with the first party `dns` module: its `zone_file` setting names a zone for `www.e2e.test` that the script adds to the module directory, the lookups ask for that name, and the run passes only when the agent applied the setting. The same test without the setting fails its criteria. The script installs the same module trees in the controller and in the agent directory, because the agent checks the tree digest of every module. The controller checks port locations against the agent capabilities, which leave out loopback, so the script names the first detected interface while the loopback pools carry the traffic.

### Limits

- `per_worker` snapshots are not implemented. The agent logs a warning and sends aggregate snapshots.
- The agent does not bind ports during `prepare`. The workers bind their sockets when the run starts.
- The redial limit counts time and not attempts, and a dial that hangs for its five second connect timeout can overshoot it by that much.
- The agent renews with the key that it enrolled with, and it never rotates the key. A new key needs a cert and key pair that one rename switches.
- A fingerprint without a CA file leaves the token open to a rogue enrollment endpoint for the first exchange, see [Trust](#trust).
- TCP sessions are not captured until `fps-runtime` reads the capture cell, see [Run time rates and captures](#run-time-rates-and-captures). Filters, ports, and snap lengths are refused, and the capture file stays on the agent until a later package defines the transfer.
