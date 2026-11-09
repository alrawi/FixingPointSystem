# Handoff RLM-04 TLS decryption and JA4 checks

| Field | Value |
|---|---|
| Issue | #186 |
| Branch | `wp/RLM-04` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-9c41e7, lease until 2026-11-09T19:41:01Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the encryption check to the realism report. It decrypts a sample of TLS sessions with the exported keys and reports the decrypted share, and it computes the JA4 fingerprint of each sampled ClientHello and compares it with the JA4 of the configured client profile. A fixture with a mismatched profile fails, and a matching fixture passes.

## Plan

- [x] 1. JA4 from the FoxIO specification in `realism/ja4.py`, with unit tests against the worked example
- [x] 2. Labels `tls` section and `tls_profile` rule field, the tshark probe, and the `encryption` check
- [x] 3. Report schema 0.4.0, CLI `--keylog`, and the job summary row
- [x] 4. Tests with generated TLS captures: stub probes and real tshark, embedded and sidecar keys
- [ ] 5. README, wiki page, log entry, pull request, and CI

## Done and verified

1 to 4: `make lint-rlm test-rlm` passes locally with tshark 4.2.2 and openssl installed, so the real tshark tests ran and did not skip.

## Remaining

Open the pull request and wait for CI.

## Next action

Run `python3 tools/swarm/fps_swarm.py review 186 --pr <PR>` after CI is green.

## Blockers and open questions

none

## Decisions taken in this package

- The TRN-04 `ClientProfile` has no JA4 field and TRN-05 has not defined browser profiles yet, so the expected JA4 of a profile comes from the labels file (`tls.profiles.<name>.ja4`), which the controller or a module fixture writes from its client profile. A rule with `tls_profile` ties sessions to a profile.
- The runner computes JA4 itself from the ClientHello bytes (tshark prints them per frame, including TCP reassembly) and compares with the `tls.handshake.ja4` field of tshark 4.2. A disagreement fails the session, because one of them misreads the hello.
- A session counts as decrypted when tshark reads the Finished message of both sides. Both TLS versions encrypt Finished, and tshark shows it only with the right keys. The field `tls.app_data_proto` is unusable, because tshark sets it without keys too.
- The report entry leaves `metrics` empty, because the catalog has no metric for these shares. The same gap as issue #89 holds here.
- Generated fixtures use the Python `ssl` module over memory buffers and `openssl` for a self signed certificate, so the tests need no network, root, or dumpcap.

## Pitfalls

- The sandbox guard refuses `source`, heredocs, and `export` in the same command as git, so write files with the editor tools.
- `tls.app_data` exists with and without keys, so it proves nothing about decryption.
- Expected JA4 values of the real tests come from tshark, because the ClientHello of the Python `ssl` module depends on the OpenSSL version.

## Verify the current state

```bash
make lint-rlm test-rlm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T18:29:00Z | claude-9c41e7 | claim | none | Claimed #186 with 4 steps |
