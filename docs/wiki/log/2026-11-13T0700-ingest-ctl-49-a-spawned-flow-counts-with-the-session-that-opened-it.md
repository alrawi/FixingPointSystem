## [2026-11-13] ingest | CTL-49 A spawned flow counts with the session that opened it

The controller needed no counting change. CON-31 made a session one run of the entry flow, and ENG-30 made the engine report it that way, with the bytes of the spawned and `after` flows under the labels of the entry flow. `clientTotals` in `controller/internal/runs/calibration.go` sums `sessions.succeeded` and the client side bytes of a scenario, and the run report reads the same series, so both now see one session with the bytes of both flows for `ftp/retr_epsv` and `sip_rtp/call_pcmu`. The earlier worker found the split in the engine and blocked the package until CON-31 and ENG-30 merged.

Changes.

- `.github/workflows/qa-gate2.yml`: `FPS_GATE2_BASIS` and its comment are gone, and the two mix steps no longer pass it. `mix.sh` and `prove-failure.sh` default to the bytes basis, so the suite runs on it with no waiver.
- `tests/gates/gate2/mix.yaml`: the header comment no longer says the run report counts the FTP data connection and the media stream as sessions.
- `docs/wiki/Gates.md`: the byte basis waiver paragraph, the paragraph on the earlier estimates, and the sentence before the waiver list say the waiver ended.
- `docs/wiki/Implementation-Plan.md`: the Gate 2 line no longer names CTL-49 as the remaining waiver.

Gate 2 on the bytes basis (qa-gate2 run 43379299570, mix job, commit fbbc8c0): verdict PASS, basis bytes, waived none. The largest difference between a byte share and its weight was 1.354 points by the run metrics (http) and 1.284 points by nDPI (http), against 2 points. `ftp/retr_epsv` reported 1,900 sessions with 7,658 bytes each (mix.yaml 7,765), and `sip_rtp/call_pcmu` 254 sessions with 37,230 bytes each (mix.yaml 37,230). Conformance was 100%, and the parsers reported nothing.

### Shared page notes

- [Gates](Gates.md): the sentence in Limits ("it runs on the session basis until the controller catalog carries `expected_bytes`. Gate 2 does not hold in full before both waivers end") is stale, because neither waiver is set. It was outside the authorized lines.
- [Testing](Testing.md), [Home](Home.md): none.
- `tests/gates/gate2/README.md` (QA-05 paths): the row `FPS_GATE2_BASIS` and the paragraph on the second waiver describe a waiver that the workflow no longer sets. The variable still works for a local run.
