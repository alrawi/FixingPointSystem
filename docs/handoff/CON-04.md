# Handoff CON-04 Scenario contract v0.2

| Field | Value |
|---|---|
| Issue | #20 |
| Branch | `wp/CON-04` |
| Pull request | #67 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-con04-d3df, lease until 2026-11-02T21:34:19Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Extend the scenario contract to v0.2. The template grammar gains module function calls with keyword arguments, such as `dns.query(name=vars.host, qtype='A')`, and the reference `last` for the bytes that the previous expect consumed. The contract states datagram rules for expect on UDP flows, and the validator checks module calls against a function list without running WASM.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Design the v0.2 grammar, `last`, and the UDP datagram rules, and bump the scenario schema
- [x] 2. Add the template function list to the module manifest schema and the example manifests
- [x] 3. Implement the template parser and module call checks in `tools/contracts/validate.py`
- [x] 4. Add validator tests for the grammar, `last`, module calls, and UDP expect
- [x] 5. Update the examples, `contracts/README.md`, and `contracts/CHANGELOG.md`
- [x] 6. Ingest into the wiki, append the Log entry, run the checks, and finish

## Done and verified

- Steps 1 to 4. The scenario schema carries `max_datagram` on expect, the manifest schema carries `functions`, and `validate.py` parses the v0.2 grammar, checks module calls against manifest catalogs, checks `last` by a definite precedence walk per side, and applies the UDP datagram rules. `python3 tools/contracts/validate.py` reports 0 errors and 0 warnings, `python3 -m pytest -q tools/contracts` passes 147 tests, and `python3 tools/contracts/check_plans.py` reports 0 errors.
- Step 5. `contracts/README.md` holds the v0.2 grammar, the module call, `last`, type, and function catalog sections, and the six datagram rules for UDP. `contracts/CHANGELOG.md` records 0.2.0. The voice was checked by a scan for em dashes, contractions, and colons outside code.
- Step 6. Wiki Home, Implementation Plan, and Log updated. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass, and `python3 -m pytest -q tools/contracts tools/plan tools/swarm` passes 199 tests.

## Remaining

Owner approval of the contract change and answers to the open questions below.

## Next action

Wait for the owner to review PR #67 and answer the open questions.

## Blockers and open questions

1. `engine_api` meaning. We bumped the example manifests to `">=0.2, <0.3"`, the example plan to engine API 0.2.0, and `ENGINE_API` in `check_plans.py` to 0.2.0, because the example scenarios now need the v0.2 grammar. `contracts/extension/README.md` ties `engine_api` ranges to minor versions of the extension package, which stays at 0.1.0. The owner decides whether `engine_api` names the scenario contract, the extension package, or one shared engine version. The OpenAPI example `engine_api: 0.1.0` lies outside the CON-04 paths and still reads 0.1.0.
2. Native modules. A manifest may declare `functions` without `extension`, which serves native first party modules under the proposed decision record 0010. The schema still requires `extension.wasm` whenever `extension` exists, as the CON-03 handoff notes.
3. Failure reasons. The metric catalog has no reason for an oversized or malformed datagram or for a template or extension failure, so the datagram rules use `pattern_mismatch`. CON-03 already names `extension_<kind>` labels that the catalog lacks. A dedicated reason such as `datagram_rejected` needs a catalog change outside the CON-04 paths.
4. The specification example uses `profiles.chrome` and `dist.pareto(...)`. v0.2 defines neither a `profiles` root nor a `dist` module, so those spellings stay invalid until a package defines them.

## Decisions taken in this package

- The manifest carries the function catalog under top level `functions`, mirroring the WIT `function-desc`. The validator reads it, and the engine rejects a module whose extension returns a different catalog. This answers the CON-03 question of how to check calls without running WASM.
- A module call renders `bytes`. Its result binds a `text` or `int` parameter only through a variable that `set` binds, which keeps static types honest without knowing what the extension returns.
- `int` widens to `float` and `text` to `bytes`. No other implicit conversion exists.
- Variables take the YAML type of their `vars` value, and set bound variables are `any`.
- `last` uses a definite precedence rule per side. A loop counts only with a positive integer literal count, and a branch only when every arm contains an expect.
- On UDP one expect consumes one whole datagram, and the stop rule becomes a check on it. `max_datagram` defaults to 65,507, the IPv4 UDP payload limit, and the engine reads into a per worker scratch buffer, so the default costs no memory per session.
- Calls nest at most 8 levels. Module names `vars`, `session`, `last`, `true`, and `false` are reserved.
- The v0.1 example scenarios `lookup`, `web_get`, and `web_browse` stay unchanged. The example plan embeds the first two, and spikes S1 (`program::tests::example_scenarios_compile`) and S4 (`test_enterprise_mix_reports_losses`) parse `web_browse` with the v0.1 grammar. The new scenarios `dns/resolve` and `http1/web_page` show the v0.2 features.

## Pitfalls

- Any change under `contracts/examples/modules/` changes the module tree digests in `contracts/examples/plans/http_dns_smoke.json`. Run `python3 tools/contracts/check_plans.py --print-digests` and copy the new digests, since the plan must stay in canonical form.
- `check_plans.py` calls `check_scenario` without a manifest, so module calls in plan sources stay unchecked.
- The first CI run failed in the `rust` and `contracts` jobs because spikes S1 and S4 read `http1/web_browse.yaml`, which we had rewritten with module calls. We restored that file and moved the example into `http1/web_page.yaml`. Run `cargo test -p fps-spike-engine` and `python3 -m pytest spikes/s4-trex-astf` after changing any example scenario.

## Verify the current state

```bash
python3 tools/contracts/validate.py               # contracts: 0 error(s), 0 warning(s)
python3 -m pytest -q tools/contracts               # 147 passed
cargo test -q --lib --manifest-path spikes/s1-engine/Cargo.toml   # 15 passed
python3 -m pytest -q spikes/s4-trex-astf          # passes
python3 tools/contracts/check_plans.py             # plans: 0 error(s)
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:05:07Z | claude-con04-d3df | claim | none | Fresh branch from main, card 6587192454 |
| 2026-11-02T20:13:44Z | claude-con04-d3df | checkpoint | see git log | Schemas, validator, examples, and tests done; README and CHANGELOG next |
| 2026-11-02T20:15:56Z | claude-con04-d3df | finish | see git log | README, CHANGELOG, and wiki done; all checks pass |
| 2026-11-02T20:18:36Z | claude-con04-d3df | checkpoint | see git log | CI failed in rust and contracts because spikes read web_browse; restored it and moved the example to web_page |
