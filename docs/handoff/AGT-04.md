# Handoff AGT-04 Signed module verification

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #161 |
| Branch | `wp/AGT-04` |
| Pull request | #319 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-d88d9e, lease until 2026-11-09T19:41:39Z |
| Last verified commit | f0c44f0 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent verifies each module tree digest and the plan signature (ModuleRef.signature, signature_key_id) against a trust store before it loads a module, and refuses unsigned, altered, unknown-key, revoked or retired modules unless developer mode is on.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Trust store parser and verifier in src/trust.rs with the CTL-11 golden vectors
- [x] 2. Plan mirror and conversion carry the signature; policy in modules::load
- [x] 3. Flags --module-trust-store and --dev-modules for run and connect
- [x] 4. Tests, Agent.md, log entry, PR

## Done and verified

`cargo test -p fps-agent` passes (trust unit tests, tests/signing.rs with 10 tests, and every older suite). `cargo fmt` clean.

## Remaining

Nothing. CI is green on f0c44f0.

## Next action

The coordinator reviews and merges PR #319.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- Signature check uses `ring` (already in Cargo.lock via rustls), no new package.
- Tree digest stays in `modules::tree_digest` (directory form of rule 1); golden vectors in tests/signing.rs.
- No trust store: signatures not required, one warning per plan (like the controller).
- `--dev-modules` waives only the signature policy, not the digest check against the plan.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
