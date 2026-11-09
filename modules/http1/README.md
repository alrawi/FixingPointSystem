# http1 module

The `http1` module generates HTTP/1.0 and HTTP/1.1 traffic between a client and a server side. Its native crate `fps-module-http1` implements the template functions `http1.request` and `http1.response` in Rust under decision D10, so scenarios render every message without a WebAssembly runtime. Every body comes from the seeded `fill()` generator, and every host name lies under the reserved `.test` domain.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Ten flow programs, six in the clear and four over TLS, listed below |
| `native/` | The crate `fps-module-http1` with the `TemplateFn` implementation `Http1` and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture, with TLS profiles for the https scenarios |
| `tests/` | Catalog, interpreter, https, fixture, and golden transcript tests, with transcripts in `tests/testdata/` |
| `tests/common/tls.rs` | The TLS profile registry of the https scenarios, which the tests and `fixture_peer` share |
| `fixtures/` | The synthetic capture `http1.pcapng` with its embedded TLS keys, its labels `expected.yaml`, its realism report `http1.report.json`, the report of the decrypted check `http1.decrypted.json`, `capture.sh`, which records it, and `decrypted_check.py` |

## Template functions

Both functions return the complete message, which is the start line, the header block, the empty line, and the body. A body is length delimited with `Content-Length` unless `chunked` is true, in which case it travels in chunks of at most `chunk_size` bytes with `Transfer-Encoding: chunked`. Chunked framing needs version `1.1`. Every text argument is checked before any byte is written, and a value that would break the framing, such as a header value with a line break, fails the session with the reason `extension_invalid_argument`.

| Function | Parameters with defaults | Headers in order |
|---|---|---|
| `request` | `method`, `path`, `host`, `accept='*/*'`, `version='1.1'`, `body=''`, `content_type=''`, `chunked=false`, `chunk_size=8192`, `connection=''`, `user_agent='Mozilla/5.0 (X11; Linux x86_64) FixingPointSystem/0.1'` | `Host`, `User-Agent`, `Accept`, `Content-Type`, `Content-Length` or `Transfer-Encoding`, `Connection` |
| `response` | `status`, `body=''`, `content_type='text/html'`, `version='1.1'`, `chunked=false`, `chunk_size=8192`, `connection=''`, `server='fps/0.1'` | `Server`, `Content-Type`, `Content-Length` or `Transfer-Encoding`, `Connection` |

An empty text value leaves its header out. A request carries a framing header when it has a body, when it is chunked, or when its method is `POST`, `PUT`, or `PATCH`. An HTTP/1.0 request may leave `host` empty and then omits `Host`. A response with status 1xx, 204, or 304 carries no framing header and must have an empty body.

## Scenarios

Each scenario has one flow whose client and server sides run against each other. The receiving side finds the end of a header block with the pattern `\r\n\r\n`, reads a length delimited body with `expect bytes`, and reads a chunked body up to the last chunk with the pattern `0\r\n\r\n`. The pattern cannot occur earlier, because `fill()` never produces a carriage return and every chunk size line is followed by data.

| Scenario | Version | Exchange |
|---|---|---|
| `single_fetch` | 1.1 | One GET with `Connection: close` and a length delimited body |
| `keepalive_browse` | 1.1 | A chunked HTML page, a lognormal think time, and `objects` image fetches with random paths on one keepalive connection |
| `upload` | 1.1 | `uploads` POST requests with length delimited bodies, each answered with 201 Created |
| `upload_chunked` | 1.1 | One PUT with a chunked body, answered with 201 Created and `Connection: close` |
| `http10_fetch` | 1.0 | One GET without keepalive and a length delimited body |
| `http10_keepalive` | 1.0 | Several GET requests with `Connection: keep-alive` on one connection |

### HTTPS scenarios

The four https scenarios run the exchanges of `single_fetch` and `keepalive_browse` over TLS. Each side upgrades its connection with `tls_upgrade` as its first action after `connect` or `accept`. The client names a browser profile of the TLS module and the SNI `www.example.test`, and the server names a profile that accepts one TLS version. The browsers offer `h2` and `http/1.1` and the servers select `http/1.1`. The certificate comes from the run authority of the TLS module. Each scenario has its own port, so the realism labels tell the profiles apart.

| Scenario | Port | Client profile | Server profile | TLS version | Request headers of |
|---|---|---|---|---|---|
| `https_fetch` | 8443 | `chrome-136` | `server-tls13` | 1.3 | Chrome 136, one GET with `Connection: close` |
| `https_keepalive_browse` | 8444 | `firefox-139` | `server-tls13` | 1.3 | Firefox 139, a chunked page and `objects` image fetches |
| `https_fetch_tls12` | 8445 | `safari-18.5` | `server-tls12` | 1.2 | Safari 18.5, one GET with `Connection: close` |
| `https_keepalive_browse_tls12` | 8446 | `edge-134` | `server-tls12` | 1.2 | Edge 134, a chunked page and `objects` image fetches |

The flow level keyword `transport: {module: tls}` of the scenario schema is not used, because `Scenario::load` rejects it. The profile names are part of the interface with the agent: a plan with these scenarios needs the four browser profiles on the client side and `server-tls13` and `server-tls12` on the server side, and it needs a connector per client session (see the Modules wiki page). The agent does not build TLS profiles yet (AGT-14), so the scenarios run in the module harness. `tests/common/tls.rs` builds the registry there.

The parameter schema in the manifest names every variable that the scenarios declare, with its default and its bounds.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-http1     # validate.py on the manifest and every scenario
make test-mod-http1     # cargo test -p fps-module-http1, then the realism score of the fixture
sudo make e2e-mod-http1 # record and score a fresh capture in the namespace harness
```

The https tests in `tests/https.rs` check the TLS layer on the wire bytes and the handshake reports, including the JA4 of every ClientHello against the `expected_ja4` of its profile, which needs only `python3`. In the https scenarios the interpreter tests and the transcript test see the plaintext above TLS. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default. The scenario tests run every scenario with both sides over the in memory duplex for five session ids and parse both directions with the framing rules of RFC 9112, so they check the message count, the versions, the status codes, the body sizes, and the framing of every message. The transcript test renders every scenario with seed 42 and session id 1 and compares both directions with `tests/testdata/<scenario>.seed42.c2s.bin` and `.s2c.bin`. It shrinks the body sizes to keep the golden files reviewable, and `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer` and the kernel TCP backend, records the router's client arm, and scores the capture with the realism runner. It runs 20 sessions per scenario with small bodies, so the capture holds 200 sessions in about 1.3 MB.

The https sessions need a shared certificate authority and keys that decrypt them. The server peer generates the authority, issues the certificate for `www.example.test`, and writes the authority certificate to a file that the client peer reads. The client appends the secrets of every handshake to a key log, and the script injects the key log into the capture with `editcap --inject-secrets tls,<file>`. The capture therefore carries its keys as a decryption secrets block, and the realism runner, tshark, and Wireshark decrypt it without a sidecar. Treat such a capture like a key log: its keys are those of a throwaway authority and synthetic traffic, so it holds no secret. `FPS_FIXTURE_KEYLOG=FILE` keeps the key log next to it. The server binds the ports of every scenario before the first session, because the client starts the next scenario as soon as one ends.

The committed capture scores as follows.

| Check | Result |
|---|---|
| nDPI 4.2.0 conformance | 120 of 120 plain sessions are HTTP (ratio 1.0), 80 of 80 https sessions are TLS (ratio 1.0) |
| tshark 4.2.2 parser errors | none, and no malformed packets |
| Encryption (RLM-04) | 80 of 80 sessions decrypt, 80 of 80 ClientHellos have the JA4 of their profile |
| `decrypted_check.py` | 80 of 80 TLS sessions decrypt, tshark parses 200 requests and 200 responses inside them, and nDPI classifies 80 of 80 decrypted sessions as HTTP |

**nDPI and TLS.** nDPI cannot see inside TLS. On the capture it reports the https sessions as `TLS`, so `expected.yaml` accepts `TLS` for them. The acceptance of MOD-04 asks for nDPI to classify at least 99% of the decrypted sessions as HTTP, and `decrypted_check.py` measures that on a decrypted view. It decrypts the capture with tshark and the embedded keys, writes the plaintext of each TLS session as a plain TCP session with the same addresses and ports (synthetic frames with correct checksums, at most 1,400 bytes of payload per segment), and scores that view with the realism runner. It fails when a TLS session does not decrypt, when a session lacks an HTTP response for every request that tshark parses, or when the nDPI ratio is below 0.99. A capture without keys fails it with 0 of 80 decrypted. The view is built from tshark's output and is not a capture of real plaintext traffic, so it shows that the decrypted bytes are HTTP that nDPI recognizes and nothing about the TLS record layer.

Zeek was not installed when we recorded the capture, so the report covers tshark only. The `mod-http1` workflow scores the committed capture and also records and scores a fresh one on every change. Chrome, Edge, and Safari share one `expected_ja4`, because rustls cannot send what separates their ClientHellos, so the JA4 check cannot tell their sessions apart.
