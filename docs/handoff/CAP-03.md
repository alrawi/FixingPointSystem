# Handoff CAP-03 Scrubbing and provenance

| Field | Value |
|---|---|
| Issue | #195 |
| Branch | `wp/CAP-03` |
| Pull request | #0 |
| Status | in progress |
| Progress | 60% (3 of 5 plan steps) |
| Current holder | claude-f353e8, lease until 2026-11-10T06:14:57Z |
| Last verified commit | 895712b |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Remove personal data and credentials from extracted sessions before scenario generation, and write the provenance record that module manifests require. `python3 -m capture scrub` reads a sessions document with payloads and writes a scrubbed one. `scrubbed: true` is set only from that scrub record. A seeded capture with known secrets yields output (sessions JSON and generated module) that contains none of them.

## Plan

- [x] 1. Claim commit, handoff
- [x] 2. Scrubber core (`capture/scrub.py`): addresses, protocol credentials, tokens, emails, names
- [x] 3. CLI `scrub`, provenance in the manifest, scenario reads the scrub record
- [x] 4. Seeded capture tests (raw, base64, percent decoded search), lint
- [ ] 5. Wiki page, README, log entry, pull request, CI

## Done and verified

- `python3 -m pytest -q capture`: 139 passed, 1 skipped (replay). `ruff format`, `ruff check`, `mypy --strict` clean.
- `capture/tests/test_scrub.py` seeds 44 secrets over HTTP, SMTP, IMAP, POP3, FTP, syslog and a binary datagram, and searches the sessions JSON, every payload and every generated YAML file in raw, percent decoded and Base64 decoded views.
- The six module fixture captures (608 sessions) scrub with `--hostnames` and then generate modules that validate with 0 errors.

## Remaining

Wiki page, README, log entry, pull request, CI.

## Next action

Update `docs/wiki/Capture.md` and `capture/README.md`, then open the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- The contract's provenance object has only `source`, `consent`, `scrubbed` and `notes`, with no extra properties, so the record (date, capture digest, rules with versions, byte change) goes into `notes` as one sentence, and the full record sits in the sessions document under `scrub`. The contract is unchanged.
- `scrubbed: true` comes only from an intact `scrub` record whose digest matches the document. `scenario --consent` and `--scrubbed` are gone and exit 1 with a pointer to `scrub`. `scrub --consent TEXT` is required.
- Replacements come from a seeded generator in order of discovery, not from a hash of the secret, so a replacement tells nothing about the original.
- Edits are computed against the original text and applied together; a structural edit beats a sweep edit on overlap. This keeps one value to one replacement and short values (below the sweep limits) still get replaced where a rule found them.
- Sessions schema 1.1 adds optional `source.sha256` and `scrub`.

## Pitfalls

- The worktree guard refuses heredocs and mixed commands. Write scripts to the scratchpad and run them.
- A server line such as `250 AUTH PLAIN LOGIN` looks like an AUTH command, so the SASL rule skips server style lines and mechanism lists.

## Verify the current state

```bash
make lint-cap test-cap PYTHON=python3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T04:45Z | claude-f353e8 | claim | 917f745 | claimed #195, branch wp/CAP-03 |
| 2026-11-10T05:25Z | claude-f353e8 | checkpoint | 895712b | scrubber, CLI, provenance, seeded tests |
