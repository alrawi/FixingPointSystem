# mqtt module

The `mqtt` module generates MQTT 3.1.1 and MQTT 5.0 connections between a client and a broker on port 1883. A client connects with `CONNECT`, publishes at QoS 0, 1, or 2, subscribes and receives, or sits idle on a keep alive. Every packet follows the standard (OASIS MQTT 3.1.1 and MQTT 5.0): the fixed header, the variable byte integer of the Remaining Length, and in version 5 the properties. A receiving side checks the packet it reads against the same rules, so a session that completes is a session whose packets both sides accepted.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Five scenarios with one flow each |
| `native/` | The crate `fps-module-mqtt`, the `fixture_peer` example, and the `print_catalog` example |
| `tests/` | Catalog, scenario, and golden transcript tests |
| `fixtures/` | `capture.sh`, `expected.yaml`, and the committed capture `mqtt.pcapng` |

## The library

The crate has no MQTT dependency. The Rust MQTT libraries (`rumqttc`, `paho-mqtt`) run a client or a broker with a network loop of their own, and a scenario needs the opposite, a writer and a reader that handle one packet when the interpreter asks for it. The encoders (`native/src/wire.rs`) and the packet reader (`native/src/read.rs`) are therefore written here. The unit tests cover the variable byte integer at the limits of the standard (0, 127, 128, 16383, 16384, 2097151, 2097152, and 268435455, and a form that is not minimal is an error), every packet type in both versions, the properties of version 5, and the rejection of truncated and malformed packets.

## How a session runs

One value of `Mqtt` serves one session and one side. It holds the version, the next packet identifier, the packet reader, the last packet that it read, and the keep alive schedule. The interpreter builds one per flow side, and a `CONNECT` sets the version of the session on the side that reads it. A client that sends `connect(version=5)` writes version 5 packets from then on.

A packet has a variable length, and the scenario language has no loop that runs until a condition holds, so a side reads a packet with a fixed pattern of reads. It reads one byte, the fixed header, with `expect: {bytes: 1}`, and then it runs `expect: {bytes: "{{ mqtt.feed(last) }}"}` six times in a `loop`. `feed` hands the bytes of the last read to the packet reader and returns the size of the next read. After the first byte the next read is one byte, and the Remaining Length takes between one and four of them, so the reader asks for one byte while the length continues and for the whole body once the length is complete. A read of size 0 is a no-op, which lets the loop run its six rounds for every packet. `set: {x: "{{ mqtt.recv('publish:1') }}"}` then checks the packet, stores its fields, and returns its kind. The argument names the kind that the flow expects (`any`, `connack`, `publish`, `publish:1` for a publish at QoS 1, and so on), and a packet of another kind fails the session. `mqtt.field('topic')` and its relatives read the stored packet. A packet body larger than 16 MiB less 8 bytes is an error, which keeps it below the limit of the interpreter for `last`.

`mqtt.pace(keepalive)` returns the milliseconds until the next slot of the keep alive schedule. Each call moves the due time on by `keepalive` seconds, so a late wake shortens the next sleep and the pings keep their spacing for the whole session. This is the absolute time pacing of `sip_rtp.pace` (MOD-14). A `delay` of the interval after every ping would drift by the lateness of each wake.

## Template functions

The manifest lists the parameters of every function, and a catalog test compares the manifest with the crate.

| Function | Purpose |
|---|---|
| `connect` | `CONNECT` with a client identifier, keep alive, user name and password, a will, and in version 5 the session expiry, the receive maximum, and the topic alias maximum |
| `connack` | `CONNACK` with the return code of version 3.1.1 or the reason code and the properties of version 5, with the defaults of Mosquitto 2 |
| `publish` | `PUBLISH` at QoS 0, 1, or 2 with the retain and dup flags. A packet identifier of -1 takes the next one of the side. Five parameters set properties of version 5 (message expiry, content type, response topic, correlation data, topic alias) |
| `puback`, `pubrec`, `pubrel`, `pubcomp` | The answers of the QoS 1 and QoS 2 flows. A packet identifier of -1 answers the packet that the side read last |
| `subscribe`, `suback`, `unsubscribe`, `unsuback` | Subscriptions for comma separated filters, with the options of version 5 (no local, retain as published, retain handling) |
| `pingreq`, `pingresp` | The keep alive pair |
| `disconnect` | `DISCONNECT`, with a reason code in version 5 |
| `varint`, `varint_value` | The variable byte integer, written and read |
| `client_id`, `reading` | A client identifier in the style of MQTT.js, and a JSON sensor reading with a sequence number and random values |
| `feed`, `recv`, `field`, `granted` | The packet reader, see above. `granted` gives the codes for a SUBACK, capped at the maximum QoS of the broker |
| `pace` | The keep alive schedule |

`field` accepts these names: `type`, `version`, `flags`, `packet_id`, `dup`, `qos`, `retain`, `topic`, `payload`, `payload_length`, `client_id`, `keepalive`, `clean`, `user`, `password`, `will_topic`, `will_payload`, `session_present`, `code`, `codes`, `filters`, `requested_qos`, and `filter_count`, and also the snake case name of any property of version 5, such as `session_expiry_interval` or `content_type`. A name that the packet does not carry is an error.

## Scenarios

| Scenario | Shape |
|---|---|
| `publish_qos0` | `CONNECT`, `CONNACK`, `messages` JSON readings at QoS 0 with a random gap, `DISCONNECT` |
| `publish_qos1` | The same at QoS 1 with a retained status message first. The broker answers each `PUBLISH` with a `PUBACK` before the next |
| `publish_qos2` | The exactly once flow, `PUBLISH`, `PUBREC`, `PUBREL`, `PUBCOMP`, for each message |
| `subscribe` | A `SUBSCRIBE` for several filters, a `SUBACK`, and `PUBLISH` packets from the broker at the granted QoS, with the matching acknowledgments from the client |
| `keepalive` | An idle connection with `pings` pings at the keep alive interval, then one message and `DISCONNECT` |

`version` is 4 for MQTT 3.1.1 and 5 for MQTT 5.0. Every scenario takes both, and the capture holds half of its sessions in each.

## Limits

* The broker is a script and not a real broker. It grants the QoS that the scenario names, does not route a message to another client, and sends the messages of the `subscribe` scenario on its own.
* The module has no TLS. MQTT over TLS on port 8883 would reuse the `tls` module, and the catalog does not ask for it.
* A packet body larger than 16 MiB less 8 bytes is an error. A `CONNECT` does not carry enhanced authentication.
* A QoS 2 session does not retransmit, and the `dup` flag is set only when a scenario passes it.

## Tests

```bash
cargo test -p fps-module-mqtt
```

The unit tests run the encoders and the reader. `tests/scenarios.rs` runs every scenario in both versions between two peers in memory with the interpreter, checks the order and the identifiers of the packets, and flips one bit to prove that a damaged packet fails the session. It also runs the `keepalive` scenario under paused time with 200 pings and requires the spacing of exactly one interval. `tests/transcript.rs` compares the bytes of every scenario at seed 42 with the files under `tests/testdata/`, and `UPDATE_GOLDEN=1` records an intended change.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness, runs `fixture_peer` on both sides, and records `fixtures/mqtt.pcapng` with 4 sessions per scenario and version, 40 sessions in all. It first requires one session per run in the capture, then scores it with nDPI and tshark. nDPI must classify at least 99% of the sessions as MQTT, and tshark must report no parser error. nDPI 4.2.0 classified 40 of 40 locally, and tshark 4.2.2 dissected the packets of both versions. The `realism` workflow measures with nDPI 4.14 in CI, which this sandbox could not run.

The client of `fixture_peer` takes its source port from a counter (`--first-port`, 40000 by default), one port per session, so that no two sessions of a capture share a 5-tuple (MOD-21). The pings of the `keepalive` scenario in the capture are 2 seconds apart, as the `keepalive` variable says, and the spacing is exact to the millisecond.

## Dispatching the module in the agent

The agent does not dispatch `mqtt` calls yet. `Mqtt` is `Clone` and a clone of a fresh value is a fresh session, so the registry entry builds `fps_module_mqtt::Mqtt::default()` and the agent clones it for every session and side. The registry line is in the log entry of MOD-18.
