# Spike S3 results

Status: **not run yet.** The harness is ready for a bare metal Linux host with KVM (see the host requirements in `README.md`). `host/run-all.sh` writes `results/s3.jsonl`, `results/host.txt`, and guest logs; copy the receiver rows into the table below.

| Guest kernel | Mode | Frame bytes | Queues | vCPU | Median Rx pps | Line Gbps | Zero copy log lines |
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
