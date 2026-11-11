# Handoff MOD-17 LDAP module

| Field | Value |
|---|---|
| Issue | #184 |
| Branch | `wp/MOD-17` |
| Pull request | #449 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-e41a7f, lease until 2026-11-11T05:14:04Z |
| Last verified commit | see the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship ldap scenarios for bind, search, and unbind with BER encoding functions. The native crate `fps-module-ldap` holds a BER codec written in this crate, the LDAPv3 message encoders and readers, and a server side that answers binds and searches from a directory fixture. nDPI must classify at least 99% of the fixture sessions as LDAP.

## Plan

- [x] 1. Claim commit, BER codec and filter parser with unit tests
- [x] 2. LDAP messages, directory model, search evaluation, the template functions and the catalog
- [x] 3. Manifest, scenarios, and the interpreter, catalog, and transcript tests
- [x] 4. fixture_peer, capture.sh, expected.yaml, and a capture in a namespace harness
- [x] 5. tshark test of every encoded message, README, Modules page, log entry with shared page notes
- [x] 6. Run the checks, open the PR, wait for CI, hand over for review

## Done and verified

- `cargo test -p fps-module-ldap`: 48 unit tests, catalog, scenarios (10), transcript (3), and tshark (3) pass.
- `cargo clippy -p fps-module-ldap --all-targets -- -D warnings` and `cargo fmt`: clean.
- `tools/contracts/validate.py` over a directory of links (the lint-mod-ldap recipe): 0 errors.
- `plan.py check`, `render --check`, `lint`: ok.
- `modules/ldap/fixtures/capture.sh` in the namespace harness: 24 sessions, nDPI 4.2.0 labels 24 of 24 LDAP (ratio 1.0), CI nDPI 4.14.0 labels 24 of 24 LDAP on a fresh capture, tshark 4.2.2 reports no parser error. The report is committed as `ldap.report.json`.

## Remaining

- Review.

## Next action

Review and merge. The coordinator adds mk/mod-ldap.mk, the workflow, and the registry entry from the log entry.

## Blockers and open questions

none

## Decisions taken in this package

- The BER codec is written in the crate, so no dependency is added.
- Every scenario starts with an anonymous bind, because nDPI 4.2.0 labels a session MQTT when its first bind carries a name.
- The coordinator files `mk/mod-ldap.mk` and `.github/workflows/mod-ldap.yml` and the agent registry entry are in the Shared page notes of the log entry.

## Pitfalls

- A nested call has the type bytes, so a text parameter needs a `set` first.
- The manifest summary of a function may not exceed 200 characters.
- Set CARGO_TARGET_DIR with an absolute path of the worktree root, or a target directory lands under modules/.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-MOD-17 cargo test -p fps-module-ldap
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11T03:46Z | claude-e41a7f | claim | pending | Claimed issue #184 with a 90 minute lease, branch from origin/main |
| 2026-11-11T05:00Z | claude-e41a7f | checkpoint | see git log | Crate, scenarios, tests, fixture, docs complete locally |
| 2026-11-11T05:35Z | claude-e41a7f | review | see git log | PR #449 opened, all CI jobs green including module (ldap) with nDPI 4.14.0 |
