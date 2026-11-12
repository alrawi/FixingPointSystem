# Gate 2 acceptance suite

This suite automates the exit criterion of Phase 2. QA-05 added it. Gate 2 holds when all 15 application modules of decision D7 pass the realism checks in module CI, a byte weighted enterprise mix stays within 2 points of its weights through the controller and the agent, and the qcow2 image boots and runs a test unattended. The wiki page [Gates](../../../docs/wiki/Gates.md) explains the design.

## Files

| File | Purpose |
|---|---|
| `run.sh` | Runs the three stages in order and writes the gate report, or writes the report again from the files of an earlier run (`--report`) |
| `build-peers.sh` | Builds the fixture peer of every native module as the calling user |
| `modules.sh` | Module stage: records and scores every native module with `realism/ci/module-realism.sh` |
| `mix.sh` | Mix stage: runs the byte weighted enterprise mix through the controller and the agent and scores the capture, or scores an earlier capture again (`--rescore`) |
| `image.sh` | Image stage: builds the qcow2 image, boots it, and runs a test unattended inside |
| `prove-failure.sh` | The negative proofs, which show that the gate fails on worse results |
| `gate2_report.py` | Decides the verdict from the files of the stages and writes `report.md` and `summary.json` |
| `mix.yaml` | The 15 entries of the mix with weights, byte estimates, parameters, ports, and nDPI names |
| `mix_config.py` | Writes the test configuration, the labels, and the staged module trees from `mix.yaml` |
| `mix_findings.py` | Prints every parser finding of the mix capture with its place in the capture and writes `mix/parser-findings.json` |
| `test_gate2_report.py`, `test_mix_config.py`, `test_mix_findings.py` | Unit tests of the three Python files |
| `requirements.txt` | The Python tools of the lint and test targets |

## Commands

```bash
make build-qa-gate2                     # agent, controller, and the 14 fixture peers
sudo make e2e-qa-gate2                  # the module stage and the mix stage
make e2e-qa-gate2-image                 # the image stage, which needs Packer, QEMU, and KVM
make lint-qa-gate2 test-qa-gate2        # shellcheck, ruff, mypy, and the unit tests
sudo tests/gates/gate2/run.sh PEERS CONTROLLER AGENT OUT [IMAGE]
tests/gates/gate2/run.sh --report OUT   # the report of the files in OUT
tests/gates/gate2/prove-failure.sh OUT  # the negative proofs
```

`run.sh` exits with 0 when the gate passes, 1 when a gate check fails, and 2 when the suite cannot run. A stage that is selected and has no files fails its checks, so a stage cannot pass by being absent. The suite needs root for the first two stages, `ip`, `tcpdump`, `ss`, `curl`, `tar`, Python 3.10 or newer with PyYAML, `ndpiReader`, `tshark`, and Zeek (`realism/ci/install-tools.sh` installs them on Ubuntu 24.04), and for the image stage Packer, QEMU, Go, and the Rust musl target.

| Variable | Default | Meaning |
|---|---|---|
| `FPS_GATE2_STAGES` | `modules,mix,image` | Stages that `run.sh` runs and that the report judges |
| `FPS_GATE2_THRESHOLD` | `0.99` | Conformance threshold of every application. A value above 1 always fails |
| `FPS_GATE2_TOLERANCE` | `2` | Largest difference in points between a measured and a configured byte share |
| `FPS_GATE2_MODULES` | all | Modules of the module stage, for `build-peers.sh` and `modules.sh` |
| `FPS_GATE2_RATE_BPS` | `8000000` | Throughput objective of the mix run |
| `FPS_GATE2_RAMP_S`, `FPS_GATE2_STEADY_S`, `FPS_GATE2_RAMP_DOWN_S` | `10`, `120`, `10` | Phases of the mix run in seconds |
| `FPS_GATE2_SEED` | `7` | Seed of the mix test configuration |
| `FPS_GATE2_MIX_WAIVE` | none | Modules that the mix leaves out because the agent does not run them yet (see Waivers) |
| `FPS_GATE2_BASIS` | `bytes` | `profile.basis` of the controller run. `sessions` sets the session shares from the byte weights in the suite and needs `--allow-waivers` (see Waivers) |
| `FPS_REALISM_PARSERS` | `tshark,zeek` | Reference parsers of the realism runner |
| `FPS_NETNS` | `gate2` | Name prefix of the mix harness |
| `FPS_ACCEL` | by `/dev/kvm` | `kvm` or `tcg`, the accelerator of the image stage |

## Stages and checks

1. **Modules.** Each native module (`python3 -m realism modules`) records a fresh capture in the namespace harness with its fixture peer and takes every realism check that its `fixtures/expected.yaml` enables. The gate requires, per module, that every check passed, that the conformance and parser error checks ran, that the lowest conformance ratio is at least the threshold over at least 10 sessions, and that tshark and Zeek ran with no finding. A second table maps the 15 applications to their evidence. HTTPS needs the encryption check of both `http1` and `http2`, which decrypts a sample of sessions and compares their JA4 fingerprints.
2. **Mix.** `mix.yaml` holds one entry for each of the 15 applications with byte weights that add up to 100. The controller turns the weights into session shares with the estimates of `expected_bytes` and corrects the mix during the run (CTL-34 and CTL-12). The gate requires that the client run completed with a passed verdict, that every module attempted a session, that the profile basis is `bytes`, that the byte share of every application stays within the tolerance of its weight both by the run metrics and by nDPI, that unmatched traffic stays within the tolerance, that the conformance and parser error checks pass, and that tshark and Zeek ran.
3. **Image.** The boot test of IMG-04 builds and boots the qcow2 image and runs HTTP and DNS inside the VM through the controller of the image. The gate requires `RESULT: PASS` with no `FAIL` line, a completed run with a passed verdict, and succeeded sessions for every application of the run.

## Output

| File in OUT | Content |
|---|---|
| `report.md`, `summary.json` | The gate report with every check and its measured value, and the same as data |
| `modules/NAME/NAME.realism.json` | The realism report of each module, with the capture and the module log |
| `modules/results.json` | The exit code of each module |
| `mix/mix.pcapng` | The capture of the router's client arm |
| `mix/realism.json` | The realism report of the mix capture, with the `mix_accuracy` check |
| `mix/parser-findings.json` | The parser findings of the mix capture with the facts of their sessions (see Parser findings) |
| `mix/run.json` | `GET /v1/runs/{id}` of the client run, which carries `calibration` (see Calibration) |
| `mix/run-report.json`, `mix/run-report.csv`, `mix/run-metrics-scenario.json` | The report and the scenario metrics of the client run |
| `mix/test-config.yaml`, `mix/expected.yaml` | The configuration and the labels that `mix_config.py` wrote |
| `mix/context.json` | Run ids, rate, phases, seed, tolerance, waivers, and commit |
| `mix/controller.log`, `mix/client-agent.log`, `mix/server-agent.log` | Logs of the three processes |
| `image/boot-test.log`, `image/run/`, `image/context.json` | The log of the boot test, the files of the run in the VM, and the accelerator and image digest |

## Calibration

The report shows how the mix control moved the client run (QA-12). `mix.sh` stores `GET /v1/runs/{id}` of the client run as `mix/run.json`, and the section `Calibration` of the report, beside the byte shares, gives the last step of `Run.calibration` (the steps are oldest first, so the last one is the state at the end): the number of steps, the time, the factor, the target and the session rate that the factor implies, and a row per mix entry with its bytes per session, whether the estimate is measured, and its byte mix multiplier. The controller records steps only for a `throughput_bps` objective and only once the factor leaves one, and multipliers only with `profile.basis: bytes`. A run without steps, or a last step without multipliers (the session basis waiver), gets a line that says so. The section is information and never decides the verdict. An output directory of an earlier run has no `mix/run.json`, and the report then reads the same `calibration` from the `run` object of `mix/run-report.json` and names the file it used.

## Parser findings

A failed parser error check names each finding in the job log, so a failure can be read without the run artifacts (QA-11). Two parts do it. The report of `gate2_report.py` has a section `Parser findings` for the mix, and `Parser findings of module NAME` for a module, with one line for each finding that the realism report lists (the first 100): the parser, the kind, the protocol layer, the message, the frame and its time in UTC, the session with its application, and the endpoints. A report that cannot list all findings says how many it leaves out. The mix stage also runs `mix_findings.py` after the realism runner. It prints a line for each finding on stderr with the offset of the finding in the capture, the packets, connections, FIN and RST of its session, `cut` (whether the capture start or stop cut the session) and `reused_tuple` (whether two connections that used one 5-tuple form the session), and `mix.sh` prints the packet count and the first and last packet time of the capture (`capinfos`) before them. A fault in this step never decides the stage. `prove-failure.sh` proves for a module and for the mix that a failed parser error check fails the gate and that the report names the finding.

**The port collision rule.** The realism runner applies it since RLM-19, for the mix and for every module (`realism/README.md`, Parser errors). The kernel picks the client port of each session at random, and tshark binds about 900 TCP and UDP ports to a dissector (`tshark -G decodes` lists them), among them 41170 (Manolito) and 44322 (Performance Co-Pilot proxy), so a clean session that draws such a client port gets findings by chance (QA-11 saw 31 in one run). The runner leaves such a tshark finding out of `errors_total` and lists it under `not_counted`, and `gate2_report.py` takes the verdict of the runner and prints each of them marked `not counted`. The suite has no copy of the rule of its own.

## Waivers

The agent runs a module only when `engine/fps-agent` links its native crate. AGT-21 adds `ntp`, `mqtt`, `ldap`, and `hls`. `FPS_GATE2_MIX_WAIVE` leaves such modules out of the mix, and the report then says `PASS WITH WAIVERS`, names them, and states that Gate 2 does not hold in full. A waiver only holds while the agent lacks the crate. Once `fps-agent` depends on it, the module is part of the mix again, so merging AGT-21 ends the waiver without an edit. `FPS_GATE2_BASIS=sessions` is a second waiver: the controller catalog does not pass the `expected_bytes` of the manifests to the compiler, so `profile.basis: bytes` fails with `ErrNoByteEstimate`. The suite then computes the session shares and the session rate from the estimates of `mix.yaml`, and the capture is still scored against the byte weights (`mix/score-config.yaml`). The report names this waiver and the verdict stays `PASS WITH WAIVERS`. The workflow sets the variable and the owner reads the verdict line before signing off the gate.

## Negative proofs

`prove-failure.sh` takes the files of a passing run and expects the gate to fail four times. A conformance threshold just above the lowest measured ratio must fail it. A failed parser error check in the `http1` module report must fail it and must name the check and the finding. The mix capture scored again with a tolerance of 0 points must fail the mix stage. A failed parser error check in the mix report must fail it and must name the check and the finding. The report job runs the first two proofs on the files of all three stages, and the mix job runs the last two on its own capture.
