# tls module

The `tls` module wraps sessions in TLS 1.2 and 1.3. Its crate `fps-module-tls` takes any `fps_io::SessionIo`, runs the handshake with rustls and the ring crypto provider, and returns a `TlsSession`, which is again a `SessionIo`. The interpreter therefore runs a flow program over TLS without change. The module is a transport wrapper and has no manifest, scenarios, or template functions. Application modules such as `http1` reach it through the opener of a session.

The [Transport](../../docs/wiki/Transport.md#tls-module) wiki page explains the design and the results. This page lists the layout and the commands.

## Layout

| Path | Contents |
|---|---|
| `native/src/profile.rs` | `ClientProfile` and `ServerProfile` with `TlsVersion`, `Group`, `CipherSuite`, and the resumption policies |
| `native/src/browser.rs` | `BrowserProfile` and `BROWSER_PROFILES`, the Chrome, Edge, Firefox, and Safari profiles with their expected and real JA4 and the gaps between them |
| `native/src/authority.rs` | `RunAuthority`, which generates a certificate authority per run and issues `ServerIdentity` chains with `LeafKey` |
| `native/src/session.rs` | `TlsConnector`, `TlsAcceptor`, `TlsSession`, and `HandshakeReport` |
| `native/src/keylog.rs` | `NssKeyLog`, which writes the NSS key log format |
| `native/src/error.rs` | `TlsError` with the alert names of the `alert` metric label, and `Side` with the `side` label values |
| `native/examples/handshake_bench.rs` | The handshake rate benchmark, measured the way spike S2 does |
| `tests/sessions.rs` | Component tests over in memory sessions and kernel TCP |
| `tests/capture.rs`, `tests/decrypt_tls13.py` | Capture and key log tests with tshark and with an independent Python decryption |
| `tests/ja4.rs`, `tests/ja4_of_hello.py` | JA4 tests of the browser profiles, computed with `realism/ja4.py` and cross checked with tshark |
| `bench/` | `run_s2.sh` and `compare_s2.py`, which compare the module with the S2 spike on one machine |
| `results/phase2-sandbox/` | The raw benchmark results of the package, see the wiki page |

## Commands

```bash
cargo test -p fps-module-tls                       # unit, session, and capture tests
cargo clippy -p fps-module-tls --all-targets -- -D warnings
modules/tls/bench/run_s2.sh 3 3                    # three alternating rounds against spike S2, 3 s per profile
cargo run --release -p fps-module-tls --example handshake_bench -- --mode session --duration 3
FPS_REQUIRE_CAPTURE_TOOLS=1 cargo test -p fps-module-tls --test capture   # fail when tshark or python3 with cryptography is missing
FPS_REQUIRE_CAPTURE_TOOLS=1 cargo test -p fps-module-tls --test ja4       # JA4 of every browser profile, with tshark
```

The JA4 tests need only `python3`, except `tshark_agrees_with_the_expected_ja4`, which needs tshark 4.2 or newer. The test `tshark_decrypts_captures_with_exported_keys` runs only where `tshark` is installed. The test `python_decrypts_a_tls13_capture_with_exported_keys` runs where `python3` can import `cryptography`. With `FPS_REQUIRE_CAPTURE_TOOLS=1` a missing tool fails the test instead of skipping it.
