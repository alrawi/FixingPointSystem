# Handoff MOD-13 SMB2 module

| Field | Value |
|---|---|
| Issue | #180 |
| Branch | `wp/MOD-13` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-9c41d7, lease until 2026-11-10T19:08:16Z |
| Last verified commit | 484a2e3 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship smb2 scenarios for file browsing, download, and upload, with client and server sides and configurable file sizes. nDPI must classify at least 99% of the fixture sessions as SMB. MOD-12 built the native functions crate `fps-module-smb2`; this package adds the manifest, the scenarios, the fixture, and `capture.sh`.

## Plan

- [x] 1. `manifest.yaml` with the catalog of the crate and the parameter schema, and the three scenarios `browse`, `download`, `upload`.
- [x] 2. Tests that run both sides of each scenario through the interpreter, and the golden transcripts.
- [x] 3. The `fixture_peer` example, `capture.sh`, and `expected.yaml`.
- [x] 4. Record the fixture in a network namespace, score it with nDPI, check how nDPI treats the 4 octet direct TCP header.
- [ ] 5. README, log entry, wiki page notes, checks, pull request, CI.

## Done and verified

- `cargo test -p fps-module-smb2`: every test binary passes, 12 tests in `scenarios.rs`, the catalog test compares the manifest, `fixture.rs` scores the committed capture.
- `modules/smb2/fixtures/capture.sh` ran in the namespace harness: 18 sessions, nDPI 4.2.0 classifies 18 of 18 as `NetBIOS.SMBv23` (ratio 1.0), tshark 4.2.2 reports no parser error.
- `tools/contracts/validate.py --modules ... --tests --no-openapi` reports 0 errors, `cargo fmt --check`, `cargo clippy -D warnings`, `shellcheck`, `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Open the pull request, wait for CI, and set the handoff to ready for review.

## Next action

Open the pull request against main.

## Blockers and open questions

none

## Decisions taken in this package

- The tests stay in `native/tests/`, where MOD-12 put its tests, and not in `modules/smb2/tests/`.
- The length of the last read or write is a variable (`tail_size`, `upload_tail`), because a flow has no arithmetic and a nested call gives octets, which `smb2.add` refuses.
- `expected.yaml` accepts `SMBv23` and `SMB`, because nDPI 4.2.0 reports `NetBIOS.SMBv23` and the name may differ in 4.14.
- The thirteen function summaries over 200 characters (the limit of the manifest schema) were shortened in `native/src/catalog.rs`, and the manifest was regenerated.

## Pitfalls

- nDPI 4.2.0 reports sessions on a port other than 445 as `Unknown`, so the scenarios keep port 445.
- The manifest validator wants every `set` value to be a string, so a counter starts as `"0"`, and summaries are at most 200 characters.
- Hooks of the sandbox refuse a command that mixes a heredoc with `git`, `.github`, or `export`; write scripts with the editor tools.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-module-smb2
python3 -m realism score --pcap modules/smb2/fixtures/smb2.pcapng --labels modules/smb2/fixtures/expected.yaml --parsers tshark --output /tmp/smb2.json
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:25Z | claude-9c41d7 | claim | none | Claimed #180, branch wp/MOD-13 from origin/main |
| 2026-11-10T17:45Z | claude-9c41d7 | checkpoint | 484a2e3 | Manifest, three scenarios, tests, golden transcripts |
| 2026-11-10T18:15Z | claude-9c41d7 | checkpoint | pending | fixture_peer, capture.sh, fixture, README, log entry |
