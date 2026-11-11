# ntp module

The `ntp` module generates NTP version 4 exchanges (RFC 5905) between a client and a server on UDP port 123. The packets are real. Every request and every reply has the 48 byte header with a leap indicator, version, mode, stratum, poll exponent, precision, root delay, root dispersion, reference identifier, and the four 64 bit timestamps, and a client that parses the reply finds the origin timestamp that it sent. The module needs no zone, key, or state, so its functions are a unit value that every session shares.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Five scenarios with one flow each |
| `native/` | The crate `fps-module-ntp`, with the codec in `src/wire.rs`, and the `fixture_peer` example |
| `tests/` | Catalog, scenario, and golden transcript tests |
| `fixtures/` | `capture.sh`, `expected.yaml`, the committed capture `ntp.pcapng` and its realism report |

## Template functions

| Function | Purpose |
|---|---|
| `query(now_ms, version=4, poll=6, mode=3, stratum=0, reference='')` | A request of 48 bytes. Mode 3 is a client, and mode 1 is a symmetric active peer, which also states its stratum, root error, reference identifier, and the time of its last clock update. A client with stratum 0 sends only the header and its transmit timestamp, as an SNTP client does |
| `reply(request, now_ms, stratum=2, reference='', kiss='')` | The server packet for a request, in mode 4 for a client and mode 2 for a peer. It copies the version, clamps the poll exponent to 4 through 17, copies the transmit timestamp of the request into its origin timestamp, and stamps its receive and transmit times. A non empty `kiss` makes it a Kiss o' Death packet |
| `field(message, name)` | One field of a packet as text, for a `set` action. The names are `version`, `mode`, `leap`, `stratum`, `poll`, `precision`, `reference_id`, `root_delay_ms`, `root_dispersion_ms`, and `transmit_ms` |

The manifest lists the parameters of every function, and `tests/catalog.rs` compares the list with the catalog of the crate.

### Time

The functions never read a clock. A scenario passes `now('epoch_ms')`, the built in function of scenario contract v0.6, so the engine clock decides every timestamp. A production run reads the wall clock of the host, and a test pins the clock, as `tests/common/mod.rs` does for the golden transcripts. The golden files hold timestamps and therefore depend on the pinned instant, which `tests/common/mod.rs` names (2026-11-10T12:00:00Z for the client and 23 ms later for the server, one second more with every read).

The engine clock ticks in milliseconds. The functions draw the position inside the millisecond from the session generator, so a timestamp lies within one millisecond of the clock. A capture can therefore show a server timestamp up to a millisecond later than the frame that carries it. Real clocks differ by more than that, and the field is no finer than that on a clock that the engine reads in milliseconds.

The timestamps cover 1900 through February 2036, the 32 bit seconds of NTP era 0. A `now_ms` outside it is an argument error and never wraps.

### The server

A server keeps its character from reply to reply. The precision, the root delay and dispersion, and the phase of its clock updates follow from its reference identifier, so every reply of one server agrees, and a jitter of up to 300 microseconds on the root error and 20 to 300 microseconds between receive and transmit time come from the session generator.

* A stratum 1 server reports no root delay and a small dispersion, and its reference identifier defaults to `GPS`. A secondary server reports both in proportion to its distance from the primary source, and its reference identifier defaults to the documentation address `203.0.113.9` (RFC 5737).
* The reference timestamp is the last clock update. It moves forward once every 64 seconds for a stratum 1 server and once every 1024 seconds for a secondary server, and it never goes back.
* A Kiss o' Death reply (RFC 5905 section 7.4) has leap indicator 3, stratum 0, the code as its reference identifier, no root error, and no reference timestamp. It copies the origin timestamp and stamps the receive and transmit times, so a client can match it to its request.

## Scenarios

| Scenario | Shape |
|---|---|
| `sntp_query` | One exchange. The client sends version 4 with the poll exponent 0 and only a transmit timestamp, and the server answers as stratum 2 with the exponent raised to 4 |
| `iburst` | Four requests with the poll exponent 6, `burst_gap_ms` (2 seconds) apart, as a daemon sends them at start |
| `poll_cycle` | A request with the exponent 6, a pause of `poll_interval_ms` (64 seconds), a second request with 6, a pause of `poll_interval_long_ms` (128 seconds), and a third with 7. The server echoes each exponent |
| `kod_rate` | A request with the exponent 4, a Kiss o' Death `RATE` reply, a back off of `poll_interval_ms`, and a request with 6 that the server answers normally |
| `symmetric_peer` | Two mode 1 packets `poll_interval_ms` apart from a stratum 3 peer, each answered in mode 2 |

The client of each scenario binds a field of the last reply with `ntp.field` and a `set` action, which shows that it parsed the reply, and the server side accepts any request of at least 48 bytes, so extension fields and a message authentication code do not stop it.

The module carries no mode 5 broadcast packets, no mode 6 or 7 control and private messages, no extension fields, no authentication, and no NTPv5 or NTS. A scenario that needs them needs a new function.

## Port 123

nDPI 4.2.0 classifies NTP only when one side of the flow uses port 123. The fixture capture therefore uses 123 on the server side, which the harness allows because the server has its own namespace and `capture.sh` runs as root. A flow on another port is `Unknown` to nDPI (`capture.sh` was tried with 5123 once, and nDPI reported 5 of 5 flows as `Unknown`). The parameter `server_port` exists for tests, and a run that realism must classify keeps 123. The tests run over in memory datagrams and never bind a port.

## Tests

```bash
cargo test -p fps-module-ntp
```

The unit tests cover the codec (round trips, field positions, the timestamp conversions and the era limit), every argument check, the replies of each stratum, the Kiss o' Death packet, and the stability of one server over 40 minutes of replies. `tests/scenarios.rs` runs every scenario between two peers in memory, parses every datagram, checks each timestamp against the pinned clocks, and checks that the paused Tokio clock advances by exactly the pauses of the scenario. `tests/transcript.rs` compares the bytes of every scenario at seed 42 with the files under `tests/testdata/`, and shows that the seed and the clock both change the bytes while the sizes stay equal. `UPDATE_GOLDEN=1` records an intended change.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness, runs `fixture_peer` on both sides, and records `fixtures/ntp.pcapng` with 6 sessions per scenario, 30 in all. It requires one session per run in the capture, which fails when two sessions share a 5-tuple (MOD-21), and then scores the capture with nDPI and tshark. nDPI 4.2.0 labels 30 of 30 sessions `NTP`, and tshark 4.2.2 reports no parser error and no malformed packet.

The client of `fixture_peer` binds one source port per session, counting up from 40000, so that no two sessions of a capture share a 5-tuple. The poll pauses shrink with `--poll-ms`, and `capture.sh` passes 1000, so the 64 and 128 second pauses become 1 and 2 seconds and a run takes about a minute. The poll exponents in the packets stay 6 and 7, and the burst keeps its real gap of 2 seconds. The capture therefore shows the poll fields and the burst of a real client, and not its poll spacing.

## Dispatching the module in the agent

The agent dispatches only the modules of its registry. A plan with an ntp scenario needs one line in the registry of `engine/fps-agent/src/modules.rs`, `stateless!(fps_module_ntp, Ntp),`, and the dependency `fps-module-ntp = { path = "../../modules/ntp/native" }` in `engine/fps-agent/Cargo.toml`.
