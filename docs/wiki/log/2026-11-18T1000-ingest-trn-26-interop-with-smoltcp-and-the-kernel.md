## [2026-11-18] ingest | TRN-26 interop of the own stack with smoltcp and the kernel

TRN-26 tests the own stack against smoltcp over the lossy link in memory and against the kernel over AF_XDP and a veth pair with netem, in both directions, with a byte checked loss workload and with checks of the TCP options on the wire. It edits only `engine/fps-tcp/`, its handoff, this entry, and `Transport.md`.

- `wiretap.rs`: a tap that reads frames with the smoltcp parser (independent of both stacks) and summarizes MSS, window scale, SACK permitted, timestamps, their use, window sizes, SACK blocks, and checksums. `violations()` compares a summary with what the RFCs ask.
- `interop.rs` and `SmolStack::bridged`: one own endpoint and one smoltcp endpoint over the TRN-22 impairment model with Ethernet framing between them (the MAC addresses come from the station tables, so no ARP). `tests/interop_smol.rs` has 8 tests, no root.
- `interop_kernel.rs` and `tests/interop_kernel.rs` (`harness = false`): the own stack in a helper process over an AF_XDP queue on `vc` or `vs` of the TRN-22 namespace link, the kernel as a socket peer at the other end, 10 scenarios (clean link, tail drop bottleneck, 1 percent loss with 20 ms, kernel options off, own options off; both directions each).
- `LossConfig::verify`: a position dependent pattern that the receiver checks byte by byte. The kernel peers take `--verify`.

Findings:

- Frames from a kernel socket reach an AF_XDP queue with a partial TCP checksum (the veth keeps `CHECKSUM_PARTIAL`), so the own stack drops them and the handshake never completes. The harness turns the transmit offload off on every interface, with `ethtool` or, because the hosted runner has none, with the ethtool ioctl in the test binary. Any package that runs a kernel peer against an AF_XDP veth end (TRN-12, TRN-28) meets this.
- The own stack has no ARP requests, so the router interface that faces it gets the MAC address that the own stack uses for the next hop.
- smoltcp 0.14 offers MSS, window scale, and SACK and never timestamps, so timestamps are not agreed with it, and the own stack falls back cleanly.
- The own stack with all options completes the bottleneck row against the kernel more slowly than with options off in one direction (11.7 against 19.4 Mbit/s, own client, sandbox, debug build). Not investigated; a question for TRN-12.

Results are in `engine/fps-tcp/results/sandbox-2vcpu-trn26/` (sandbox: no netem, so the two 1 percent rows ran in CI only: 5.15 Mbit/s with 20 retransmissions own client to kernel, 4.50 with 11 kernel client to own server). The own client reached 3.6 Mbit/s with 517 retransmissions on the bottleneck row on the hosted runner (11.7 and 41 in the sandbox); a question for TRN-12.

### Shared page notes

- Testing page: `cargo test -p fps-tcp --test interop_smol` needs no root. `cargo test -p fps-tcp --test interop_kernel` needs root (re-runs through `sudo -n`), AF_XDP, `ip`, `tc`, and for the 1 percent rows `sch_netem`. `CI=true` turns every skip into a failure, `FPS_REQUIRE_AFXDP=1` and `FPS_REQUIRE_NETEM=1` do too. `FPS_INTEROP_RESULTS=<dir>` writes `interop-kernel.txt`.
- Workflow (coordinator files): no change is needed for the acceptance. The `rust` job of `ci.yml` (`cargo test --workspace`, `CI=true`) ran all 10 kernel scenarios, the two 1 percent loss rows with netem included, and the 8 smoltcp tests on the hosted runner (PR 651, job 134684988626); a missing `sch_netem` would have failed it. Optional, to keep the numbers as an artifact and to start the test when only `engine/fps-tcp/**` changes (the `trn.yml` path filters do not list it), add this job to `trn.yml` and `engine/fps-tcp/**` to both path filters:

  ```yaml
    interop:
      runs-on: ubuntu-24.04
      timeout-minutes: 20
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - name: Install iproute2 and the netem module
          run: |
            set -euo pipefail
            tools/ci/apt-install.sh iproute2
            if ! sudo modprobe sch_netem; then
              tools/ci/apt-install.sh "linux-modules-extra-$(uname -r)"
              sudo modprobe sch_netem
            fi
        - name: Own stack against smoltcp and the kernel
          run: cargo test -p fps-tcp --test interop_smol --test interop_kernel -- --nocapture
          env:
            FPS_REQUIRE_AFXDP: "1"
            FPS_REQUIRE_NETEM: "1"
            FPS_INTEROP_RESULTS: ${{ runner.temp }}/interop
        - uses: actions/upload-artifact@v4
          if: always()
          with:
            name: interop-results
            path: ${{ runner.temp }}/interop
            if-no-files-found: ignore
  ```

  Through `tcp-bench.yml` it is easier: in the `loss` job, after the step `Install netem and namespace tools`, add the step below, then dispatch with `job: loss`.

  ```yaml
        - name: Interop of the own stack with the kernel
          run: cargo test -p fps-tcp --test interop_kernel -- --nocapture
          env:
            FPS_REQUIRE_AFXDP: "1"
            FPS_REQUIRE_NETEM: "1"
  ```
- `Cargo.lock` changed: `libc` is a dev-dependency of `fps-tcp` (the ethtool ioctl in the test binary). No root `Cargo.toml` edit.
- Transport page: this package added "Interop of the own stack with smoltcp and the kernel (TRN-26)" with the matrix.
- Module development: nothing changes for module authors.
