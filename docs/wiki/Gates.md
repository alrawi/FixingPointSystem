# Gates

This page describes the gate suites of the QA workstream. Each phase ends at a gate, and the [Implementation Plan](Implementation-Plan.md) names its exit criterion. A gate suite lives under `tests/gates/gate<N>/`, runs only public interfaces, and writes `report.md` with the measured values, which the owner reads before signing off the gate. QA-04 added the first suite.

## Gate 1

Gate 1 holds when an HTTP/1.1 and DNS mix is classified at 99% by nDPI through the controller and the agent. The suite [`tests/gates/gate1/`](https://github.com/alrawi/FixingPointSystem/tree/main/tests/gates/gate1) automates the criterion end to end, and its [README](https://github.com/alrawi/FixingPointSystem/blob/main/tests/gates/gate1/README.md) lists every file, setting, and output.

```bash
make build-qa-gate1          # fps-agent and fps-controller as the calling user
sudo make e2e-qa-gate1       # the suite and the negative proof
```

### Topology

```text
 gate1-client namespace                gate1-router          gate1-server namespace
 fps-controller (REST 127.0.0.1:7080)
 fps-agent gate1-client --side client
 clients 198.18.0.0/24 on lo  eth0 --- cli0  [capture]  srv0 --- eth0  servers 198.19.0.0/24 on lo
                                                                         fps-agent gate1-server --side server
                              mgmt0 ------------------------------------ mgmt0   (agent gRPC, not captured)
```

The suite brings up the QA-01 harness and starts the real `fps-controller` in the client namespace. It enrolls two identities of the real agent binary over REST and starts each with `fps-agent connect`. The agent `gate1-client` runs in the client namespace with `--side client` and dials the controller on the loopback. The agent `gate1-server` runs in the server namespace with `--side server` and dials over a veth link between the two namespaces, so control traffic never crosses the captured arm. The pools come from the benchmarking range of RFC 2544 and become local routes on the loopback of their namespace, and the router routes each pool to its arm, as the AGT-01 end to end test does.

We run two agent identities because the controller compiles one plan per agent with both sides on the slices of that agent, and one process lives in one namespace. Each identity runs its own run of the same configuration. The server run holds 120 seconds longer than the client run, and the suite stops it once the client run ends. The client run decides the gate, and its report is the run report of the gate.

### Steps

1. Bring up the harness, add the management link, and route the pools.
2. Start the controller with random tokens, enroll and start both agents, and wait until both are online.
3. Install the tracked trees of `modules/http1` and `modules/dns` through `POST /v1/modules`.
4. Store `test-config.yaml` once per agent through `POST /v1/configs` and validate it. The mix holds `http1/single_fetch` on port 80 with weight 0.35, `http1/keepalive_browse` on port 8080 with weight 0.25, and `dns/lookup_a` on port 53 with weight 0.4, at 100 sessions per second for a ramp of 5 seconds and a hold of 20 seconds.
5. Start the server run and wait until the server agent listens on every port of the mix.
6. Record the router's client arm, start the client run, wait until it ends, and stop the server run.
7. Fetch `GET /v1/runs/{id}/report` in JSON and CSV and `GET /v1/runs/{id}/metrics?view=application` of the client run.
8. Score the capture with `python3 -m realism score` and the parsers tshark and Zeek, write the parser findings with `parser_findings.py`, and write `report.md` and `summary.json` with `gate1_report.py`.

### Checks

| Check | Passes when |
|---|---|
| Client run verdict | The client run completed and its verdict passed, with `sessions.failed_ratio <= 0.01` |
| Sessions in the capture | Each of `http1` and `dns` has at least 100 sessions in the capture |
| Conformance | The conformance ratio of each application is at least the threshold, 0.99 by default |
| Realism conformance check | The conformance check of the realism report passed |
| Parser error check | tshark and Zeek ran and the parser error check of the realism report passed; the runner does not count the state weirds of Zeek, among them `premature_connection_reuse` (see Parser findings) |
| Captured sessions | The capture holds from 99% to 100% of the sessions of each application that the agent attempted |

The suite exits with 0 when every check passes, 1 when one fails, and 2 when it cannot run. The last check ties the scored traffic to the run, so a capture that missed sessions or held foreign sessions fails. It allows 1% fewer sessions, because two sessions that reuse one 5-tuple form one session of the realism runner. One CI run captured 976 of 977 `dns` sessions, which two queries with the same client address, source port, and server address explain.

### Negative proof

`prove-threshold.sh OUT` copies the capture and the run files of a passing run, sets the threshold 0.001 above the lowest measured ratio, and scores the copy again with `run.sh --rescore`. It passes only when the gate then exits with 1. A ratio of 100% gives the threshold 1.001, which no capture can meet. The realism runner accepts thresholds up to 1, so the suite passes it the smaller of the threshold and 1 and compares every ratio with the full threshold itself.

`prove-corruption.sh OUT` is the second negative proof, and `prove-threshold.sh` runs it after the threshold proof. It copies the capture of a passing run, overwrites one chunk size line of one chunked HTTP response with a chunk size of 17 hex digits (the frame keeps its length, so only the HTTP message is corrupt), and scores the copy again. It passes only when the gate exits with 1, the parser error check is among the failed checks, and `parser-findings.json` names the corrupted frame. tshark reports the frame as a malformed packet.

### Parser findings

The suite keeps the findings of the parser error check as files in OUT, in every run: `parser-findings.json` and `.csv` with the parser, the kind, the frame, the session, the facts of the session (packets, SYN, FIN, and RST counts, first and last time) and whether the capture boundary cut it, and `parser-findings.pcapng` with the frames of the sessions that carry a finding. `tcpdump.log` and the `capture_dropped` field of `context.json` carry the packets that the kernel dropped. The workflow uploads `*.json`, `*.csv`, `*.pcapng`, and `*.log` of OUT.

QA-09 looked into the single `http1` parser error that the suite reported twice on 2026-11-10 (once in the `gate` job of the CTL-39 branch and once in the push of MOD-12, both passing on rerun). Both runs had fewer `http1` sessions in the capture than the agent attempted (1337 and 1339 of 1340), which a capture boundary cannot explain, because a session that the stop cuts is still a session of the capture. Two connections that use one 5-tuple form one session of the realism runner, and `http1/single_fetch` lets the server close first, so the server side holds the old connection in TIME_WAIT and the client kernel may pick the same source port again. That occurred in 16 of the last 86 gate runs and in both failed runs; the 70 runs without a merged session had no finding. tshark 4.2.2 reported nothing in 24 local runs with the real agents, even in a capture with 259 reused sessions, so the finding is a Zeek weird, and the first CI run of the QA-09 branch (1338 of 1340 sessions) confirmed it: `parser-findings.json` listed one finding, the Zeek weird `premature_connection_reuse` in a session with two connections. The artifacts of the two original failures could not be read from the sandbox of QA-09, which had no Zeek.

QA-09 first let the gate skip `premature_connection_reuse` in a session with more than one TCP connection. RLM-17 then added the weird to `ZEEK_STATE_WEIRDS` in `realism/parsers.py`, so the realism report does not count it, and QA-10 removed the rule of the suite (`REUSE_WEIRDS`). The gate takes the verdict of the parser error check as it is, and a tshark finding or a Zeek weird that is not a state weird fails it. The findings files stay and still list `reused_tuple` for each finding.

### Measured values

A local run on 2026-11-03 with debug builds on a 2 vCPU Firecracker microVM (Linux 6.18.44, nDPI 4.2.0, tshark 4.2.2, Zeek 9.0.1) passed. The client run attempted 2,364 sessions and none failed. The capture held 1,382 `http1` sessions and 982 `dns` sessions, which equals the sessions that the agent attempted, nDPI classified all of them as HTTP and DNS for a ratio of 100% each, and the parsers reported no error. The negative proof failed the gate at the threshold 1.001 as expected. The `gate` job of the pull request on `ubuntu-24.04` passed with 1,340 `http1` and 977 `dns` sessions out of 2,317, again at 100% each with no parser error, and its negative proof failed the gate at 1.001. Two runs on the same machine gave the same counts, and the counts differ between machines because the arrivals follow the clock of the run. The workflow [`qa-gate1.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/qa-gate1.yml) runs the suite and the proof as root on `ubuntu-24.04`, adds `report.md` to the job summary, and uploads the reports as the artifact `gate1-report`.

### Limits

- The suite runs two agent identities, one per side, until the controller can place the client and server sides of one plan on different agents or the agent can enter a namespace per port.
- The suite captures only the client arm, which carries every session of the mix in both directions.
- Run ids carry the prefix `cfg_`, the same prefix as configuration ids, which we report to the CTL workstream.
