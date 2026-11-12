# http2 module

The `http2` module generates HTTP/2 traffic between a client and a server side, with prior knowledge on a cleartext port (h2c) and over TLS 1.3 with ALPN `h2`. Its native crate `fps-module-http2` implements twenty template functions in Rust under decision D10 (MOD-05), and the scenarios of this package call them. Every body comes from the seeded `fill()` generator, and every host name lies under the reserved `.test` domain.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Four flow programs, two in the clear and two over TLS, listed below |
| `native/` | The crate `fps-module-http2` with the `TemplateFn` implementation `Http2` (one HPACK encoder and decoder per session side) and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture, with TLS profiles for the https scenarios |
| `tests/` | Scenario, https, interop, transcript, and fixture tests, with golden transcripts in `tests/testdata/` |
| `tests/common/` | The in memory runner, the TLS profile registry that tests and `fixture_peer` share, and an independent frame reader (`frames.rs`) |
| `fixtures/` | The synthetic capture `http2.pcapng` with its embedded TLS keys, its labels `expected.yaml`, `capture.sh`, which records it and writes the result of the decrypted check as `http2.decrypted.json` |

## Scenarios

Each scenario has one flow whose client and server sides run against each other. A side reads a frame with two `expect` actions, the 9 octet header and then `http2.frame_length(last)` octets, because a flow cannot parse binary data with `set`. The server waits for the 24 octet connection preface before it sends its SETTINGS, as real servers do, so the client is the first sender on the connection, which payload classifiers rely on.

| Scenario | Port | Transport | Exchange |
|---|---|---|---|
| `h2c_browse` | 8082 | cleartext, prior knowledge | curl-like client: page on stream 1, then six requests in flight on the odd streams 3 to 13 with interleaved DATA frames |
| `h2c_upload` | 8081 | cleartext, prior knowledge | nghttp2-like client: two concurrent POST uploads on streams 1 and 3 that fill the stream window and wait for the WINDOW_UPDATE frames of the server |
| `https_browse` | 8443 | TLS 1.3, ALPN `h2` | the exchange of `h2c_browse` with the SETTINGS of Chrome 136 (client profile `chrome-136`) |
| `https_upload` | 8444 | TLS 1.3, ALPN `h2` | the exchange of `h2c_upload` with the SETTINGS of Firefox 139 (client profile `firefox-139`) |

What the scenarios show, and what the tests check on the frames:

- **Concurrent streams.** Six requests are in flight on one connection, and the server writes one DATA frame per stream in every round, so the DATA frames interleave.
- **Realistic SETTINGS.** Clients announce `SETTINGS_ENABLE_PUSH` 0, `MAX_CONCURRENT_STREAMS`, and an initial window and a connection WINDOW_UPDATE in the shape of curl (33554432), Chrome (6291456, 15663105), and Firefox (131072, 12517377). The server follows nginx. Every SETTINGS frame is acknowledged once.
- **Server push disabled.** Every client announces `SETTINGS_ENABLE_PUSH` 0, and the tests check that no PUSH_PROMISE frame appears.
- **Flow control.** The upload scenarios send exactly one stream window of DATA per round and then wait for the WINDOW_UPDATE frames, and the browse scenarios top up the connection window.
- **PING.** The client of the browse scenarios and the server of the upload scenarios send a PING that the peer acknowledges with the same 8 octets.
- **Graceful GOAWAY.** The server sends a GOAWAY with last stream 2147483647 and then a GOAWAY with the last stream that it processed, both with NO_ERROR.

The https scenarios use `tls_upgrade` right after `connect` and `accept`. The client names a browser profile of the TLS module with the SNI `www.example.test`, and the server names the profile `server-tls13-h2`, which accepts TLS 1.3 only and selects `h2`. The existing `server-tls13` profile selects `http/1.1`, so this package adds the h2 profile to the registry in `tests/common/tls.rs`. The agent has no such profile yet (see the Modules wiki page and the log entry of MOD-06).

The parameter schema in the manifest names every variable that the scenarios declare, with its default and its bounds. Scenarios have no arithmetic and cannot branch on what they read, so stream ids are literals and the counts of frames come from variables and from `http2.data_frames`.

**TLS limit.** The TLS session of the TLS module fails with "received plaintext buffer full" when one read of ciphertext completes more than 16 KiB of plaintext while earlier plaintext is unread (`modules/tls/native/src/session.rs`, `poll_fill`). The https scenarios therefore run with small bodies in the tests and in the fixture (`tests/common/mod.rs` `tls_small()`, `--var-tls` in `capture.sh`). The defaults of the https scenarios equal the cleartext ones, so a plan that runs them with large bodies can hit the limit until the TLS module backs off instead of failing.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-http2     # validate.py on the manifest and every scenario
make test-mod-http2     # cargo test -p fps-module-http2, then the realism score of the fixture
make fuzz-mod-http2     # the 60 s fuzz test of the frame and HPACK parsers
sudo make e2e-mod-http2 # record and score a fresh capture in the namespace harness
```

- `tests/scenarios.rs` runs every scenario with both sides over the in memory duplex and parses both directions with its own frame reader (`tests/common/frames.rs`, with `hpack::Decoder`). It checks SETTINGS first, one ACK per side, `SETTINGS_ENABLE_PUSH` 0, no PUSH_PROMISE, odd rising stream ids, pseudo header order, one response per request, DATA interleaving, flow control credit, matching PINGs, and the two GOAWAY frames.
- `tests/interop.rs` replays both directions of every scenario into the `h2` crate, once as a server (client bytes) and once as a client (server bytes). All four scenarios pass in both directions.
- `tests/transcript.rs` renders every scenario with seed 42 and session id 1 and compares both directions with `tests/testdata/<scenario>.seed42.{c2s,s2c}.bin`. `UPDATE_GOLDEN=1` rewrites them.
- `tests/https.rs` checks the profile and port binding, the TLS 1.3 handshake with ALPN `h2`, the encrypted wire, the key log, and the JA4 of every ClientHello.
- `tests/fixture.rs` runs `python3 -m realism decrypted` on the committed capture, with the labels `realism/examples/decrypted/http2.labels.yaml`. It skips itself when tshark or an ndpiReader that knows HTTP/2 is missing, and `FPS_REQUIRE_CAPTURE_TOOLS=1` turns that into a failure.
- The catalog test of `fps-module-http2` compares `FUNCTIONS` with the `functions` list of the manifest.
- The 60 s fuzz test is `cargo test -p fps-module-http2 --release --test fuzz -- --ignored fuzz_60s`.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer` and the kernel TCP backend, records the router's client arm, injects the TLS keys with `editcap --inject-secrets`, and scores the capture. It runs 20 sessions per scenario, so the capture holds 80 sessions (40 cleartext, 40 TLS) in 2.7 MB. Treat the capture like a key log: its keys are those of a throwaway authority and synthetic traffic.

| Check | Result |
|---|---|
| tshark 4.2.2 parser errors | none, and no malformed packets, on all 80 sessions |
| tshark HTTP/2 inside TLS | `python3 -m realism decrypted`: 40 of 40 TLS sessions decrypt and tshark reports no parser error on the decrypted view |
| nDPI 4.2.0 (`libndpi-bin`, `ndpiReader -d`) | cleartext: 0 of 40 classified, all Unknown, because this nDPI has no HTTP/2 protocol. TLS sessions: 40 of 40 as TLS |
| nDPI 4.14.0 (`ndpiReader -d`, built in the workflow, GitHub Actions) | cleartext: 40 of 40 as HTTP2, TLS sessions: 40 of 40 as TLS, decrypted view of the TLS sessions: 40 of 40 as HTTP2 (ratio 1.0) |
| nDPI of nfstream 6.6.0 (local cross check) | the same counts as nDPI 4.14.0 |

**nDPI and HTTP/2.** The acceptance asks for nDPI to classify at least 99% of the sessions as HTTP/2, cleartext directly and TLS through the decrypted view. The nDPI 4.2.0 of Ubuntu 24.04 cannot do that for cleartext, because it has no HTTP/2 detector, and `-d` switches port guessing off. nDPI 4.14.0, which `realism/ci/build-ndpi.sh` builds from a pinned commit, can: its `ndpiReader` classifies 40 of 40 cleartext sessions as `HTTP2` (the label of `expected.yaml`), 40 of 40 TLS sessions as `TLS`, and 40 of 40 decrypted sessions as `HTTP2`, and tshark and the realism runner report 0 parser errors (the CI run of the pull request has the report). Releases after 4.2 write the `-C` flow file with `|` and `ndpiReader -h` ends with 1, and the realism runner reads both, so `FPS_NDPI_READER` names the new `ndpiReader` directly. nDPI needs the client to be the first sender, because with the server SETTINGS ahead of the client preface only 26 of 40 cleartext flows were classified, so the server side of every scenario waits for the 24 octet preface. The cleartext ports 8082 and 8081 are not among the ports where Zeek starts its HTTP analyzer: on port 8080 Zeek reported one parser error per h2c session. The shared `realism` workflow builds the same nDPI 4.14 through `realism/ci/install-tools.sh` since RLM-12.

**TLS.** nDPI cannot see inside TLS. On the capture it reports the https sessions as `TLS`, so `expected.yaml` accepts `TLS` for them, and `python3 -m realism decrypted` measures HTTP/2 on a decrypted view, as MOD-04 did for http1. It decrypts the capture with tshark and the embedded keys, writes the plaintext of each TLS session as a plain TCP session with the same addresses and ports (synthetic frames with correct checksums), and scores that view with the realism runner. It fails when a TLS session does not decrypt, when the capture does not hold the 40 TLS sessions that `capture.sh` expects, or when the nDPI ratio is below 0.99. The command does not count the HTTP/2 requests and responses inside the sessions, which the former check script of the module did. The view is built from tshark's output and is not a capture of real plaintext traffic, so it shows that the decrypted bytes are HTTP/2 that nDPI recognizes and nothing about the TLS record layer.

**Counting the TLS sessions (MOD-23).** Once the module job of the realism workflow failed with "the capture has 39 TLS sessions, expected 40" on a pull request that did not touch http2, and passed on rerun. The same run scored the same capture with 40 of 40 sessions in the conformance, encryption, and JA4 checks, so the capture was whole and the generator had opened 40 connections. `python3 -m realism decrypted` lists the TLS sessions as the streams with a ClientHello that a plain `tshark -Y "tls.handshake.type == 1"` finds, and on the ports 8443 and 8444 tshark finds one by its heuristic, which takes a stream for TLS only when the first TCP segment of the client holds at least the 5 octets of a record header. We cut the first ClientHello of the committed capture after 1 to 4 octets and the check reports exactly 39 TLS sessions, expected 40, while 5 octets or more and every reordering, duplication, or loss of the handshake frames that we tried leave the count at 40. The encryption check of `realism score` passes `-d tcp.port==P,tls` and finds all 40. `capture.sh` therefore ran the decrypted check through a wrapper script that added that option for the https ports when tshark read the capture and for no other file. RLM-18 moved the fix into the runner, which decodes the server ports of the session rules of the labels as TLS (`realism/tests/test_decrypted.py` cuts a ClientHello after 1 and after 4 octets and counts all sessions), so MOD-24 removed the wrapper and its tests, and the committed capture lists its 40 TLS sessions without it. When the check still fails, `capture.sh` prints the SYNs, the distinct 5-tuples, and the streams with a ClientHello on the https ports, which tells a reused source port (kernel picked the same port for two connections, the realism runner joins them into one session, and tshark keeps two streams) from a connection without ClientHello. We could not read the capture of the failed run (its artifact is not reachable from the sandbox), so what split the ClientHello there is not known; the Decode As of the runner makes the count independent of it.

The Zeek check of the shared realism job needs the ports above, and we did not run Zeek locally. Chrome 136 and Firefox 139 have the `expected_ja4` values of `modules/tls/native/src/browser.rs`, which `tests/https.rs` checks against real ClientHello bytes.
