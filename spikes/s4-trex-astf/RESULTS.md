# Spike S4 results

TRex ASTF can express deterministic cleartext scenarios, but it performs no real TLS key exchange and cannot follow content that the device under test changes. We therefore recommend against TRex ASTF as the general tier T2 backend.

## Translation coverage

We translated both example tests on 2026-11-02. Generated profiles and JSON reports are in `results/`.

| Test | Actions | Exact | Approximate | Unsupported | Exact share |
|---|---|---|---|---|---|
| `http_smoke` (`http1/web_get`) | 11 | 8 | 3 | 0 | 73% |
| `enterprise_mix_10g` (`web_browse`, `web_get`, `dns/lookup`) | 36 | 22 | 13 | 1 | 61% |

The translated `http1/web_get` program carries 17,244 payload bytes per session. Spike S1 measured the same 17,244 application bytes per session when it ran this scenario over loopback.

The losses fall into five classes.

| Loss | Verdict | Effect on realism |
|---|---|---|
| `expect pattern` and `expect min_bytes` become fixed byte watermarks | Approximate | Exact when both sides are emulated. A DUT that changes payload length, such as a proxy that adds headers or a device that compresses content, leaves the watermark unmet and the flow stalls. |
| `fill()` becomes one repeated byte | Approximate | Every body is identical, so the duplicate payload ratio becomes 100% and the content variety target fails. |
| Lognormal, exponential, and Pareto delays | Unsupported | ASTF offers fixed and uniform delays only, so think time distributions are lost. |
| Load ramps and objectives other than connection rate | Approximate | A driver must update the rate multiplier at run time, and throughput objectives convert to connection rates through an estimate of bytes per session. |
| `branch`, `set`, `rand_int`, `session.*`, `spawn`, and ordering across flows | Unsupported | ASTF programs are fixed byte scripts with counters, and templates run independently. |

## TLS in ASTF

We reviewed the TRex master branch at commit 27e0153 (2025-11-05), whose `VERSION` file reads v3.08.

1. The ASTF guide (`doc/trex_astf.asciidoc`, line 71) lists "Simulate L7 application on top of TLS using OpenSSL" among the mode's goals, and line 108 lists "TLS support" under "On radar". The "What works" list does not mention TLS.
2. `git grep` finds no OpenSSL include and no `SSL_CTX_new`, `SSL_read`, or `SSL_write` call in `src/` outside the bundled DPDK tree.
3. The bundled OpenSSL wrapper (`scripts/external_libs/trex-openssl/trex_openssl.py`) serves DTLS in the stateless wireless services (`trex_service_ap.py`, `trex_stl_wlc.py`), not ASTF.
4. The ASTF HTTPS example (guide lines 541 to 582) replays a captured HTTPS session from `avl/delay_10_https_0.pcap`.

ASTF therefore replays recorded TLS bytes. Every session repeats the same client random, key share, and fingerprint, and a TLS inspection device cannot complete a handshake with it.

## Recommendation for decision D1

- Do not adopt TRex ASTF as the tier T2 backend. It fails the encryption, content variety, and temporal realism properties of the specification.
- Keep the translator as an optional export for cleartext, deterministic throughput tests on existing TRex installations.
- Evaluate a DPDK backend with a user space TCP stack, starting with F-Stack, in a Phase 3 spike. The S1 interpreter already runs over any byte stream through its `SessionIo` trait, so an F-Stack socket adapter that implements that trait can plug in without interpreter changes.
