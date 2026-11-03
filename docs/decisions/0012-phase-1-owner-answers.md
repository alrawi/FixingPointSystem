# D12. Answers to the Phase 1 owner questions

- Status: Accepted (owner, 2026-11-03)
- Date: 2026-11-03
- Deciders: repository owner

## Context

The Phase 1 packages raised 23 questions for the owner in their handoff files, log entries, and pull requests. The coordinator asked them one at a time, each with a recommended answer, and the owner answered every one. This record keeps the answers in one place, and the work packages that apply them cite the item numbers below.

## Decisions

| Item | Question | Answer | Applied by |
|---|---|---|---|
| 1 | Does the module license allow list keep AGPL-3.0? | Yes, because GPL-3.0 section 13 permits the combination | No change |
| 2 | Does `GET /metrics` need a token? | It stays open by default, and a setting can require a bearer token | CTL-19 |
| 3 | What happens on Linux older than 6.3, which lacks `IP_LOCAL_PORT_RANGE`? | The agent refuses to start with a clear error | AGT-06 |
| 4 | May the transport crate call `setsockopt` through `libc`? | Yes, inside one private `sys` module under `deny(unsafe_code)`, with a `SAFETY` comment and a test for every block | Coding Standards |
| 5 | Where does CI install Zeek from? | The Zeek repository on the openSUSE Build Service, with one pinned version | RLM-11 |
| 6 | Which host runs the nightly benchmark? | A self hosted runner on the owner's Linux server | QA-08 |
| 7 | Does the agent part of a port location pin the port? | Yes. A bare interface or `*/interface` spreads across agents | CON-09, CTL-18 |
| 8 | What happens when a pool has fewer hosts than workers? | Rates follow host shares, and the total rate stays as configured | CTL-18 |
| 9 | How do we treat seed 0? | The schema requires a seed of at least 1, and the controller chooses a random nonzero seed when none is given | CON-09, CTL-18 |
| 10 | Where does the bytes per session estimate come from? | An `expected_bytes` field in the manifest, corrected by calibration during the run | CON-09, CTL-21 |
| 11 | Does an agent keep redialing after an abort? | Yes, with a backoff that restarts at 200 ms with jitter, and a limit that defaults to five minutes, after which the agent exits | AGT-06 |
| 12 | Does a heartbeat timeout drop the plan when no traffic runs? | Yes, so a reconnected agent always starts clean | No change |
| 13 | How do API metric views map to catalog levels? | Every catalog metric names its view | CON-08 |
| 14 | Can a criterion select labels? | Yes, through an optional `where` filter | CON-09, CTL-20 |
| 15 | Which statistic suffixes may a histogram criterion use? | The fixed list p50, p90, p99, p999, min, max, mean, and count | CON-08 |
| 16 | Do the `x-fps-metric` annotations of CON-02 stay? | Yes | No change |
| 17 | Does the catalog get dedicated failure reasons? | Yes, `datagram_rejected`, `template_error`, and `extension_error` | CON-08, ENG-14 |
| 18 | What happens to `profiles` and `dist` in the specification example? | Phase 2 defines them, and the example is Phase 2 syntax until then | CON-10 |
| 19 | Does the API carry failure reasons? | Yes, `Run.reason` and the criterion fields `observed`, `passed`, and `reason` | CON-07, CTL-20 |
| 20 | Do we clean up the API contract? | Yes, `type: string` on enums, `GET /metrics` documented, and double precision metric values | CON-07 |
| 21 | Do metrics get a side label? | Yes, `client` or `server` on session and transport metrics | CON-08, ENG-14 |
| 22 | How does a test configuration name a DNS zone file? | Through generic module settings that each manifest describes with a JSON schema | CON-09, CTL-18, MOD-19 |
| 23 | Does the parser check keep `--parsers auto`? | Yes, and the report lists every skipped parser | RLM-11 |

## Consequences

The new packages CON-07, CON-08, CON-09, CTL-18, CTL-19, CTL-20, AGT-06, ENG-14, RLM-11, QA-08, and MOD-19 join Phase 1 as follow ups that Gate 1 does not wait for. CON-10 and CTL-21 join Phase 2. QA-08 needs the owner to register the self hosted runner.
