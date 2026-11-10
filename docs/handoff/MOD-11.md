# Handoff MOD-11 SSH module

| Field | Value |
|---|---|
| Issue | #178 |
| Branch | `wp/MOD-11` |
| Pull request | none yet |
| Status | in progress |
| Progress | 40% (2 of 6 plan steps) |
| Current holder | claude-3cddb6 (the lease on the status card renews at every checkpoint) |
| Last verified commit | see git log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship ssh scenarios with real key exchange through a native extension backed by a GPL-3.0 compatible SSH library. Deliverable: interactive shell and file copy shaped scenarios with configurable session length. Acceptance: the server and client complete key exchange in every fixture session, and nDPI classifies them as SSH.

## Plan

- [x] 1. Choose the SSH library, check its license, and write the crate skeleton, manifest, and first scenario
- [x] 2. Implement the native functions and the fixture peer with real key exchange
- [ ] 3. Write the scenarios (shell, file copy) and the interpreter, catalog, and transcript tests
- [ ] 4. Write capture.sh, expected.yaml, and the Zeek check
- [ ] 5. Write the README and the log entry with the shared page notes
- [ ] 6. Run the checks, open the PR, and wait for CI

## Done and verified

- Step 1 and 2: crate `modules/ssh/native` (package `fps-module-ssh`) with a real SSH 2.0 transport: `curve25519-sha256` key exchange, `ssh-ed25519` host key signed and verified through the RustCrypto `ssh-key` crate, `chacha20-poly1305@openssh.com` packets. Verified with `cargo test -p fps-module-ssh --lib`: 43 tests pass (round trip, tamper, replay, forged signature, swapped host key, hostile bytes).
- Step 3 (part): four scenarios (`interactive_shell`, `remote_command`, `scp_upload`, `scp_download`) and the manifest. `tools/contracts/validate.py` accepts them; the only error is the missing fixture capture, which step 4 adds.

## Remaining

- The interpreter tests (`modules/ssh/tests`: catalog, scenarios, transcript), the golden transcripts, and `native/examples/fixture_peer.rs` are not written yet.
- `fixtures/capture.sh`, `fixtures/expected.yaml`, the committed capture, and the Zeek check are not written yet.
- README, the log entry with the shared page notes (`mod-ssh.yml`, `mk/mod-ssh.mk`, Modules page rows), and the Modules.md component page section.
- Interop check of the transport against an independent SSH implementation (paramiko) to confirm that the cipher and key derivation match the RFCs and OpenSSH.

## Next action

Write modules/ssh/tests (catalog, scenarios, transcript) and the fixture_peer example.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:46Z | claude-3cddb6 | claim | none | Claimed issue 178, card 6767703439 |
| 2026-11-10T13:45Z | claude-3cddb6 | checkpoint | see git log | Crate, manifest, and four scenarios written; 43 unit tests pass |
