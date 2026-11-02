# Spike S3: AF_XDP on virtio-net

S3 measures the AF_XDP packet rate inside KVM guests with virtio-net and vhost-net. The result decides whether tier T1 (10 Gbps per agent on 16 vCPU) is reachable with AF_XDP on KVM, and which Ubuntu LTS kernel the appliance needs (decision D6).

## Host requirements

The test runs two Linux guests on one host and sends traffic between them over a private bridge. It needs no physical test NIC.

| | Minimum | Full T1 test |
|---|---|---|
| CPU | x86_64, 8 cores / 16 threads | 32 cores / 64 threads |
| RAM | 16 GB | 32 GB |
| Free disk | 20 GB | 40 GB |
| Guest size | `VCPUS=4 MEM_MB=4096` | `VCPUS=16 MEM_MB=8192` |
| Queue counts | `QUEUE_SET="1 2"` | `QUEUE_SET="1 2 4 8"` |

The host must run Linux on bare metal with virtualization enabled in the firmware, so that `/dev/kvm` exists. It needs root access, internet access for the Ubuntu cloud image (about 600 MB) and guest packages, and the packages `qemu-system-x86 qemu-utils cloud-image-utils curl`.

## Method

1. `host/launch-vms.sh` boots two Ubuntu 24.04 guests from the official cloud image. A private bridge joins their multiqueue virtio-net test interfaces, and a user mode NIC carries management traffic.
2. `host/run-all.sh` waits for both guests, then runs every combination of frame size (64 and 1,500 bytes), mode (driver default and forced copy), and queue count. For each combination it starts the receiver, waits 3 seconds, and runs the sender for 30 seconds.
3. In each guest, `guest/run-afxdp.sh` runs DPDK `testpmd` with the AF_XDP poll mode driver (`rxonly` or `txonly`). It reports the median of the non zero per second packet rates, the line rate in Gbps, the kernel, the driver, and the queue count as one JSON line.
4. `KERNEL=hwe host/run-all.sh` installs the Ubuntu hardware enablement kernel in both guests, reboots them, and repeats the measurements.

## Running

```bash
sudo apt install -y qemu-system-x86 qemu-utils cloud-image-utils curl
sudo VCPUS=16 MEM_MB=8192 QUEUES=8 spikes/s3-afxdp/host/launch-vms.sh
sudo QUEUE_SET="1 2 4 8" spikes/s3-afxdp/host/run-all.sh
sudo KERNEL=hwe QUEUE_SET="1 2 4 8" spikes/s3-afxdp/host/run-all.sh
sudo spikes/s3-afxdp/host/stop-vms.sh
```

Results land in `spikes/s3-afxdp/results/s3.jsonl`, with guest logs in `logs-*.tgz` and host details in `host.txt`. A queue count must stay below the guest's vCPU count, because `testpmd` uses one core per queue plus a main core.

## Pass criterion

AF_XDP reaches 10 Gbps of 1,500 byte frames (about 813,000 frames per second) on 16 vCPU. If it does not, we plan a DPDK virtio PMD path for tier T1.
