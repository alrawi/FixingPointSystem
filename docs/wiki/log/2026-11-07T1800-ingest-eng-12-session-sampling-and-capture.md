## [2026-11-07] ingest | ENG-12 session sampling and capture

ENG-12 added the crate `engine/fps-capture`. It samples sessions by a seeded rate or every nth session, records the bytes of a sampled session through `Tapped` and `TappedDatagram` wrappers, synthesizes Ethernet, IPv4, IPv6, TCP, and UDP frames around them, and writes the frames into a bounded ring of pcapng files with the TLS key log of each session embedded as a decryption secrets block, written next to the file, or both. We updated [Engine](../Engine.md) with a section on sampling, the cost when sampling is off, frame synthesis, the ring, the key log modes, and the tests.

The tshark tests of the crate skip themselves without tshark and fail with `FPS_REQUIRE_CAPTURE_TOOLS=1`. They passed locally on tshark 4.2.2, but no workflow installs tshark for the crate yet.

### Shared page notes

1. Testing, `test-eng` row: the target already runs `cargo test -p fps-capture` through the crate glob in `mk/eng.mk`. The row could name the capture tests, which read the ring back with an independent pcapng reader and, where tshark exists, decrypt TLS sessions from the embedded secrets and from the sidecar key log.
2. CI: no workflow runs the tshark tests of `fps-capture` with `FPS_REQUIRE_CAPTURE_TOOLS=1`. A job in `eng.yml` or a new `eng-capture.yml` could install tshark as the `mod-tls` workflow does and run `cargo test -p fps-capture --test capture`.
3. Implementation Plan and Home: the Engine row may list `fps-capture` as built. The open points that AGT-05 must settle are in the last section of Engine.
