## [2026-11-06] ingest | QA-08 owner host baseline

The owner registered the `fps-bench` runner on an 8 vCPU Intel Core i7-8705G with Linux 7.0. The first scheduled run and a manual run timed out on almost every scenario after the first, because Tailscale loads conntrack and the suite filled its 262,144-entry table. The owner applied a oneshot unit that exempts loopback from conntrack and sets the `performance` governor, and the next run, 42231498234, finished with no failures. We recorded it as `bench/baselines/s1-engine-owner-host.json` and documented the host tuning in `bench/README.md` and [Benchmarks](../Benchmarks.md).

### Shared page notes

None.
