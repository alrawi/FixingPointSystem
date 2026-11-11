# Handoff MOD-15 HLS video module

| Field | Value |
|---|---|
| Issue | #182 |
| Branch | `wp/MOD-15` |
| Pull request | #0 |
| Status | in progress |
| Progress | 33% (2 of 6 plan steps) |
| Current holder | claude-fc5237, lease until 2026-11-11T05:13:47Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship `hls` scenarios that fetch a master playlist, a media playlist, and then segments at the segment cadence over a bitrate ladder. Deliverable: master and media playlists and synthetic segment bodies of the configured size. Acceptance: the temporal check accepts the segment fetch cadence, and the byte share matches the configured bitrate within 5%.

## Plan

- [x] 1. Write the native crate (playlists, MPEG-TS segments, sizes, pacing) and the manifest with unit and catalog tests
- [x] 2. Write the scenarios and the interpreter and golden transcript tests over an in memory network
- [ ] 3. Write `fixture_peer`, `capture.sh`, and record and score the fixture in the QA-01 harness
- [ ] 4. Write the cadence and byte share check with its tests
- [ ] 5. Write the README, the log entry with the shared page notes, and the Modules page section
- [ ] 6. Run the checks, open the PR, and wait for CI

## Done and verified

Steps 1 and 2: crate `fps-module-hls` (11 functions: master, media, segment, segment_size, content_length, playlist_uri, playlist_count, resolve, rendition, next, pace), manifest, scenarios `vod_ladder`, `vod_segments` (multi flow, one spawned connection per segment), and `live_refresh`, interpreter, catalog, and golden transcript tests. `cargo test -p fps-module-hls`: 30 unit tests, 2 catalog, 9 scenario, 2 transcript tests pass; `cargo clippy -p fps-module-hls --all-targets -D warnings` and `cargo fmt --check` are clean. The segment cadence is exact on the paused clock (every gap equals segment_ms).

## Remaining

Steps 3 to 6: fixture_peer, capture.sh, the fixture capture, the cadence and byte share check, README, log entry, Modules page section, PR.

## Next action

Step 3: write `native/examples/fixture_peer.rs` with a source port per connection, then `fixtures/capture.sh` and `fixtures/expected.yaml`.

## Blockers and open questions

none

## Decisions taken in this package

- Playlist and segment bodies come from `hls`, the HTTP framing from `http1` (manifest `requires`), so there is no duplicate HTTP code.
- The player is data driven: it reads the URIs from the playlists it received (`hls.playlist_uri`, `hls.resolve`), and the server reads the rung and segment number out of the request line.
- A flow has no condition, so ladder stages are explicit: `stages` times `stage_segments`, rung picked by `hls.rendition(ladder, 'ramp:K', n)`.
- `hls.pace` is due at once on its first call and then one segment apart on an absolute schedule (sip_rtp.pace idea), and the delay comes before the request, so request k is at t0 + k * segment_ms.

## Pitfalls

- Nested template calls render bytes, so a function that a scenario calls with a nested result must declare the parameter as bytes, not text (`hls.resolve`). The validator reports it.
- `target-MOD-15` is not covered by .gitignore. Add files by path, never with `git add -A`.

## Verify the current state

```bash
cargo test -p fps-module-hls
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:13Z | claude-fc5237 | claim | none | Claimed #182 |
| 2026-11-10T18:10Z | claude-fc5237 | checkpoint | see git log | Steps 1 and 2 done |
