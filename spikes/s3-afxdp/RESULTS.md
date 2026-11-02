# Spike S3 results

Status: **blocked, needs a KVM host.** The Phase 0 build container exposes no `/dev/kvm`. The harness in this directory is complete and passes `shellcheck`.

Fill one row per JSON line that `guest/run-afxdp.sh` writes on the receiver.

| Kernel | Mode | Frame bytes | Queues | vCPU | Median Rx pps | Line Gbps | Zero copy observed |
|---|---|---|---|---|---|---|---|
| 6.8 (GA) | default | 64 | 1 | 16 | | | |
| 6.8 (GA) | copy | 64 | 1 | 16 | | | |
| 6.8 (GA) | default | 1500 | 1 | 16 | | | |
| 6.8 (GA) | copy | 1500 | 1 | 16 | | | |
| HWE | default | 64 | 1 | 16 | | | |
| HWE | copy | 64 | 1 | 16 | | | |
| HWE | default | 1500 | 1 | 16 | | | |
| HWE | copy | 1500 | 1 | 16 | | | |

Record the host CPU model, host kernel, and QEMU version with the results.
