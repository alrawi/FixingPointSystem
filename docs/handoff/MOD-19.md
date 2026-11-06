# Handoff MOD-19 DNS zone file setting

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #142 |
| Branch | `wp/MOD-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-571e77, lease until 2026-11-06T17:29:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A test configuration names a zone file through `modules.dns.settings.zone_file` (decision D12 item 22), and the dns server answers from it. Deliver the `zone_file` setting in the manifest settings schema, its use in the Rust server side, and a fixture that answers from a configured zone.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim commit
- [x] 2. Manifest settings schema and the Settings type in the crate, with tests
- [x] 3. fixture_peer and capture.sh pass the setting, README and wiki page
- [ ] 4. Checks, PR, CI

## Done and verified

- Steps 2 and 3: `cargo test -p fps-module-dns` passes (40 unit tests, catalog, scenarios, 4 transcript tests), `cargo clippy -p fps-module-dns --all-targets -- -D warnings` and `cargo fmt --check` are clean, `make lint-mod-dns` reports 0 errors, `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Open the PR and wait for CI. The namespace harness and realism job cannot run in this sandbox (no ip, tshark, ndpiReader), so the realism job on the PR is the evidence for that acceptance item. The setting does not reach an agent through the controller until `ModuleRef` gains a settings field.

## Next action

Open the pull request, then poll CI.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- Settings load through `Dns::from_settings(&serde_json::Value, base_dir)` with the module directory as base, so the path in `dns_zone.yaml` (`zones/example.test.zone`) resolves.
- A configured zone must hold an SOA record, because the SOA owner is the zone origin.
- The default stays the bundled zone. The contract example manifest says the server answers with the scenario address without a zone, but the real module always served its shipped zone, and the task says to keep the default.
- `fixture_peer --settings JSON` is the module level channel. `--zone FILE` stays as a shorthand. `capture.sh` passes `FPS_DNS_SETTINGS` or the default JSON to both peers.

## Pitfalls

The sandbox tool refuses compound shell commands that mention git or run source; use plain separate commands. Adding serde_json changed the root Cargo.lock by one line.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T16:00Z | claude-571e77 | claim | | Claimed, branch from origin/main 3243a04 |
