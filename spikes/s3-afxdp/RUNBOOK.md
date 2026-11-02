# Spike S3 runbook for an agent

Follow these steps on the Linux host that will run spike S3. Stop and report if any check fails; do not work around a failed check.

## 1. Check the host

Run these checks and record the output.

```bash
uname -m            # must print x86_64
ls -l /dev/kvm      # must exist
nproc               # hardware threads
free -g             # total memory
df -h /var/tmp      # free disk
systemd-detect-virt # must print "none" (bare metal)
```

Pick the size from the results.

| Host | Size |
|---|---|
| 40 or more threads, 32 GB or more RAM, 40 GB or more free disk | Full: `VCPUS=16 MEM_MB=8192 QUEUES=8 QUEUE_SET="1 2 4 8"` |
| 16 or more threads, 16 GB or more RAM, 20 GB or more free disk | Minimum: `VCPUS=4 MEM_MB=4096 QUEUES=2 QUEUE_SET="1 2"` |
| 8 or more threads, 12 GB or more RAM, 20 GB or more free disk | Small: `VCPUS=3 MEM_MB=3072 QUEUES=2 QUEUE_SET="1 2"` |
| Anything smaller | Stop and report the host details |

The small size gives each VM one main core and two forwarding cores, which leaves two host threads for QEMU and the vhost workers. It measures the packet rate per queue, which is the number that sets the AF_XDP defaults. It cannot show scaling beyond two queues, so label every result from a small host as "small host" and treat the 10 Gbps verdict as indicative.

## 2. Prepare

```bash
git clone https://github.com/alrawi/FixingPointSystem && cd FixingPointSystem
git checkout -b s3-results
sudo apt-get update && sudo apt-get install -y qemu-system-x86 qemu-utils cloud-image-utils curl
```

## 3. Run

Use the size you picked in every command.

```bash
sudo VCPUS=… MEM_MB=… QUEUES=… spikes/s3-afxdp/host/launch-vms.sh
sudo QUEUE_SET="…" spikes/s3-afxdp/host/run-all.sh
sudo KERNEL=hwe QUEUE_SET="…" spikes/s3-afxdp/host/run-all.sh
sudo spikes/s3-afxdp/host/stop-vms.sh
```

After `launch-vms.sh` returns, confirm in both guests that `command -v dpdk-testpmd` prints a path and that `ls /usr/lib/x86_64-linux-gnu/dpdk/pmds-24.0/librte_net_af_xdp.so.24` succeeds. Use `ssh -i $WORK/id_ed25519 -p 2221 ubuntu@127.0.0.1` for the sender and port 2222 for the receiver. Stop and report if either check fails.

The first boot takes several minutes while cloud-init installs packages. If every receiver row reports `median_pps` of 0 while the sender rows do not, run `sudo sysctl -w net.bridge.bridge-nf-call-iptables=0`, then repeat the run step. Report that you changed this setting.

## 4. Record

1. `sudo chown -R "$USER" spikes/s3-afxdp/results`
2. Replace the table in `spikes/s3-afxdp/RESULTS.md` with one row per receiver (`"role":"rx"`) line in `results/s3.jsonl`. Use the guest kernel string from the JSON.
3. Under the table, state the size you used. State whether any 1,500 byte receiver row reaches 813,000 packets per second (10 Gbps), and for which kernel, mode, and queue count. Add the host CPU, host kernel, and QEMU version from `results/host.txt`.
4. Commit `spikes/s3-afxdp/RESULTS.md` and `spikes/s3-afxdp/results/`, then push the `s3-results` branch. If the push fails, report the path of the results directory.

## 5. Report

Report the pass or fail verdict against 813,000 packets per second, the best 1,500 byte and 64 byte receiver rates, whether zero copy mode beat forced copy mode, and any step that failed.
