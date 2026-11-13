## [2026-11-13] ingest | RLM-22 Request cadence and bytes for HLS in the realism labels

The `temporal` labels can now say what `modules/hls/fixtures/hls_check.py` checks, which MOD-24 could not move into them (see the MOD-24 log entry of 2026-11-12, "Packet gap labels in place of the module timing scripts"). Three additions, all optional, so a labels file without them behaves as before:

- A `temporal.packets` target of the client direction takes `path`, a regular expression on the HTTP request target. The stream of a session then holds only the matching requests, and a gap is the time between two of them, so the segment cadence of a keepalive connection no longer mixes with the playlist refreshes. `join_ms` joins the matching requests of the sessions of an application that follow each other into one stream (the `vod_segments` scenario, one connection per segment). The sessions are read by the new `realism/exchanges.py`, which follows HTTP/1.1 `GET` requests with a `Content-Length` response and fails a target for a session that it cannot follow, as `hls_check.py` did.
- Every target takes `min_samples`, which replaces the value of the section and of `--ks-min-samples` for that target. A paced constant takes `mean_tolerance_ms`, the distance of the mean gap of each stream from its packet time. A stream that fails only in its mean is named in the reason.
- A new `temporal.rungs` section reads the response sizes per rung (`realism/rungs.py`): the request path of a segment with the rung as its first group, `duration_ms`, and `bits_per_second` per rung. A stream with at least `min_samples` requests needs every body within `body_tolerance_bytes` (188) of `rate * duration / 8`, the byte share of each rung (response head and body) within `share_tolerance` (5%, relative), the bit rate of the stream within `rate_tolerance` (5%), and with `walk: up` rungs that rise through the whole ladder. `streams` is the number of judged streams that the capture must hold, a number or a range.

The report format is 0.20.0: the comparisons of the `temporal` entry gained `path` and `min_samples`, and the entry gained `rungs`. All three are optional in the schema, so a 0.19.0 report validates. The controller copy `controller/internal/report/schema/realism-report.schema.json` and `ui/src/api/schema.d.ts` (`node ui/scripts/gen-api.mjs --check` passes) carry the change. The labels schema `realism/schema/labels.schema.json` has the new fields. A comparison with a `path` and the `rungs` add no `realism.*` series (the catalog has none for bytes, and two paths of one application would give two series with equal labels), so a failure shows in `failures` and in `realism.check_failures`.

Evidence is `realism/tests/test_http_rungs.py` (47 tests, pure Python, no tshark and no nDPI). It reads the committed hls fixture and changes nothing under `modules/hls/`. With the labels below the capture passes, and the bit rate and rung shares of its 7 playbacks equal the numbers of `hls.playbacks.json` (375327 against 373333 bit/s for each ladder playback). The same capture with the time inside every playback stretched by 1.15 or 0.8 fails for all three kinds of playback, a stretch of 1.06 (every gap inside 40 ms, the mean 24 ms off) fails only because of `mean_tolerance_ms`, and a ladder whose top rung is 700 kbit/s fails on the committed bytes. Synthetic keepalive playbacks with a playlist request 20 ms before each segment fail for a top rung that is 20% short, a padded response head that moves the share with exact bodies, a bit rate 5% off on the wire, rungs out of order or with a rung skipped, a rung outside the ladder, too few requests, the wrong number of streams, and a session that is not HTTP/1.1. The plain packet gap target fails on the same playlist-and-segment keepalive that the path target passes.

```yaml
temporal:
  alpha: 0.01
  min_samples: 30
  arrivals:
    applications:
      hls_segment: {dist: constant, ms: 400, tolerance_ms: 40}
  packets:
    applications:
      hls_vod:     {direction: client, path: '/v\d+/seg\d+\.ts$', dist: constant, ms: 400, tolerance_ms: 40, within: 0.9, mean_tolerance_ms: 20, min_samples: 3}
      hls_live:    {direction: client, path: '/v\d+/seg\d+\.ts$', dist: constant, ms: 400, tolerance_ms: 40, within: 0.9, mean_tolerance_ms: 20, min_samples: 3}
      hls_segment: {direction: client, path: '/v\d+/seg\d+\.ts$', dist: constant, ms: 400, tolerance_ms: 40, within: 0.9, mean_tolerance_ms: 20, min_samples: 3, join_ms: 800}
  rungs:
    applications:
      hls_vod:     {path: '/v(\d+)/seg\d+\.ts$', duration_ms: 400, bits_per_second: [160000, 320000, 640000], walk: up, streams: 3, min_samples: 4}
      hls_segment: {path: '/v(\d+)/seg\d+\.ts$', duration_ms: 400, bits_per_second: [160000, 320000, 640000], walk: up, streams: 1, min_samples: 4, join_ms: 800}
      hls_live:    {path: '/v(\d+)/seg\d+\.ts$', duration_ms: 400, bits_per_second: [160000, 320000, 640000], streams: 3, min_samples: 4}
```

Not verified here: the Go controller tests (Go modules cannot be downloaded in this sandbox, so only `cmp` of the schema copy was run), and `ndpiReader`, `tshark`, and Zeek runs of the hls job (the new tests need none of them).

### Shared page notes

- [Module Development](Module-Development.md), section "Fixtures and realism", needs two edits (MOD paths, so the coordinator applies them before merge).

  1. In the table of sections, the `temporal` row ends with "Arrival times, sizes, and packet gaps agree with their targets, with a KS test for a distribution and a tolerance for a constant". Replace that cell with "Arrival times, sizes, packet gaps, and the bytes per rung of a playback agree with their targets, with a KS test for a distribution and a tolerance for a constant".

  2. Replace the paragraph that starts "A stream needs at least `min_samples` gaps (30 by default) to be judged" (it ends with "so that you know they can fail.") with the text between the lines below, including its code block:

  ---
  A stream needs at least `min_samples` gaps (30 by default) to be judged, so record enough packets per stream, or give the target its own `min_samples` when a stream is short. A target of the client direction of an HTTP application can select the requests of one kind with `path`, a regular expression on the request target, so that a keepalive connection that alternates playlist and segment requests keeps the cadence of its segments apart from its playlist refreshes. `join_ms` makes the matching requests of the sessions that follow each other one stream, for a playback whose segments each use a connection, and `mean_tolerance_ms` also bounds the mean gap, so a cadence that drifts inside `tolerance_ms` fails. The bytes of an adaptive bit rate playback go in a `temporal.rungs` entry of the application. It gives the request path of a segment (its first group is the rung number), `duration_ms` of media per response, and `bits_per_second` of each rung. Every stream of requests with at least `min_samples` requests then needs the body of each response within `body_tolerance_bytes` of `rate * duration / 8` (188 by default), the share of each rung in the bytes within `share_tolerance` (5% by default), and the bit rate of the stream within `rate_tolerance` (5%). `walk: up` asks for rungs that rise through the whole ladder, and `streams` for the number of playbacks that the capture must hold (`realism/README.md`, Temporal distribution check, RLM-22). The labels read HTTP/1.1 `GET` requests whose responses have a `Content-Length`, and fail a session that they cannot follow, so a module that answers with chunked bodies cannot use `path` or `rungs`.

  ```yaml
  temporal:
    packets:
      applications:
        hls_vod: {direction: client, path: '/v\d+/seg\d+\.ts$', dist: constant, ms: 400, tolerance_ms: 40, within: 0.9, mean_tolerance_ms: 20, min_samples: 3}
    rungs:
      applications:
        hls_vod: {path: '/v(\d+)/seg\d+\.ts$', duration_ms: 400, bits_per_second: [160000, 320000, 640000], walk: up, streams: 3, min_samples: 4}
  ```

  A property that no label expresses still needs a small check script next to the fixture. Test the labels with a capture that you change on purpose, as `modules/sip_rtp/fixtures/test_packet_labels.py` does, so that you know they can fail.
  ---

- [Realism](Realism.md) is updated by this package (the temporal section, the report format 0.20.0, the metrics paragraph, the evidence). It still says in the MOD-24 notes that `hls_check.py` stays: that stays true until the MOD follow up below merges.
- [Modules](Modules.md): none. The README of the hls module describes `hls_check.py`, and the MOD follow up rewrites it.
- [Testing](Testing.md): the realism runner has one more test file, `realism/tests/test_http_rungs.py`, which reads the hls fixture and needs no external tool.
- [Home](Home.md) and [Implementation Plan](Implementation-Plan.md): none. The report format number appears only in the Realism page.
- Findings, for the MOD package that follows (it moves `hls_check.py` into labels and does the rest):
  - Add the `temporal` section above to `modules/hls/fixtures/expected.yaml` (it keeps its `arrivals` label for `hls_segment`), then delete `hls_check.py`, `test_hls_check.py`, `hls.playbacks.json`, and the `hls_check` calls in `capture.sh`, `mk/mod-hls.mk`, and the workflow, as MOD-24 did for rtp. Put `test_packet_labels.py`-style tests (the capture passes, a retimed copy fails) in its place. `realism/tests/test_http_rungs.py` has `retime` and a synthetic keepalive writer that it can reuse.
  - The ladder (`160000,320000,640000`), the segment duration (400 ms), and the session counts move from `params.json` into the labels, so they must match the scenario and `capture.sh` by hand. Keep a test that reads `params.json` (or the scenario) next to the labels if they must not drift apart.
  - Differences from the script: a stream with fewer than `min_samples` requests is counted as too short and not failed (the `streams` count catches a missing playback), the labels judge the playback count per application rather than for the whole capture, and a run of segment sessions joins at gaps of `join_ms` (800 ms is the two segment durations of the script).
  - The metric catalog has no series for the rung bytes. A CON package could add one (for example the largest relative share error per application) if the run report should carry it, which needs owner approval.
