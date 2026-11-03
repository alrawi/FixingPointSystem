# http1 module

The `http1` module generates HTTP/1.0 and HTTP/1.1 traffic between a client and a server side. Its native crate `fps-module-http1` implements the template functions `http1.request` and `http1.response` in Rust under decision D10, so scenarios render every message without a WebAssembly runtime. Every body comes from the seeded `fill()` generator, and every host name lies under the reserved `.test` domain.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Six flow programs, listed below |
| `native/` | The crate `fps-module-http1` with the `TemplateFn` implementation `Http1` and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture |
| `tests/` | Catalog, interpreter, and golden transcript tests, with transcripts in `tests/testdata/` |
| `fixtures/` | The synthetic capture `http1.pcapng`, its labels `expected.yaml`, its realism report, and `capture.sh`, which records it |

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

The parameter schema in the manifest names every variable that the scenarios declare, with its default and its bounds.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-http1     # validate.py on the manifest and every scenario
make test-mod-http1     # cargo test -p fps-module-http1, then the realism score of the fixture
sudo make e2e-mod-http1 # record and score a fresh capture in the namespace harness
```

The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default. The scenario tests run every scenario with both sides over the in memory duplex for five session ids and parse both directions with the framing rules of RFC 9112, so they check the message count, the versions, the status codes, the body sizes, and the framing of every message. The transcript test renders every scenario with seed 42 and session id 1 and compares both directions with `tests/testdata/<scenario>.seed42.c2s.bin` and `.s2c.bin`. It shrinks the body sizes to keep the golden files reviewable, and `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer` and the kernel TCP backend, records the router's client arm, and scores the capture with the realism runner. It runs 20 sessions per scenario with small bodies, so the capture holds 120 sessions in about 600 KB.

The committed capture scores as follows. nDPI 4.2.0 classifies 120 of 120 sessions as HTTP from payload alone, so the conformance ratio is 1.0 against the threshold 0.99, and tshark 4.2.2 reports no parser errors and no malformed packets. Zeek was not installed when we recorded it, so the report covers tshark only. The `mod-http1` workflow scores the committed capture and also records and scores a fresh one on every change.
