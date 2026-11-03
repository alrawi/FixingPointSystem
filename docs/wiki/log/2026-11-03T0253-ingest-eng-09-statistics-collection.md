## [2026-11-03] ingest | ENG-09 Statistics collection

We implemented `fps-stats` with per worker shards keyed by the catalog names of `contracts/metrics/catalog.yaml`, HDR histograms, the merge of cumulative worker snapshots, and the proto3 JSON mapping of `CounterSnapshot`. The [Engine](Engine.md) page gained the section "Statistics collection", which records the recording rules, the merge, and the JSON form. The crate depends on `hdrhistogram` with only its serialization feature, `base64`, `serde_json`, and `thiserror`, and its tests on `serde`, `serde_norway`, and `proptest`. The evidence is `cargo test -p fps-stats` and `docs/handoff/ENG-09.md` on the branch `wp/ENG-09`.

## Shared page notes

The crates table on the [Engine](Engine.md) page still says that `fps-stats` is empty. We left the table alone because other engine packages edit the page in the same batch. The coordinator should replace the `fps-stats` row with the text below.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-stats`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-stats) | The catalog table, per worker shards, HDR histograms, the snapshot merge, and the proto3 JSON mapping of `CounterSnapshot` | ENG-10 publishes one snapshot per second from the worker runtime |
