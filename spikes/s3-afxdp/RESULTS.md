# Spike S3 results (small host)

Run on 2026-11-02 following `RUNBOOK.md` at the **Small** size: `VCPUS=3 MEM_MB=3072 QUEUES=2 QUEUE_SET="1 2"`. Every result here is a **small host** result. The 10 Gbps verdict is indicative only, and the run can't show scaling beyond two queues. One row per receiver (`"role":"rx"`) line in `results/s3.jsonl`, driver `virtio_net`, 30 second tests.

| Guest kernel | Mode | Frame bytes | Queues | vCPU | Median Rx pps | Line Gbps | Zero copy log lines |
|---|---|---|---|---|---|---|---|
| 6.8.0-147-generic | default | 64 | 1 | 3 | 900364 | 0.605 | 1 |
| 6.8.0-147-generic | copy | 64 | 1 | 3 | 929874 | 0.625 | 1 |
| 6.8.0-147-generic | default | 1500 | 1 | 3 | 531629 | 6.465 | 1 |
| 6.8.0-147-generic | copy | 1500 | 1 | 3 | 781948 | 9.508 | 1 |
| 6.8.0-147-generic | default | 64 | 2 | 3 | 986637 | 0.663 | 1 |
| 6.8.0-147-generic | copy | 64 | 2 | 3 | 1083665 | 0.728 | 1 |
| 6.8.0-147-generic | default | 1500 | 2 | 3 | 845141 | 10.277 | 1 |
| 6.8.0-147-generic | copy | 1500 | 2 | 3 | 705670 | 8.581 | 1 |
| 7.0.0-42-generic | default | 64 | 1 | 3 | 900835 | 0.605 | 1 |
| 7.0.0-42-generic | copy | 64 | 1 | 3 | 914234 | 0.614 | 1 |
| 7.0.0-42-generic | default | 1500 | 1 | 3 | 819725 | 9.968 | 1 |
| 7.0.0-42-generic | copy | 1500 | 1 | 3 | 837527 | 10.184 | 1 |
| 7.0.0-42-generic | default | 64 | 2 | 3 | 1335363 | 0.897 | 1 |
| 7.0.0-42-generic | copy | 64 | 2 | 3 | 1332517 | 0.895 | 1 |
| 7.0.0-42-generic | default | 1500 | 2 | 3 | 862103 | 10.483 | 1 |
| 7.0.0-42-generic | copy | 1500 | 2 | 3 | 866294 | 10.534 | 1 |

**Size:** Small (`VCPUS=3 MEM_MB=3072 QUEUES=2 QUEUE_SET="1 2"`), small host.

**10 Gbps verdict (813,000 pps at 1,500 bytes): pass, indicative.** Five 1,500 byte receiver rows reach 813,000 pps:

- 7.0.0-42-generic (HWE), copy, 2 queues: 866,294 pps (best)
- 7.0.0-42-generic (HWE), default, 2 queues: 862,103 pps
- 6.8.0-147-generic (GA), default, 2 queues: 845,141 pps
- 7.0.0-42-generic (HWE), copy, 1 queue: 837,527 pps
- 7.0.0-42-generic (HWE), default, 1 queue: 819,725 pps

The best 64 byte receiver rate is 1,335,363 pps (HWE, default, 2 queues).

**Default versus forced copy:** default mode didn't consistently beat `force_copy=1`. Copy was faster in 5 of the 8 pairs. Under HWE the two modes are within about 2% of each other in every pair. Under GA the results go both ways, by large margins. The `zero_copy_log_lines` count is 1 in every row, including the forced copy rows, because it matches DPDK's `Zero copy between umem and mbuf enabled` message. That message is about DPDK's buffer handling, not the kernel's XSK bind mode. So these logs don't show that default mode actually ran in zero copy.

**Variance:** an earlier GA run on this host, discarded because its HWE half failed, gave noticeably different numbers. For example, 1,500 bytes, default, 2 queues measured 729,276 pps there and 845,141 pps here, and 1,500 bytes, default, 1 queue measured 653,178 pps there and 531,629 pps here. Treat differences of less than about 20% between rows as noise on this host.

**Host** (from `results/host.txt`):

- CPU: Intel(R) Core(TM) i7-8705G CPU @ 3.10GHz, 8 threads
- Host kernel: 7.0.0-42-generic
- QEMU: QEMU emulator version 10.2.1 (Debian 1:10.2.1+ds-1ubuntu3.3)

**Changes made during the run:**

- `guest/user-data.yaml` now persists `vm.nr_hugepages=1024` in `/etc/sysctl.d`. Without it, DPDK failed after the HWE reboot with `No free 2048 kB hugepages`.
- `net.bridge.bridge-nf-call-iptables` was not changed.
