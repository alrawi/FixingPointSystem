# Spike S1 results

We ran the suite in `run-bench.sh` on 2026-11-02 in the Phase 0 build environment, a 2 vCPU Firecracker microVM (Intel Xeon at 2.10 GHz, Linux 6.18). Every run emulates clients and servers in one process over loopback, so each figure includes the cost of both sides. Raw JSON is in `results/phase0-container/`.

## Interpreter overhead

| Workload | Interpreter sessions/s | Baseline sessions/s | Interpreter vs baseline |
|---|---|---|---|
| Keepalive, 4 requests of 4 KiB per session (mean of 2 runs) | 14,841 | 15,474 | 4.1% lower |
| Connection rate, 1 request with an empty body | 27,693 | 28,010 | 1.1% lower |
| Bulk, 4 requests of 1 MiB per session | 1,211 | 1,639 | 26.1% lower |

The interpreter meets the 25% overhead criterion for keepalive and connection rate workloads and misses it by 1.1 points for 1 MiB bodies.

During the spike we found that `expect bytes` grew the pattern buffer for bulk bodies. Discarding bodies through a reused 64 KiB scratch buffer raised bulk throughput from 34.1 to 40.7 Gbit/s. The remaining gap most likely comes from rendering a fresh 1 MiB `fill()` body per response, a copy the baseline avoids by sending one pre rendered buffer. Per session content is a realism requirement, so Phase 1 should keep it and remove the copy by writing `fill()` segments as vectored writes that point into the shared filler buffer. We have not profiled this hypothesis.

## Scale on 2 vCPU

| Workload | Sessions/s | L7 throughput | Session latency p50 / p99 | Failures |
|---|---|---|---|---|
| Keepalive interpreter, 64 concurrent | 14,739 | 2.03 Gbit/s | 4.3 / 6.3 ms | 0 |
| Connection rate interpreter, 64 concurrent | 27,693 | 0.05 Gbit/s | 2.2 / 4.1 ms | 0 |
| Connection rate interpreter, 1 runtime thread | 26,615 | 0.05 Gbit/s | 2.1 / 5.3 ms | 0 |
| Bulk interpreter, 16 concurrent | 1,211 | 40.66 Gbit/s | 12.9 / 20.1 ms | 0 |
| `web_browse` with lognormal think time, 4,000 concurrent | 8,138 | 4.58 Gbit/s | 446 / 1,014 ms | 0 |

The `web_browse` run held 4,000 concurrent sessions without failures. The two vCPUs were saturated (1.78 cores busy), so session latency reflects queueing behind CPU and not think time alone.

## Tier T0 assessment

Tier T0 asks for 5,000 new sessions per second and 1 Gbit/s on 4 vCPU. The kernel backend exceeded both on 2 vCPU while emulating both sides, with 27,693 sessions per second and up to 40.66 Gbit/s.

Three limits apply to this result.

- Loopback skips NIC drivers, virtio, and the wire, so these figures bound the interpreter and the TCP stack from above.
- The single thread run reports 0.98 busy cores but reaches 96% of the two thread rate. The kernel processes loopback traffic partly in softirq context outside the process, so per core figures from loopback understate the true CPU cost.
- Spike S3 measures the virtio path that loopback hides.

## Recommendation

Keep the bytecode interpreter design for Phase 1. Add vectored `fill()` writes, then rerun the bulk workload against the 25% criterion on a 4 vCPU KVM guest with virtio-net.
