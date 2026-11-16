# fps-backend-afxdp

AF_XDP packet input and output for the engine. The crate gives a user space transport whole Ethernet frames in batches on one queue of a network device. The [Transport](https://github.com/alrawi/FixingPointSystem/blob/main/docs/wiki/Transport.md#af_xdp-backend) wiki page describes the design, the defaults from spike S3, and the limits.

## Layout

| File | Contents |
|---|---|
| `src/frame.rs` | `FrameIo`, the frame trait with batched receive and send, and `QueueStats` |
| `src/config.rs` | `PortConfig`, `QueueConfig` with the S3 defaults, `BindMode`, and `BoundMode` |
| `src/port.rs` | `open_port`, which opens the queues and attaches the XDP program |
| `src/queue.rs` | `AfXdpQueue`, one socket with its UMEM and four rings |
| `src/ring.rs` | The single producer, single consumer rings shared with the kernel |
| `src/sys.rs` | The system calls of AF_XDP and BPF, the only module besides `ring.rs` with `unsafe` code |
| `src/worker.rs` | `Workers`, one thread per queue |
| `src/memory.rs` | `frame_pair`, an in memory fake of `FrameIo` for tests |
| `tests/veth.rs` | The acceptance test on veth pairs in the namespace harness |

## Commands

```bash
cargo test -p fps-backend-afxdp                      # unit tests, and the veth test through sudo -n
FPS_AFXDP_FRAMES=100000 cargo test -p fps-backend-afxdp --test veth -- --nocapture
cargo clippy -p fps-backend-afxdp --all-targets -- -D warnings
```

The veth test needs root, `ip` from iproute2, and a kernel with AF_XDP and veth. It starts itself again through `sudo -n`, skips with a message where that fails, and fails instead of skipping when `CI=true` or `FPS_REQUIRE_AFXDP=1`.
