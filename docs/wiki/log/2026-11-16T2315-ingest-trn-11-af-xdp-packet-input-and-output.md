## [2026-11-16] ingest | TRN-11 AF_XDP packet input and output

TRN-11 (issue #562, batch 36) added the crate `engine/fps-backend-afxdp`, which gives a user space transport whole Ethernet frames in batches on one queue of a network device. `FrameIo` is the published frame trait, with `recv_batch` and `send_batch_with` that lend frame memory and never block, the copying `send_batch`, `flush`, `wait_readable`, and counters. `open_port` opens one AF_XDP socket per queue, each with its own UMEM and the fill, completion, receive, and transmit rings, attaches a six instruction XDP program through a BPF link, and returns an `AfXdpQueue` per queue. A queue binds with the kernel's choice of zero copy or copy mode, and the mode that the socket reports through `XDP_OPTIONS` is the one that the crate returns. `Workers` runs one thread per queue, pinned if asked. The defaults of `QueueConfig` come from spike S3. The [Transport](../Transport.md#af_xdp-backend) page describes the design, the defaults, and the limits.

The acceptance test `engine/fps-backend-afxdp/tests/veth.rs` runs on the QA-01 harness. It passes frames of 60 to 1,514 bytes both ways over a veth pair on one queue, and over a veth pair with two queues with one worker per queue on both ends, and each frame comes back intact on the queue that sent it. A request for zero copy fails with `EOPNOTSUPP` on the veth pair, `Auto` falls back to copy mode, and the socket confirms the mode. The test starts itself again through `sudo -n` when it is not root, so the existing `rust` job of `ci.yml` runs it, and it fails instead of skipping when `CI=true`.

The first version of the receive path stalled after exactly `ring_size` frames, because it pushed recycled frames to the fill ring without refreshing the cached consumer index. The veth test found it, and `recv_batch` now refreshes the index before it pushes. The kernel also releases the queue of a closed socket in a work queue, so a bind right after a close can fail with `EBUSY` for a few milliseconds, which the Transport page lists as a limit.

The zero copy bind succeeds nowhere in this package, because veth pairs have no zero copy driver. TRN-19 binds in zero copy mode on virtio-net guests.

### Shared page notes

- [Testing](../Testing.md), table of make targets, row `test-eng`: append "; it includes `fps-backend-afxdp`, whose test `tests/veth.rs` runs AF_XDP queues over veth pairs in the namespace harness. The test needs root, so it starts itself again through `sudo -n` as a normal user, skips with a message where neither works, and fails instead of skipping when `CI=true` or `FPS_REQUIRE_AFXDP=1`".
- [Implementation Plan](../Implementation-Plan.md), table of seams: add the row `| Ethernet frame I/O in batches | `FrameIo` in `engine/fps-backend-afxdp` (TRN-11) | TRN | TRN-12, HST-03, TRN-19 |`.
- Workflow, optional and not needed for the acceptance, because the `rust` job of `ci.yml` already runs the test through `cargo test --workspace`. A dedicated job in `.github/workflows/trn.yml` makes a skip impossible and keeps the captured output. Add `engine/fps-backend-afxdp/**` to both `paths` lists of the file, and add this job.

  ```yaml
    afxdp:
      runs-on: ubuntu-24.04
      timeout-minutes: 15
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - name: AF_XDP veth test
          run: cargo test -p fps-backend-afxdp -- --nocapture
          env:
            FPS_REQUIRE_AFXDP: "1"
  ```

- Module Development: no change, because the package changes nothing about how a module is written.
- `Cargo.lock` gains one entry for the new crate, which depends on `libc` and `thiserror` 2, both already in the lock file.
