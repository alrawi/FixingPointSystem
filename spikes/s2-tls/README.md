# Spike S2: TLS cost per core

S2 measures how many TLS handshakes one core completes for the client side, the server side, and both sides together. The results set the TLS performance targets.

## Method

- A rustls client and server run in memory on one thread, so the figures exclude network and kernel cost.
- We time client calls and server calls separately. An agent emulates only clients in client simulation and both sides in application simulation.
- The server presents a two certificate chain (leaf and ECDSA P-256 CA). The client verifies it, as a browser would.
- Profiles cover TLS 1.3 and 1.2, ECDSA P-256 and RSA 2048 leaf keys, X25519 and secp256r1 key exchange, and full and resumed handshakes. TLS 1.3 handshakes include delivery of the server's session tickets.
- RSA keys come from the `openssl` command, because the ring provider does not generate RSA keys.

We use the ring crypto provider. Its license (Apache-2.0 AND ISC) is compatible with GPL-3.0. The crates.io metadata for aws-lc-sys 0.47.0 also lists only GPL-3.0 compatible licenses, so a later switch to aws-lc-rs remains open.

## Usage

```bash
cargo run --release -p fps-spike-tls -- --duration 4 --json out.json
```

## Results

See `RESULTS.md`.
