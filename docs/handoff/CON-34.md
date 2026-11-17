# Handoff CON-34 DHCP configuration of a host pool

| Field | Value |
|---|---|
| Issue | #598 |
| Branch | `wp/CON-34` |
| Pull request | #603 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-b16e88, lease until 2026-11-17T03:39:51Z |
| Last verified commit | 8cb7802 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

HST-04 obtains host addresses through DHCP when a host pool configures it. Add the optional `dhcp4` object to a client host pool (test configuration v0.3 to v0.4), the `Dhcp4` message on `HostPoolSlice` (agent protocol 0.2.5 to 0.2.6), the example, the negative checker cases, the plan checker change, the validator gateway fix, and `contracts/changes/CON-34.md`. The vetting agent (Fable 5.2, D13, 2026-11-17) specified it exactly.

## Plan

- [x] 1. Schema v0.4, proto 0.2.6, README and wiki version lines
- [x] 2. Example `dhcp_clients.yaml`, plan checker, validator gateway fix
- [x] 3. Tests (negative cases, dual stack regression)
- [x] 4. `contracts/changes/CON-34.md` with the consumers and the failing tests, log entry, Contracts page
- [x] 5. Pull request, CI, review

## Done and verified

- `python3 tools/contracts/validate.py`: 0 errors. `python3 tools/contracts/check_plans.py`: 0 errors.
- `python3 -m pytest tools/contracts`: 371 passed, 1 skipped, 1 failed. The failure is `test_controller_copies_are_byte_identical` for the test configuration schema, which the consumer commit fixes by copying the file (outside this package's paths).
- `grpc_tools.protoc` compiles the proto file. `buf` is not installed here, so CI checks `buf lint` and `buf breaking`.
- `python3 tools/plan/plan.py check`, `render --check`, `lint`: pass.
- The two dual stack tests fail on the old `validate.py` and pass on the new one.

## Remaining

The consumer commit (coordinator): schema copy, `regen-agentpb`, compiler, store fixtures, agent mirrors, UI. `contracts/changes/CON-34.md` lists each step and each test that fails until then.

## Next action

The coordinator lands the consumer commit on `wp/CON-34` (steps in `contracts/changes/CON-34.md`), then CI should go green. CI of 8cb7802 is red only in jobs that the consumers explain: store example and schema copy tests, `agentpb/gen` currency, `lint-ui` (`schema.d.ts`), the Rust jobs that build the agent, and the `contracts` schema copy test.

## Blockers and open questions

none

## Decisions taken in this package

- The plan checker also rejects a `dhcp4` slice that is not a client slice. The specification names the client rule for the schema only. It mirrors the schema, so a plan that no configuration could compile fails early.
- A gateway of a family with no pool prefix is an error that names the family.
- `vlan` and `mac_base` stay allowed on a DHCP pool. The specification did not restrict them.

## Pitfalls

- The README says "Host i uses the MAC address", not "Host index i", in plan rule 5; a text replace on the wrong wording silently matches nothing.
- With a gateway in the pool, a pool of 254 hosts in a /24 exceeds the usable count, so a dual stack test needs a smaller `count`.
- `engine/fps-agent/src/convert.rs` builds `pb::HostPoolSlice` without `..Default::default()`, so the crate stops compiling when field 13 exists.

## Verify the current state

```bash
python3 tools/contracts/validate.py          # contracts: 0 error(s)
python3 -m pytest -q tools/contracts         # 1 expected failure: the embedded schema copy
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17T03:09Z | claude-b16e88 | claim | none | Card 6935757529, steps 1 to 4 |
| 2026-11-17T03:25Z | claude-b16e88 | checkpoint | 8cb7802 | Contracts, example, checker, validator fix, tests, change entry pushed, PR #603 |
| 2026-11-17T03:45Z | claude-b16e88 | review | see PR | CI red as predicted, only in consumer jobs; consumer commit is the coordinator's |
