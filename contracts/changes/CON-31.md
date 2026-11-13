# CON-31 A session is a scenario session, and its other flows add bytes

Contract. The flow program semantics of `contracts/README.md` (the sentence "Each flow runs one session", the new section Sessions and flows, and the words of the action table and the datagram rules that meant a connection), the descriptions of the `sessions.*` and `bytes.*` series and of the label `reason` in the metric catalog `contracts/metrics/catalog.yaml`, and the row `session.id` of the template names. CTL-49 found that the engine makes every flow of a scenario instance a session of its own (`engine/fps-runtime/src/instance.rs`, `worker.rs`), as the old sentence and the old catalog descriptions say. The FTP data connection and the RTP media stream then count as sessions beside their control flow and their dialog, so the run metrics, the calibration of the controller, and the Gate 2 byte basis see half the bytes of an ftp or sip_rtp session, and a mix by session shares carries the wrong shares. The owner approved the change as a contract change on 2026-11-12, and decided the open question of the package in a comment on issue #537: flows that `after` orders and that nothing spawns fold into the session as well. The change is a change of meaning and adds no field.

## Versions

No contract version moves.

- The metric catalog stays at 0.8.0. No metric, unit, level, view, label, or label value changes, and only descriptions and comments change, as only text changed when the agent protocol stayed at 0.2.3 (CON-16) and OpenAPI stayed at 0.10.0 (CON-23). The catalog version names the shape of the document, and a reader that parses the catalog sees the same names, types, units, and labels. The numbers that the series report change when ENG-30 changes the engine, and the version cannot carry that change, because the engine and the controller ship together and the series have no version of their own. This entry, the sentence in `contracts/README.md`, and the engine change of ENG-30 carry it. No pin of the catalog version (`tools/contracts/test_contracts.py`, `realism/tests/test_mix.py`) moves.
- The scenario contract stays at v0.6. The schema, the grammar, the validator, and the verdict on every scenario stay as they are. The rule gives `flows[0]` the name entry flow, which the agent and the runtime already use (`assemble::check_flows`, `RuntimeConfig`).
- OpenAPI (0.12.0), the agent protocol (0.2.4), the test configuration (v0.3), the extension interface, and the module manifest schema stay as they are.

## The rule

A session is one run of the entry flow of a scenario instance, and the entry flow is the first flow of the scenario. Every other flow of the instance adds its bytes to that session, and its failure fails it. No other flow counts as a session of its own. `contracts/README.md`, Sessions and flows, states the six rules. Their effect on the series is as follows.

| Series | New meaning |
|---|---|
| `sessions.attempted`, `sessions.rate` | One per start of the entry flow of an instance |
| `sessions.established` | The first `connect` or `accept` of the entry flow completed |
| `sessions.succeeded` | Every flow of the instance, spawned or after, finished every action |
| `sessions.failed` | Any flow of the instance failed or timed out, counted once, with the `reason` of the first flow that failed. The flows that the failure cuts short add no `aborted` count of their own |
| `sessions.concurrent` | Instances that have started and not yet ended, so a session stays open while an `after` flow or a spawned flow outlives the entry flow |
| `sessions.duration` | From the start of the entry flow to the end of the last action of any flow of the instance |
| `bytes.tx`, `bytes.rx`, `throughput.bps` | Every flow of the instance counts, under the labels of the entry flow (`application` is its module, `scenario` its scenario) |
| `sessions.response_time`, `transactions.*`, `actions.*`, `tcp.*`, `tls.*`, `port.*` | Unchanged. They count transactions, actions, connections, and packets, which a flow still owns |

The rule for `after` is the owner's decision of 2026-11-12. A flow that `after` orders belongs to the session of its instance as a spawned flow does. A flow that no `after` orders and nothing spawns, and that is not the first flow, starts with the entry flow (the interpreter starts every flow that is not spawned and whose `after` list is done), and it belongs to the session too. A spawned flow that runs several times (one data connection for each transfer, one segment flow for each segment) adds the bytes of every run.

A client session and a server session follow the same rule. Each side counts the sessions that its own workers run, and the label `side` tells them apart.

### What does not change

- A scenario with one flow has one session per run, and every count of such a scenario stays as it is.
- A capture still shows each flow as its own connection, with its own 5-tuple, and the realism pipeline and the classifier still call a connection a session. The capture counters of the engine (`sessions_written` of `fps-capture`) count connections, so they keep counting each flow.
- The seeded draws, `session.id`, and the session ids of the flow connections stay as they are. Only the words of the contract change: `session.id` was "the session sequence number", and the engine gives every flow run its own number, so the row now says that (see below).
- The validator reports the same findings. It does not reject a spawned entry flow, and the agent still refuses one at prepare.

## Text changes

`contracts/README.md`

- "Each flow runs one session" becomes the section Sessions and flows, with the six rules, the meaning of the word connection, and the series that keep their meaning.
- The action table and the datagram rules said session where they meant what a flow side opens: `connect` opens a connection, `accept` accepts one connection, `tls_upgrade` wraps the running connection, and datagram rule 6 says a connection receives only the datagrams of its peer. The rules that say a datagram fails the session stay, because a failed flow fails its session.
- The row `session.id` of the template names says what the engine does: a number for the run of one flow, shared by its two sides, unique within the run, and different for each flow and each run of a spawned flow.

`contracts/metrics/catalog.yaml`

- A comment above `sessions.attempted` states the rule for the whole group.
- The descriptions of `sessions.established`, `sessions.succeeded`, `sessions.duration`, `bytes.tx`, and `bytes.rx`, and the label `reason`, say what the table above says.
- The descriptions of `sessions.attempted`, `sessions.concurrent`, `sessions.failed`, and `sessions.failed_ratio` keep their words, which fit the rule as they stand ("Sessions that the engine started", "Sessions open at the sample instant", "Sessions that ended with a failed or timed out action, by failure reason", and the ratio). The controller prints them as the `# HELP` lines of its Prometheus golden file (`controller/internal/metrics/testdata/prometheus.txt`), a file that no follow up of the authorization names, and the rule of the comment is what they mean. A later change that rewords them takes that file with it.
- `sessions.response_time` keeps its description, which already speaks of a transaction.

`contracts/schemas/scenario.schema.json` keeps two descriptions that say session where they mean a connection (`max_datagram`, which is fine, and `tls_upgrade`, "the transport of the running session"). The controller embeds a byte copy of the schema, and a wording change would need a refresh of the copy for no change of behavior, so the next scenario contract change rewords it.

## Compatibility

The text takes effect in the engine with ENG-30. Until then the engine counts every flow, and a reader of the series sees the old numbers. When ENG-30 merges, the series of a multi flow scenario fall (one session for each instance in place of one for each flow side), and the bytes per session that the calibration measures double, which is the true figure.

| Scenario | Sessions of one instance before | After |
|---|---|---|
| `ftp/retr_epsv`, `ftp/stor_pasv`, `ftp/list_pasv` | 2 (control, one data connection) | 1 |
| `ftp/batch_download`, `ftp/mixed_session` | 1 plus one for each transfer (4 for `mixed_session`) | 1 |
| `sip_rtp/call_pcmu`, `call_pcma`, `call_g729`, `call_opus` | 2 (dialog, media) | 1 |
| `hls/vod_segments` | 1 plus one for each segment (7 with `segments: 6`) | 1 |
| `dns/tcp_fallback` | 2 (`udp`, then `tcp` after it) | 1 |

A test configuration with the `sessions` basis keeps its shares, because a share is a share of starts. The shares of a configuration with the `bytes` basis and the session rate of a `throughput_bps` objective use `expected_bytes`, which already counts the bytes of the whole session (the guide says so, and the first party estimates of MOD-25 are the bytes of every connection of one session), so they were the figures that the engine counted wrongly and they stay as they are.

## Consumers

The engine and the tests below count the sessions of a multi flow scenario or write down the old meaning. ENG-30 changes the engine counters and every test of the first two groups. I searched `engine/`, `controller/`, `realism/`, `tests/`, `modules/*/native/tests/`, and the rest of the tree (`cli/`, `sdk/`, `ui/`, `capture/`, `bench/`, `tools/`, `.github/`) for the first party multi flow scenarios (`ftp/*`, `sip_rtp/call_*`, `hls/vod_segments`, `dns/tcp_fallback`), for the inline scenarios with `spawn` or `after`, and for every assertion on the `sessions.*` series.

### Engine code (ENG-30)

| File | What changes |
|---|---|
| `engine/fps-runtime/src/instance.rs` | `started`, `ended_with` (established, succeeded, failed, duration, bytes), and the `SideRecord` loop count for the entry flow only. Every other flow side adds its bytes to the series of the entry flow, and its failure fails the instance with its own reason once. The sides that the failure aborts add no `aborted` count |
| `engine/fps-runtime/src/worker.rs` | `Worker::new` and `metrics_of` share one `SessionMetrics` per module of the flows. The series of the instance carry the labels of the entry flow, so a flow of another module adds its bytes there too (no first party scenario has one) |
| `docs/wiki/Engine.md` | The Series row of the table of Multi flow scenario instances ("Every flow side counts as a session"), the paragraph that follows it (each side counts under its own reason, and `sessions.concurrent` falls when the instance returns), and the row `tests/instances.rs` same data port ("32 succeeded sessions") |

### Tests that count sessions of a multi flow scenario (ENG-30)

| Test | Counts today | After the change |
|---|---|---|
| `engine/fps-runtime/tests/instances.rs`, `mixed_session_runs_with_one_data_connection_per_transfer` | `INSTANCES * FLOW_SESSIONS` = 32 for `sessions.attempted`, `established`, and `succeeded` of each side, and for the series labeled `application` ftp and `scenario` mixed_session | `INSTANCES` = 8. The capture lines of the test (`stats.sessions_written` = 32 and the four distinct data ports of one host) count connections and keep `FLOW_SESSIONS`, so the test needs two constants |
| the same file, `a_data_connect_before_the_listener_is_retried` | attempted 2, succeeded 2, failed 0 on each side | 1, 1, 0 |
| the same file, `a_flow_with_after_runs_when_its_predecessor_is_done` | attempted 4, succeeded 4, failed 0 on each side (two instances of two flows) | 2, 2, 0. This is the test of the owner's `after` decision |
| the same file, `a_failing_flow_fails_the_instance_and_aborts_the_others` | Client: attempted 2, succeeded 0, failed `expect_timeout` 1 and `aborted` 1. Server: attempted 2, failed 2 | Client: attempted 1, succeeded 0, failed `expect_timeout` 1 and no `aborted`. Server: attempted 1, failed 1 with the reason of its first failing flow. The test still checks that `sessions.concurrent` returns to 0 |
| the same file, `every_flow_of_an_instance_calls_its_extensions` | attempted 4, succeeded 4, failed 0 on each side | 2, 2, 0 |
| the same file, `concurrent_instances_that_draw_the_same_data_port_all_succeed` | `INSTANCES * FLOW_SESSIONS` succeeded sessions on each side | `INSTANCES`. The capture lines keep counting connections |
| `engine/fps-runtime/tests/datagrams.rs`, `run_calls` (`a_sip_call_with_rtp_runs_through_the_instance_runner`, `concurrent_calls_that_draw_the_same_rtp_port_all_succeed`) | `calls * FLOW_SESSIONS` (dialog and media) for attempted and succeeded on each side | `calls`. `stats.sessions_written` and the count of 5-tuples keep `calls * FLOW_SESSIONS`, so this file needs two constants too |
| `engine/fps-runtime/tests/tls_sessions.rs`, `a_spawned_flow_upgrades_to_tls` | `2 * SESSIONS` succeeded sessions on each side | `SESSIONS`. The `tls.handshakes` count (`SESSIONS`, the data flow only) keeps its value, because it counts handshakes |
| `engine/fps-agent/tests/first_party.rs`, `an_ftp_scenario_with_data_flows_runs_through_the_agent` (`ftp_run`, and `repeat_the_ftp_test`) | `ok >= 4` succeeded client sessions of the application ftp, "one full instance is four sessions", with a doc comment that says four client sessions for each instance | `ok >= 1`, one whole instance, and the comments say one session |
| the same file, `a_sip_call_with_a_media_flow_runs_through_the_agent` | `ok >= 4`, "several whole calls of two sessions each", with a doc comment that says two client sessions for each call | `ok >= 2`, two whole calls, and the comments say one session |
| `engine/fps-agent/tests/controller_e2e.sh`, the ftp run (`ftp_mixed`) | The criterion `sessions.succeeded > 3`, `ftp_succeeded -ge 4` and its message "at least one instance of four", and the comments that say four sessions for each instance | `> 0` and `-ge 1`, with "one whole instance" |
| the same script, the sip_rtp run (`sip_test`) | The criterion `sessions.succeeded > 3`, `sip_succeeded -ge 4` and its message "two whole calls of two sessions", and the comments that say two sessions for each call | `> 1` and `-ge 2`, "two whole calls" |
| the same script, `module_run hls vod_segments ... 7` | At least 7 succeeded sessions, "one whole instance gives one session for the playlists and six for the segments" | At least 1, "one whole instance gives one session" |

Searched and unchanged. These tests run one flow, or they count something other than sessions:

- `engine/fps-runtime/tests/{loopback,capture,properties,raw,extensions,tls_upgrade}.rs` and `engine/fps-agent/tests/{standalone,tls,run_control,session,extensions,contract}.rs` run scenarios of one flow, and their counts stay as they are.
- `engine/fps-interp/tests/instances.rs` runs multi flow instances with `spawn` and `after`, and it reads the flow records and errors of the interpreter and no `sessions.*` series. It changes only if ENG-30 changes `SideRecord`.
- `modules/*/tests/scenarios.rs` and `modules/*/native/tests/` measure the bytes of the connections of an instance over an in memory network, and `ESTIMATE_SESSIONS` counts instances. No test of a module reads a session counter. The bytes they measure are the bytes of the whole session, as the rule says.
- `controller/` tests (`internal/runs`, `internal/report`, `internal/metrics`, `cmd/fps-controller/e2e_test.go`) feed synthetic series and no multi flow engine run, so no count of theirs depends on the number of flows. `controller/internal/runs/calibration.go` (`clientTotals`) and `criteria.go` take the series as they are, and CTL-49 checks them against the engine after ENG-30.
- `cli/`, `sdk/python/`, and `ui/src/` read `sessions.*` from the API and test against synthetic runs, `tests/gates/gate1/` runs scenarios of one flow, and `tests/gates/gate2/mix_config.py` and `gate2_report.py` read the session counts and bytes of the run report as they are. `engine/fps-agent/src/criteria.rs` and `run.rs` evaluate the criteria on the series of the engine, so they follow ENG-30 without an edit. None of them depends on the number of flows.
- `realism/` reads captures and labels. `realism/mix.py` reads `sessions.attempted` and the bytes of a run report as the metrics side of the mix check, and its Limits paragraph (a scenario that opens several connections per session has more sessions in a capture than in the metrics) is right under the new rule. `realism/tests/test_structure.py` builds instances from synthetic captures.

### Gate 2 and the controller (CTL-49)

| File | What changes |
|---|---|
| `.github/workflows/qa-gate2.yml` | `FPS_GATE2_BASIS: sessions` goes once the bytes basis passes (CTL-49) |
| `tests/gates/gate2/mix.yaml` | The header comment says that the run report seems to count the FTP data connection and the media stream as sessions. It stops. The `expected_bytes` of `ftp/retr_epsv` (7765) and `sip_rtp/call_pcmu` (37230) are the bytes of a whole session and stay |
| `docs/wiki/Gates.md` | The paragraph on the byte basis and the paragraph on the earlier estimates say the run counts the data connection and the media stream as sessions |
| `controller/internal/runs/calibration.go` and its tests | Read `sessions.succeeded` and the bytes of the client side. No code change is expected, and CTL-49 checks the calibration on a run of `ftp/retr_epsv` and `sip_rtp/call_pcmu` |

### Contract copies and pins

| File | What changes |
|---|---|
| `controller/internal/metrics/catalog.yaml` | A byte copy of the catalog (`test_controller_copies_are_byte_identical`). I applied the copy on this branch, as the coordinator authorized |
| `controller/internal/metrics/testdata/prometheus.txt` | No change, because the descriptions that it prints are unchanged |
| `tools/contracts/test_contracts.py`, `realism/tests/test_mix.py` | No change, because the catalog version stays at 0.8.0 |
| `ui/src/api/schema.d.ts` | No change. It embeds no description of the catalog |
| `controller/internal/registry/schema/scenario.schema.json` and the other schema copies | No change, because the schemas are untouched |

### Documentation

| Page | What changes |
|---|---|
| `docs/wiki/Module-Development.md` | The section Flows, the row `session.id`, and the section `expected_bytes` say what a session holds. The exact text is in the Shared page notes of the log entry of this package |
| `docs/wiki/Engine.md` | See the engine code table (ENG-30) |
| `docs/wiki/Gates.md` | See Gate 2 (CTL-49) |
| `docs/wiki/Contracts.md` | A section for this change, in this package |
| `modules/ftp/README.md`, `modules/sip_rtp/README.md`, `docs/wiki/Modules.md` | They name connections and flows and not sessions of a scenario, and say nothing about a counter, so they stay |

## Verification

`python3 tools/contracts/validate.py` passes, `python3 -m pytest tools/contracts` passes (it holds the check that the controller copy of the catalog is byte identical), and `python3 tools/plan/plan.py check` passes. The Go and Rust tests of the consumer tables run in the CI of the consumer packages, not here.
