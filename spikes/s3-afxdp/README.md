# Spike S3: AF_XDP on virtio-net

S3 measures the AF_XDP packet rate inside KVM guests with virtio-net and vhost-net. The result decides whether tier T1 (10 Gbps per agent on 16 vCPU) is reachable with AF_XDP on KVM, and which Ubuntu LTS kernel the appliance needs (decision D6).

## Method

1. `host/launch-vms.sh` boots two Ubuntu 24.04 guests from the official cloud image. A private bridge joins their multiqueue virtio-net test interfaces, and a second user mode NIC carries management traffic.
2. In each guest, `guest/run-afxdp.sh` runs DPDK `testpmd` with the AF_XDP poll mode driver. The receiver runs `rxonly` and the sender runs `txonly`, for 64 and 1,500 byte frames. Each size runs twice, once with the driver default (zero copy when the kernel driver supports it) and once with `force_copy=1`.
3. The script records the median of the per second packet rates after a five second warm up, the line rate in Gbps, the kernel, the driver, and the queue count, in one JSON line per test.
4. Repeat steps 2 and 3 with the GA kernel and with the hardware enablement kernel (`sudo apt install linux-generic-hwe-24.04`, then reboot).

## Running

```bash
# On a KVM host, as root
SSH_KEY=~/.ssh/id_ed25519.pub VCPUS=16 QUEUES=8 ./host/launch-vms.sh
scp -P 2222 guest/run-afxdp.sh ubuntu@127.0.0.1:   # receiver
scp -P 2221 guest/run-afxdp.sh ubuntu@127.0.0.1:   # sender
ssh -p 2222 ubuntu@127.0.0.1 sudo ./run-afxdp.sh rx &
ssh -p 2221 ubuntu@127.0.0.1 sudo ./run-afxdp.sh tx
./host/stop-vms.sh
```

Start the receiver first. Both sides walk the same size and mode sequence with the same per test duration, so they stay aligned. Set `QUEUES=8` in the guests to measure multiqueue scaling.

## Pass criterion

AF_XDP reaches 10 Gbps of 1,500 byte frames (about 813,000 frames per second) on 16 vCPU. If it does not, we plan a DPDK virtio PMD path for tier T1.

## Status

Not run yet. The Phase 0 build environment has no `/dev/kvm`, so this spike needs one of the owner's KVM hosts. See `RESULTS.md`.
