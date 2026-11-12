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

## Gate 2

Gate 2 holds when all 15 application modules of decision D7 pass the realism checks in module CI, a byte weighted enterprise mix stays within 2 points of its weights, and the qcow2 image boots and runs a test unattended. QA-05 added the suite [`tests/gates/gate2/`](https://github.com/alrawi/FixingPointSystem/tree/main/tests/gates/gate2), and its [README](https://github.com/alrawi/FixingPointSystem/blob/main/tests/gates/gate2/README.md) lists every file, setting, and output.

```bash
make build-qa-gate2            # agent, controller, and the 14 fixture peers
sudo make e2e-qa-gate2         # the module stage and the mix stage
make e2e-qa-gate2-image        # the image stage, which needs Packer, QEMU, and KVM
```

### Stages

| Stage | What it runs | Needs |
|---|---|---|
| Modules | `realism/ci/module-realism.sh` for each of the 14 native modules, with its fixture peer, in the namespace harness of QA-01 | root, tshark, Zeek, nDPI |
| Mix | The controller and two agent identities run the byte weighted mix of `mix.yaml` in the namespace harness, and the realism runner scores the capture of the client arm | root, the controller and agent builds, tshark, Zeek, nDPI |
| Image | `image/build.sh` and `image/boot-test.sh --role all-in-one --run` (IMG-04) | Packer, QEMU, and KVM or TCG |

`gate2_report.py` reads the files of the three stages and writes `report.md` and `summary.json`, each check with its measured value. A stage that is selected and has no files fails its checks, so the gate cannot pass by a missing stage. The exit code is 0 for a pass, 1 for a failed check, and 2 when the suite cannot run. HTTPS has no module of its own: it takes the encryption check of both `http1` and `http2`, which decrypts a sample of sessions and compares their JA4 fingerprints.

### The mix

`mix.yaml` has one entry for each of the 15 applications, with a byte weight (the weights add up to 100), the bytes per session that the controller needs to turn byte weights into session shares, the scenario parameters, and the nDPI names of the application row. Every entry listens on its own transport and port, because the labels assign a session to its application by its server port and nDPI cannot tell `http1` from `hls`, which share the row `http`. FTP data and RTP fall into the row `aux`, which has no weight. The run uses a throughput of 8 Mbit/s for 10 s of ramp up, 120 s of steady load, and 10 s of ramp down with the seed 7. The arrivals are Poisson per entry, so a row with few sessions varies by about its square root: the weights keep the rows with large sessions (SMB, HTTP/2, and HTTPS) above 100 sessions each.

### Waivers

Two waivers keep the suite honest about what the system cannot do yet. The report names both and then says `PASS WITH WAIVERS`, and the workflow sets them.

- **Modules the agent does not run.** `engine/fps-agent` links the native crate of a module to run it, and AGT-21 adds `ntp`, `mqtt`, `ldap`, and `hls`. `FPS_GATE2_MIX_WAIVE` leaves them out of the mix. A waiver lapses by itself once `engine/fps-agent/Cargo.toml` depends on the crate, so merging AGT-21 puts the four modules into the mix without an edit. Their module checks run in the module stage regardless.
- **The byte basis.** The controller reads `expected_bytes` since CTL-47, and every manifest carries it since MOD-25. A run on the bytes basis still misses the mix by up to 7.84 points, because the run counts the FTP data connection and the RTP media stream as sessions of their own. The calibration then measures half the real bytes of an ftp or sip_rtp session and doubles their shares. CTL-49 counts a spawned flow with the session that opened it. Until then `FPS_GATE2_BASIS=sessions` keeps the waiver.

### Negative proof

`prove-failure.sh` takes the files of a passing run and expects the gate to fail four times: with a conformance threshold just above the lowest measured ratio, with a failed parser error check in the `http1` module report (the report must name the check and the finding), with the mix capture scored again at a tolerance of 0 points, and with a failed parser error check in the mix report (the report must name the check and the finding). The report job of the workflow runs the first two on the files of all three stages, and the mix job runs the last two on its own capture.

### Parser findings and the port collision rule

The report also shows the calibration of the mix run (QA-12). `mix.sh` stores `GET /v1/runs/{id}` of the client run as `mix/run.json`, and the section `Calibration` gives the last step of `Run.calibration` (factor, target, session rate, and for each mix entry the bytes per session, whether it is measured, and the byte mix multiplier). The controller records steps only for a `throughput_bps` objective once the factor leaves one, and multipliers only on the basis `bytes`, so the report says so when a run has no step or a step has no multipliers. The section never decides the verdict.

A failed parser error check names each finding in the job log, because the run artifacts are not always readable (QA-11). The `Parser findings` section of the report lists the parser, the kind, the protocol layer, the message, the frame and its time, the session with its application, and the endpoints of each finding that the realism report lists. `mix_findings.py` adds a line for each finding of the mix with its offset in the capture, the packets, connections, FIN and RST of its session, whether the capture start or stop cut the session, and whether two connections of one 5-tuple form it, and `mix.sh` prints the packet count and the first and last packet time of the capture before them. The file `mix/parser-findings.json` keeps the same facts.

The first run that failed with the new lines (run 43233842055, attempt 3, 2026-11-12) had 31 findings, all from tshark: one `pcp` malformed packet in an `http2` session on TCP 44322 and 30 `manolito` malformed packets in an RTP stream on UDP 41170. The kernel picks the client port of a session at random, tshark 4.2.2 binds 922 TCP and UDP ports to a dissector (`tshark -G decodes`), and 44322 (Performance Co-Pilot proxy) and 41170 (Manolito) are two of them, so tshark decoded the payload with a dissector that does not fit it. That is the likely cause of the 10 findings that PR 486 saw in its first attempt too (QA-11 could not read the artifacts of that run). The **port collision rule** keeps these findings from failing the mix: a tshark finding does not count when the client port of its session is bound to a dissector for the transport and the server port is not bound to the same dissector. The rule holds only with a `parser-findings.json` that lists every finding of the realism report, it never applies to Zeek, and the report and the log still print each collision marked `not counted`. A finding in a session whose client port is free, an unlisted finding, and a missing file all fail the stage. The module stage has no such rule, because its sessions are too few to draw a port that collides.

QA-11 also found that the report job can judge a stale artifact: after a rerun of failed jobs, the run holds two artifacts named `gate2-mix`, and `actions/download-artifact` takes one of them by an id order that does not follow time. On PR 486 the report job of attempts 2 and 3 judged the failed mix of attempt 1 (client run `run_a48cc9ec...`), although the mix job of attempt 2 had passed (client run `run_fff8389d...`). The upload steps of `qa-gate2.yml` need `overwrite: true` (see the Shared page notes of the log entry).

### Measured values

The workflow run for pull request 487 on `ubuntu-24.04` passed in all five jobs on 2026-11-12 with the verdict `PASS WITH WAIVERS`.

- **Modules.** All 14 modules passed every check that their `expected.yaml` enables, with conformance at 100% over 16 (`ssh`) to 200 (`http1`) sessions and no finding of tshark or Zeek. HTTPS decrypted 80 of 80 `http1` sessions and 40 of 40 `http2` sessions with a JA4 match each.
- **Mix.** The 10 modules that the agent runs attempted 19,606 sessions in the client run (8,392 `dns`, 4,138 `ftp`, 1,697 `http1`, 213 `http2`, 799 `imap`, 491 `pop3`, 544 `sip_rtp`, 185 `smb2`, 1,510 `smtp`, and 1,637 `ssh`) and none failed. The largest difference between a byte share and its weight was 0.898 points by the run metrics (HTTPS) and 1.367 points by nDPI (HTTPS, where the TLS records add bytes that the metrics do not count), against the 2 point limit. Unmatched traffic was 0%, conformance was 100% for every application, and the parsers reported nothing. The mix scored again at a tolerance of 0 points failed the gate as required.
- **Image.** The boot test passed 51 checks with no failure on KVM, and the unattended run in the VM completed with a passed verdict and 557 succeeded sessions (349 `http1`, 208 `dns`).

An earlier run of the mix with guessed byte estimates missed the limit by up to 8.6 points, which shows that the check can fail. The estimates in `mix.yaml` now come from the bytes per scenario session of that run, where the run report seems to count the FTP data connection and the SIP media stream as sessions of their own (the session counts of both doubled against their weights). Another controller or a heavier agent changes these estimates only through the controller, and the byte basis ends the need for them once the catalog carries `expected_bytes`.

### Limits

- The mix leaves out `hls`, `ldap`, `mqtt`, and `ntp` until AGT-21 merges, and it runs on the session basis until the controller catalog carries `expected_bytes`. Gate 2 does not hold in full before both waivers end.
- Every manifest of a module with scenarios carries `expected_bytes` (MOD-25). The suite still stages the module trees with the estimates of `mix.yaml` for the mix scenarios, and does not change the tracked manifests.
- The suite runs two agent identities, one per side, as the Gate 1 suite does.
- The image stage needs Packer, QEMU, and a runner with KVM or the patience for TCG.
