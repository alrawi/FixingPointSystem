# hls module

The `hls` module generates HTTP Live Streaming traffic (RFC 8216) over HTTP/1.1. A player fetches a master playlist, a media playlist, and then the segments of a bitrate ladder at the segment duration, and the server answers with MPEG-TS segments whose size follows the bit rate of the rung. Its native crate `fps-module-hls` implements eleven template functions in Rust under decision D10. The HTTP framing comes from `http1.request` and `http1.response`, so the module requires `http1` and duplicates none of its code. Every body is synthetic, every host name lies under the reserved `.test` domain, and a segment carries random bytes behind valid transport stream framing, so it holds no third party media.

## Layout

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Three flow programs, listed below |
| `native/` | The crate `fps-module-hls` with the `TemplateFn` implementation `Hls` (a counter and a pace due time per flow side) |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets, with a source port of its own per connection (MOD-21) |
| `native/examples/print_catalog.rs` | Prints the `functions` block of the manifest |
| `tests/` | Catalog, scenario, and golden transcript tests |
| `fixtures/` | The capture `hls.pcapng`, its labels `expected.yaml`, `params.json`, `capture.sh`, and `hls_check.py` with its tests |

## Scenarios

| Scenario | Port | Exchange |
|---|---|---|
| `vod_ladder` | 8090 | One keepalive connection. The master playlist, then for each stage the media playlist of its rung and `stage_segments` segments, one per `segment_ms`. The player walks up the ladder, one rung per stage. |
| `vod_segments` | 8091 and 8092 | The playlist connection on 8091 spawns one connection per segment on 8092 (`Connection: close`), one `segment_ms` apart. This is the cadence that the temporal check of the realism runner sees. |
| `live_refresh` | 8093 | A live playlist with a sliding window. The player reloads the playlist and fetches the newest segment once per cycle. |

The player is data driven. It reads the playlist URIs that it received (`hls.playlist_uri`, `hls.resolve`), and the server reads the rung and the segment number out of the request line. A flow has no condition and no arithmetic, so the stages are explicit: `stages` times `stage_segments` segments, and `segments` must equal that product.

## Segments, sizes, and pacing

The size of a segment is `bit_rate * segment_ms / 8000` octets, rounded to whole 188 octet packets and at least three. The segment opens with a PAT and a PMT, followed by video access units of ten packets with a PCR in the first packet, one audio frame after each, and null packets up to the size. The video and the audio take multiples of 16 packets, so every continuity counter is back at its start value at the end of a segment. The counters of the PAT and the PMT start at the segment number modulo 16. A parser that reads consecutive segments, also of different rungs, therefore sees no continuity gap (tshark 4.2.2 reports none for the fixture).

`hls.pace(segment_ms)` is due at once on the first call and then one segment duration apart on an absolute schedule, as `sip_rtp.pace` does. A `delay` of `segment_ms` after every request drifts, because each sleep wakes late.

## Acceptance

The acceptance asks that the temporal check accepts the segment fetch cadence and that the byte share matches the configured bit rate within 5%.

* The temporal check of the realism runner compares the gaps between the starts of sessions. `expected.yaml` gives the application `hls_segment` (port 8092) the constant arrival `400 ms ± 40 ms`. The capture holds one playback of 33 segments, so 32 gaps lie between 398.2 and 401.8 ms, and the check passes with `min_samples: 30`.
* The runner cannot see the gaps between the requests inside one keepalive connection, and it reads no bytes. `fixtures/hls_check.py` reads both from the capture. For each playback it requires a mean gap within 5% of the segment duration and 90% of the gaps within 10%, a body within 188 octets of its configured size, the byte share of each rung within 5% of the share that the bit rates give, and the bit rate of the playback within 5% of the configured one. The vod playbacks must walk up the whole ladder. The result is in `fixtures/hls.playbacks.json`: the mean gaps are 400.1 to 400.3 ms, and the bit rate is 375327 against 373333 configured for `vod_ladder` (0.5%).

## Fixture

`fixtures/capture.sh` records the capture in the QA-01 harness. The parameters (ladder `160000@426x240,320000@640x360,640000@960x540`, segment duration 400 ms, the number of sessions and segments) are in `params.json`, which the script, the peer, and `hls_check.py` all read. The capture holds 40 TCP sessions, each with its own source port.

| Check | Result |
|---|---|
| nDPI 4.2.0 | 40 of 40 sessions as HTTP |
| tshark 4.2.2 parser errors | none |
| temporal check (n = 32 gaps) | passes |
| `hls_check.py` | passes on all 7 playbacks |

Zeek did not run locally. The CI run with nDPI 4.14 gives the second nDPI result.

## Tests

```bash
make lint-mod-hls
make test-mod-hls
sudo make e2e-mod-hls
```

`cargo test -p fps-module-hls` runs 31 unit tests, the catalog test, the scenario tests, and the golden transcripts (`UPDATE_GOLDEN=1` rewrites them). `python3 -m unittest discover -s modules/hls/fixtures` tests the check with synthetic captures.

## Limits

The agent does not dispatch `hls` calls yet. The module has state, so `engine/fps-agent/src/modules.rs` needs a registration with the `Hls` value, as for `http2`, and `vod_segments` is a multi flow scenario that the agent runs only after AGT-16 covers it. The tests and `fixture_peer` run it through the interpreter and the kernel backend.
