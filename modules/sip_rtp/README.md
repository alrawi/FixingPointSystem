# sip_rtp module

The `sip_rtp` module generates telephone calls: SIP signaling over UDP on port 5060 (RFC 3261) and the RTP media of the call (RFC 3550, RFC 3551). A call is a real dialog. The client sends an `INVITE` with an SDP offer (RFC 3264, RFC 8866), the server answers `100 Trying`, `180 Ringing`, and `200 OK` with an SDP answer that picks a codec, the client sends the `ACK` and opens the media flow, both sides send RTP packets at the packet time of the codec, and the client ends the call with a `BYE` that the server answers with `200 OK`. A refused call is the `INVITE`, `100 Trying`, `486 Busy Here`, and the `ACK` of the failure.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Five scenarios, four with a signaling flow and a spawned media flow, one with a signaling flow |
| `native/` | The crate `fps-module-sip-rtp` and the `fixture_peer` example |
| `tests/` | Catalog, scenario, and golden transcript tests |
| `fixtures/` | `capture.sh`, `expected.yaml`, the committed capture `sip_rtp.pcapng`, the tests of its packet gap labels, and the Zeek check |

## How a call runs

The `signaling` flow carries the SIP messages, one per datagram. The client builds the `INVITE` with `sip_rtp.invite`, and every later message answers the message before it (`last`), so the Call-ID, the tags, the branch, and the CSeq follow from the dialog and no variable carries them. `sip_rtp.sdp_port` and `sip_rtp.codec` read the media port and the chosen codec out of an SDP body, and a `set` stores them in the variables `rtp_port` and `codec`.

The `media` flow is spawned (`spawn`), reads those two variables from its parent, and runs on its own UDP 5-tuple. Both sides send in lock step on that one pair of ports: the client sends a packet, reads the packet of the server, and sleeps until the next slot, and the server reads, then sends. One 5-tuple that carries RTP in both directions is what nDPI needs to classify the session as RTP, and the scenario language has no concurrency inside one side, so the lock step is also the only way to send both directions from one flow.

The server spawns its media flow before the ring time, so the RTP port is bound long before the client learns it from the `200 OK`. When the server spawned it after the ring time, the first RTP packet could reach the port before the server bound it.

### Packet interval

A `delay` of the packet time after every packet drifts, because a sleep wakes a little late and the delays add up. In the harness a 20 ms packet time came out at 21.5 ms. `sip_rtp.pace` keeps a due time per flow side, moves it on by one packet time at every call, and returns the time left, so a late wake shortens the next sleep. The mean gap in the committed capture is within 0.1 ms of the packet time, and single gaps usually vary by a millisecond or two, with a rare outlier of several milliseconds, as the timer of a loaded host does.

The packet time is the `ptime_ms` variable, a multiple of 10 from 10 to 120. The RTP timestamp advances by `ptime_ms` times the clock rate of the codec, so the packet time is also in the packets. The `temporal.packets` labels of `fixtures/expected.yaml` list the packet times that the scenarios use (20 and 30 ms).

## Template functions

| Function | Purpose |
|---|---|
| `port` | An even RTP port from the range `rtp_port_min` to `rtp_port_max` |
| `invite` | The `INVITE` with the SDP offer of the listed codecs, `telephone-event`, and `a=ptime` |
| `response` | A response to a request (`100`, `180`, `200` with an SDP answer, `486`, and the `200 OK` of a `BYE`) |
| `ack`, `bye` | The `ACK` and the `BYE` of the dialog of a response. The `ACK` of a final failure reuses the branch of the `INVITE`, as RFC 3261 requires |
| `sdp_port`, `codec` | The media port of an SDP body, and the first codec of it that the accept list names |
| `rtp` | The next RTP packet of this flow side: version 2, the marker bit on the first packet, a sequence number that counts up, a timestamp that advances by one packet time, a fixed SSRC, and a payload of the size of the codec |
| `packets`, `pace`, `hangup_ms` | The number of packets in `talk_ms`, the sleep until the next packet is due, and the time from the start of the media to the `BYE` |

The manifest lists the parameters of every function.

## Codecs

| Codec | Payload type | Packet | Notes |
|---|---|---|---|
| `PCMU`, `PCMA` | 0, 8 | 8 bytes per ms | 160 bytes at 20 ms |
| `G722` | 9 | 8 bytes per ms | The timestamp counts at 8000 Hz, as RFC 3551 says |
| `G729` | 18 | 1 byte per ms | 20 bytes at 20 ms |
| `opus` | 111, dynamic | 40 to 110 bytes at 20 ms | A variable bit rate. The first byte is a valid table of contents byte (SILK wideband, one frame), because tshark rejects random Opus frames as parser errors. The clock is 48000 Hz |

The payload of the other codecs is random bytes, as the output of a speech codec looks.

## Scenarios

| Scenario | Shape |
|---|---|
| `call_pcmu` | PCMU at 20 ms, 10 s of talk time, a ring time of 2 s |
| `call_pcma` | PCMA at 30 ms, 9 s of talk time, offered as `PCMA,PCMU` |
| `call_g729` | G.729 at 20 ms, 12 s of talk time |
| `call_opus` | Opus at 20 ms, 15 s of talk time, offered as `opus,PCMA` |
| `busy` | `INVITE`, `100 Trying`, `486 Busy Here`, `ACK`, and no media |

`codecs` is the offer of the client and `accept` is the list of the server. A server answers with the first offered codec that it accepts. When there is none, the call fails with an `extension_error`, as a `488` would end it. `talk_ms`, `ring_ms`, and `ptime_ms` set the length of a call.

## Limits

* Only the dialog of one call exists. There is no registration, no authentication challenge, no forking, no re-INVITE, no hold, and no `CANCEL`.
* A call sends no RTCP, no DTMF events, and no silence suppression. Both directions run for the whole talk time.
* The payload of the media is synthetic bytes, never audio.
* The signaling flow of the server serves one call, so calls run one after another. A capture with concurrent calls needs the agent to run many instances.
* **The agent cannot run the call scenarios today.** A scenario with several flows of which one is UDP is refused (`check_flows` in `engine/fps-agent/src/assemble.rs`), because the instance runner of the runtime opens TCP connections only. The four call scenarios have a UDP media flow beside the signaling flow, so they need a datagram opener in the runtime, which is a follow up of AGT-16. `busy` has one flow and can run as soon as the registry dispatches `sip_rtp`. The fixture peer runs every scenario today.

## Tests

```bash
cargo test -p fps-module-sip-rtp
python3 -m unittest discover -s modules/sip_rtp/fixtures
```

The unit tests cover the SIP parser and writer, the SDP offer and answer, the codec table, and the RTP header. `tests/scenarios.rs` runs every scenario between two peers over an in memory UDP network on a paused clock, so the packet gaps are exact. It checks the dialog structure, the SDP negotiation, the RTP interval, that the media ends before the `BYE`, the refused call, distinct Call-IDs and ports per call, that a call without a common codec fails, and that a lost RTP packet fails the call with a timeout. `tests/transcript.rs` compares the bytes of every flow at seed 42 with the files under `tests/testdata/`. `UPDATE_GOLDEN=1` records an intended change. The Python tests cover the packet gap labels with the committed capture and the Zeek check with synthetic logs.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness, runs `fixture_peer` on both sides over kernel UDP sockets, and records `fixtures/sip_rtp.pcapng` with 3 calls per scenario, 15 calls and 27 sessions in all (15 SIP, 12 RTP). The calls are shortened to 2 s of talk time and 300 ms of ringing. The script:

1. requires one session in the capture for every session that the peers ran, which fails when two sessions share a 5-tuple (MOD-21),
2. scores the capture with nDPI and tshark (`expected.yaml`): every SIP session must be classified as SIP, every RTP session as RTP, and tshark must report no parser error, and every RTP stream must keep its packet interval (below), and
3. when Zeek is installed, runs `zeek_check.py`, which requires one `INVITE` row in `sip.log` for every call, with its own Call-ID and a row that carries a final status code, and no `sip` entry in `weird.log`.

The same score covers the packet interval (MOD-24, which replaced `rtp_check.py`). The `temporal.packets` section of `expected.yaml` requires of every RTP stream, in each direction, that 90% of its gaps lie within 8 ms of 20 ms or of 30 ms, whichever is nearest to the median gap of the stream. The committed capture has gaps from 0.09 ms to 63.7 ms in a stream of 20 ms packets and passes. `fixtures/test_packet_labels.py` reads the capture with the temporal check of the runner, needs no tool, and shows that the capture passes and that it fails when the packets of one call are shifted by up to 14 ms. The check is looser than the script was: it does not read the packet time from the RTP timestamps, so a scenario with another packet time needs an entry in `ms`, and it does not test the mean gap, so the exact pacing stays with the paused clock tests of `tests/scenarios.rs`.

The client of `fixture_peer` takes its source port from a counter, one port per flow, so that no two sessions of a capture share a 5-tuple.

## Dispatching the module in the agent

The agent dispatches `http1`, `dns`, `content`, `smtp`, `imap`, `pop3`, `ftp`, `http2`, and `ssh` today. A plan with a `sip_rtp` scenario needs a registry entry for `sip_rtp` that builds one `SipRtp` per flow side, as `fixture_peer` does, because the RTP stream and the pacing state live in the value.
