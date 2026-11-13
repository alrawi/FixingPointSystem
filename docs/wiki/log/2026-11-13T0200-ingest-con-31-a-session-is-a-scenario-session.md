## [2026-11-13] ingest | CON-31 A session is a scenario session, and its other flows add bytes

A session is now one run of the entry flow of a scenario instance (`contracts/changes/CON-31.md`, approved by the owner on 2026-11-12). Every other flow of the instance, whether a `spawn` starts it or `after` orders it, adds its bytes to that session, and its failure fails the session once. No other flow counts as a session of its own. The change edits `contracts/README.md` (the new section Sessions and flows, and the words of the action table and datagram rules that meant a connection), the descriptions of `sessions.established`, `sessions.succeeded`, `sessions.duration`, `bytes.tx`, `bytes.rx`, and the label `reason` in the metric catalog, the controller copy of the catalog (which the coordinator authorized), and [Contracts](Contracts.md). The engine counts the new meaning with ENG-30, and CTL-49 resumes after it.

Decisions.

- No contract version moves. The catalog stays at 0.8.0 and the scenario contract at v0.6, because only text changes and no document shape, name, unit, label, or verdict does (the precedent is CON-16 and CON-23). The follow ups of CON-27 that a version bump needs, the pins in `tools/contracts/test_contracts.py` and `realism/tests/test_mix.py`, are therefore not needed, and `ui/src/api/schema.d.ts` embeds no catalog description.
- The descriptions of `sessions.attempted`, `sessions.concurrent`, `sessions.failed`, and `sessions.failed_ratio` keep their words, which fit the rule. The controller prints them as `# HELP` lines of `controller/internal/metrics/testdata/prometheus.txt`, which no authorized follow up names, so rewording them would have turned the controller tests red. A comment above `sessions.attempted` states the rule for the group instead. Rewording them later takes the golden file with it.
- A session spans its whole instance: it ends with the last flow, so `sessions.concurrent` stays up while an `after` flow runs and `sessions.duration` runs to the last action of any flow. The owner's decision named bytes and failure only, and this follows from "one session for each run of a scenario instance".
- A session fails once, with the reason of the first flow that failed. The flows that the failure aborts add no `aborted` count, so one failed data flow no longer shows as a failed data session plus an `aborted` control session.
- The session carries the labels of the entry flow. No first party scenario has flows of two modules, so the choice changes no series today.
- `session.id` is described as the number of one flow run (both sides share it, every flow and every run of a spawned flow has its own). That is what the engine does, and the draws do not change. The old row said "session sequence number", which the new definition would have made false.
- The two descriptions of `scenario.schema.json` that say session for a connection stay, because the controller embeds a byte copy of the schema. The next scenario contract change rewords them.

ENG-30 has its list in the change entry: the engine counters of `instance.rs` and `worker.rs`, the tests of `engine/fps-runtime/tests/{instances,datagrams,tls_sessions}.rs`, `engine/fps-agent/tests/first_party.rs`, and `engine/fps-agent/tests/controller_e2e.sh` with the numbers that they count today and after the change, and `Engine.md`. CTL-49 has `.github/workflows/qa-gate2.yml`, the header of `tests/gates/gate2/mix.yaml`, and the Gates page.

### Shared page notes

- [Module Development](Module-Development.md), the module documentation rule. This change tells a module author what a session holds, so the coordinator applies three edits. The contract holds as of this package, and the engine counts it with ENG-30. Edit 3 states what a run reports, which is true only once ENG-30 has merged, so apply edit 3 with ENG-30 or word it as the contract.

  Edit 1, section Flows (replaces the first paragraph):

  > Each flow has a client side and a server side that each run their action list in order. A flow fails when an action fails or times out (`contracts/README.md`, Flow program semantics). A session is one run of the first flow of the scenario, which is the entry flow, and every other flow of the scenario instance belongs to that session (see [Spawned flows, profiles, and TLS](#spawned-flows-profiles-and-tls)).

  Edit 1b, the row `session.id`, `session.flow` of the table of template names:

  > | `session.id`, `session.flow` | int, text | The number of the run of the flow that renders the template, shared by both sides and different for every flow and every run of a spawned flow, and the flow id |

  Edit 2, section Spawned flows, profiles, and TLS (add after the first paragraph):

  > **How spawned flows count.** A session is one run of the entry flow of a scenario instance. A flow that the entry flow spawns, and a flow that `after` orders, belongs to that session and is never a session of its own. It adds its payload bytes to the bytes of the session, every time it runs, and its failure fails the session. An `ftp` scenario with three transfers is one session that holds one control connection and three data connections, and a `sip_rtp` call is one session that holds the SIP dialog and the RTP media stream. A capture still shows every flow as its own connection. The session ends when its last flow ends, and a session fails with the reason of the first flow that failed.

  Edit 3, section `expected_bytes`, in four places.

  The first paragraph, after "with the default parameters of the scenario", add: "A session is one run of the entry flow, and every other flow of the instance, spawned or ordered with `after`, belongs to it, so the estimate counts the bytes of all of them (see [Spawned flows, profiles, and TLS](#spawned-flows-profiles-and-tls))."

  Step 2 of How to measure it, replace the first two sentences with: "Add the payload bytes of every connection that the session opens, client to server and server to client, for every flow of the instance. That sum includes the connections of the flows that the entry flow spawns, once for every run of a spawned flow (the three data connections of an FTP session with three transfers, the RTP media of `sip_rtp`), and the connections of the flows that `after` orders. `modules/hls/tests/scenarios.rs` does this with `c2s().len() + s2c().len()` over `net.conns()`."

  After the paragraph "The estimate is a starting point", add: "**What the run counts.** The run metrics count one session for each start of the entry flow, and `bytes.tx` and `bytes.rx` hold the bytes of all its flows under the labels of the entry flow. The controller therefore measures bytes per session in the same unit as your estimate, and the calibration moves the rate by the real figure. A scenario whose spawned flows once counted as sessions of their own needs no new estimate, because `expected_bytes` always meant the bytes of the whole session."

  The last paragraph of the section stays as it is.

- [Engine](Engine.md): ENG-30 rewrites the Series row of the table of Multi flow scenario instances, the paragraph after it, and the row `tests/instances.rs` same data port (32 succeeded sessions becomes 8). It is in the package of ENG-30, not here.
- [Gates](Gates.md): CTL-49 rewrites the paragraph on the byte basis and the paragraph on the earlier estimates, which say the run counts the data connection and the media stream as sessions, when `FPS_GATE2_BASIS` goes.
- [Testing](Testing.md): none. This package adds no test, and the tests of the consumers change with ENG-30.
- Catalog version: it stays at 0.8.0, so [Home](Home.md) and the Contracts overview keep their numbers.
