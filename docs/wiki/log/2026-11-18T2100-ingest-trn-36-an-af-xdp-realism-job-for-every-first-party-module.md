## [2026-11-18] ingest | TRN-36 an AF_XDP realism job for every first party module

TRN-36 runs every first party module that has scenarios through the agent forced onto AF_XDP and scores the capture like its kernel run. It edits only `engine/fps-datapath/realism/` (the generic script, `make_plan.py`, `finalize.py`, `profiles.yaml`, and their tests), `engine/fps-datapath/results/sandbox-2vcpu-trn36/` (new), `docs/handoff/TRN-36.md`, this entry, and the Transport page (a new section, "Every first party module on AF_XDP (TRN-36)"). It changes no Rust code, workflow, `mk/` file, or `Cargo.lock`.

- `module-afxdp.sh MODULE AGENT_BINARY OUTPUT_DIR` (root) is the TRN-12 script for any module. `http1-afxdp.sh` is now a call of it, with the same arguments, so the job of the TRN-12 entry keeps working.
- `make_plan.py` builds one plan for each round. A plan listens once for each port of a server host, and most modules run all scenarios on one port, so a round holds scenarios until two would listen on one transport and port, and the script runs the agent once for each round into one capture. The labels of the kernel run keep their ports (TRN-12 had moved the plain http1 scenarios to other ports). Each round has its own client prefix, MAC base, and seed, so no two sessions of the capture share a 5-tuple.
- `profiles.yaml` holds what each `capture.sh` sets: sessions, variables, moved ports, skipped scenarios, the decrypted check, the checks that the kernel run makes next to the score, and the exclusion list. `finalize.py` puts the sessions that the agent started into the labels that name a count (the Poisson arrivals of the agent do not give the counts of the kernel run), and runs those checks (`structure_check.py` of ftp and the `zeek_check.py` of ftp, smtp, ssh, and sip_rtp, with the counts that follow from the started sessions).
- `content` and `tls` have no scenarios and no run: `content` is a function library that the mail, ftp, and hls scenarios call, and `tls` is the TLS library that the https scenarios exercise.

Evidence (sandbox, 2 vCPU, root, kernel 6.18, debug build, `engine/fps-datapath/results/sandbox-2vcpu-trn36/`). I built nDPI 4.14 with `realism/ci/build-ndpi.sh` and installed Zeek 9.0.1 from the repository that `install-tools.sh` uses, so the runs score with the tools of CI (tshark 4.2.2, Zeek 9.0.1, nDPI 4.14). All 15 modules pass: every session that the agent started succeeded, and conformance, parser errors (tshark and Zeek), and every check of the kernel run pass. Run times are 8 to 32 seconds each.

| Module | Rounds | Scenarios | Sessions | Checks scored |
|---|---|---|---|---|
| dns | 6 | 6 | 116 | conformance, parser errors |
| ftp | 5 | 5 | 36 | conformance, parser errors, structure_check.py, zeek_check.py |
| hls | 1 | 2 | 6 | conformance, parser errors, temporal |
| http1 | 6 | 10 | 196 | conformance, parser errors, encryption, decrypted view |
| http2 | 1 | 4 | 87 | conformance, parser errors, encryption, decrypted view |
| imap | 5 | 6 | 46 | conformance, parser errors, encryption, decrypted view |
| ldap | 4 | 4 | 27 | conformance, parser errors |
| mqtt | 5 | 5 | 21 | conformance, parser errors |
| ntp | 5 | 5 | 30 | conformance, parser errors |
| pop3 | 5 | 5 | 36 | conformance, parser errors |
| sip_rtp | 5 | 5 | 18 calls | conformance, parser errors, temporal, zeek_check.py |
| smb2 | 3 | 3 | 22 | conformance, parser errors |
| smtp | 3 | 7 | 57 | conformance, parser errors, encryption, decrypted view, zeek_check.py (plain, and the decrypted view) |
| ssh | 4 | 4 | 19 | conformance, parser errors, zeek_check.py |
| strike | 1 | 1 | 23 | conformance, parser errors, structure |

### The exclusion list

Three scenarios cannot pass on AF_XDP yet. They are left out of the run (the key `excluded` in `profiles.yaml`, which is the machine readable list that AGT-29 reads in `check_af_xdp`), and the kernel fallback of AGT-29 covers them: an unforced agent runs a plan with one of these mix entries on the kernel backend and reports it, a forced agent refuses it with the cause. No module needs ICMP, UDP datagrams above the MTU, several pools on a side, VLANs, or DHCP. Every other scenario of these modules, and every other module, is not excluded.

| Mix entry | Cause (evidence: `engine/fps-datapath/results/sandbox-2vcpu-trn36/excluded/<module>/`, a run with `FPS_AFXDP_EXCLUDED=run`: the report and `agent-N.log`) | Fix, outside my paths |
|---|---|---|
| `dns/tcp_fallback` | The TCP leg of the scenario (`after: udp`) binds TCP 53 on the server only when the UDP leg ends, and the client dials right after the answer. After the first bind the hub of AGT-25 keeps the datapath listener of the port open without a subscriber, so the next SYN gets a SYN-ACK and the first data segment gets `RST`. The client retries a refused connect and not a reset, so the session fails with `unexpected_reset` (about 1 session of 24 in an earlier run, intermittent: the evidence run passed 24 of 24; a kernel answers that SYN with an RST at once, and the client retries) | `engine/fps-agent/src/transport.rs`, `Port::route`: do not hand back a session at once when no call holds its host. Keep it for `PENDING_GRACE` (2 s, `engine/fps-runtime/src/rendezvous.rs`) and give it to the next call that subscribes the host, and reset it when the grace ends. Module-Development already tells authors to hold such a connection for that long |
| `hls/vod_segments` | Each playback spawns 33 segment flows on TCP 8092, and the server side of each binds after the client dialed, so the same reset fails the playback (0 of 1 playbacks in the evidence run, `unexpected_reset`, and the temporal check fails), and the temporal checks then see a segment session without a request | The same change |
| `strike/conn_churn` | The client connects and sends an `RST`, and the server accepts and sends an `RST`. The own stack drops a connection that was reset before `accept`, so the accept times out (`connect_timeout`; 1 of 24 sessions passed in the evidence run, and the structure check fails); a kernel keeps the connection in the accept queue and `accept` returns it | `engine/fps-tcp-own`: deliver such a connection to the listener, and let its first read report the reset |

Removing an entry from `excluded` puts the scenario back in the run (and `drop_rules`, `drop_applications` hold the labels that went with it).

Findings:

- **The first UDP datagrams are lost without a pause.** With the agent started at once after the bridge came up, the first round lost its first datagrams (dns `lookup_a`, ntp `sntp_query`: 3 to 4 sessions failed with `expect_timeout`, and a UDP session does not retransmit). The script waits a second (`FPS_AFXDP_SETTLE`). Every later round was clean. A production agent on a freshly attached port may meet the same loss; I did not find the cause beyond the timing.
- **The profiles are the capture scripts.** The variables that matter are easy to miss: ntp needs `--poll-ms 1000` of its peer (`poll_interval_ms` 1000 and `poll_interval_long_ms` 2000, else `poll_cycle` waits 64 and 128 seconds), ftp and sip_rtp need the host addresses in their payload (`server_ip`, `client_ip`), which the script sets to the first hosts of the pools of the round (sip_rtp uses one client host so that the SDP names the sender), and the TLS scenarios of http2 need the small bodies.
- **Seeded arrivals.** The same seed gives the same sessions for each scenario, so a run repeats its counts (two consecutive runs passed the same rows). The counts are not the kernel counts, and a scenario with a count of 2 or 3 (ssh `scp_download`, mqtt `subscribe`, sip_rtp `call_opus`) has few sessions. Use `FPS_AFXDP_SESSIONS` for more.
- **Two more sweeps.** The whole set passed in two further consecutive sweeps. One ldap run of the second sweep failed because I edited the script while it ran, and the rerun passed.
- **Not verified here:** the hosted runner. The sandbox kernel has no IPv6, and the runs use IPv4 pools, as the agent's veth test does. The coordinator's job runs everything on the hosted runner.

### Shared page notes

**CI job (coordinator, new file `.github/workflows/trn-afxdp-modules.yml`).** One matrix job for the 15 modules that pass, behind a job that builds the agent once. It uses hosted `ubuntu-24.04`, `sudo`, and `realism/ci/install-tools.sh` (which builds nDPI 4.14 and installs Zeek), with a timeout of 30 minutes for each module (a run takes under a minute, and the install takes the rest). It keeps no secret and passes the module through `env`. `modules/**` is in the path filters because the profiles read the modules. Add `afxdp-modules` to the required checks only after it is green once. Once it is, the `afxdp` job of `mod-http1.yml` (TRN-12) is a duplicate for http1 and can go.

```yaml
# SPDX-License-Identifier: GPL-3.0-only
#
# The AF_XDP realism run of every first party module (TRN-36). The agent job builds
# fps-agent once. The modules job runs one job per module on a GitHub hosted runner,
# as root through passwordless sudo: it runs the scenarios of the module through the
# agent forced onto AF_XDP between two veth pairs and scores the capture like the
# kernel run of the module (engine/fps-datapath/realism/module-afxdp.sh). The
# scenarios on the exclusion list of TRN-36 are not run (profiles.yaml, key
# `excluded`). The jobs use no secret, and no event text reaches a script.
name: trn-afxdp-modules

on:
  push:
    branches: [main]
    paths:
      - engine/**
      - modules/**
      - realism/**
      - tests/harness/pcapng.py
      - tools/ci/apt-install.sh
      - Cargo.toml
      - Cargo.lock
      - .github/workflows/trn-afxdp-modules.yml
  pull_request:
    paths:
      - engine/**
      - modules/**
      - realism/**
      - tests/harness/pcapng.py
      - tools/ci/apt-install.sh
      - Cargo.toml
      - Cargo.lock
      - .github/workflows/trn-afxdp-modules.yml

permissions:
  contents: read

concurrency:
  group: trn-afxdp-modules-${{ github.ref }}
  cancel-in-progress: true

jobs:
  agent:
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo build -p fps-agent
      - uses: actions/upload-artifact@v4
        with:
          name: fps-agent
          path: target/debug/fps-agent
          retention-days: 1
          if-no-files-found: error

  modules:
    needs: agent
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        module:
          - dns
          - ftp
          - hls
          - http1
          - http2
          - imap
          - ldap
          - mqtt
          - ntp
          - pop3
          - sip_rtp
          - smb2
          - smtp
          - ssh
          - strike
    steps:
      - uses: actions/checkout@v4
        with:
          persist-credentials: false
      - name: Install the harness and realism tools
        run: realism/ci/install-tools.sh
      - uses: actions/download-artifact@v4
        with:
          name: fps-agent
          path: agent
      - run: chmod +x agent/fps-agent
      - name: Record the module on the AF_XDP backend and score it
        env:
          MODULE: ${{ matrix.module }}
        run: |
          sudo env "PATH=/opt/zeek/bin:$PATH" FPS_AFXDP_PARSERS=tshark,zeek \
            engine/fps-datapath/realism/module-afxdp.sh "$MODULE" "$PWD/agent/fps-agent" "$RUNNER_TEMP/afxdp-$MODULE"
      - name: Keep the result files
        if: always()
        env:
          MODULE: ${{ matrix.module }}
        run: |
          sudo chown -R "$(id -u):$(id -g)" "$RUNNER_TEMP/afxdp-$MODULE" 2>/dev/null || true
          rm -rf "$RUNNER_TEMP/afxdp-$MODULE/modules"
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: afxdp-${{ matrix.module }}
          path: ${{ runner.temp }}/afxdp-${{ matrix.module }}
          if-no-files-found: ignore
          retention-days: 7
```

A per module job in `qa-gate2.yml` works as well, with the same step (the matrix above is the simpler choice, because a failure names its module).

**Agent and stack fixes (outside the paths of TRN-36; an AGT and a TRN package apply them).**

1. (bug #688) `engine/fps-agent/src/transport.rs`, `Port::route` and the hub task that resets a session without a call: keep an unrouted session for `PENDING_GRACE` and give it to the next call that subscribes its host (the table of the exclusion list above). It lets `dns/tcp_fallback` and `hls/vod_segments` back into the run. Test: a plan of dns `tcp_fallback` with 100 sessions on a veth pair runs with no failed session, as the kernel run of the module does (`unexpected_reset` appeared in 1 of 24 sessions of this scenario and in the first playback of `hls/vod_segments`). Then remove both entries from `excluded` in `profiles.yaml` and the labels they dropped come back with them.
2. (bug #689) `engine/fps-tcp-own`: a connection that was reset before `accept` goes to the listener (the cause of `strike/conn_churn`). Test: `conn_churn` on AF_XDP with 100 sessions and no failed session. Then remove `conn_churn` from `excluded`.
3. AGT-29: `check_af_xdp` refuses these mix entries by `module/scenario`: `dns/tcp_fallback`, `hls/vod_segments`, and `strike/conn_churn`. The list is the key `excluded` of `engine/fps-datapath/realism/profiles.yaml` (YAML, one reason for each). Do not copy it by hand: read it or keep a test that compares the two.

**Module development, "Watch out for" (exact entry; the Module development page is not in my paths).** Place it under "Listeners, ports, and spawned flows".

> **A TCP flow that follows another on the same port is reset on AF_XDP.** You see a session fail with `unexpected_reset`, and a capture with SYN, SYN-ACK, ACK, the first data segment, and an `RST` from the server, in about 1 session of 24 of a scenario like `dns/tcp_fallback` (a UDP leg, then a TCP leg on the same port with `after`) or in a long playback of `hls/vod_segments` (33 spawned flows on one port). The datapath listener of a port stays open after the first flow bound it, and the hub resets a connection that arrives while no flow waits, after the handshake, which the client does not retry as it retries a refused connect. A kernel answers that SYN with an RST, and the client retries. Until the hub holds such a connection for the grace period, these scenarios are on the exclusion list of the AF_XDP realism run and run on the kernel backend (TRN-36).

**Module development, "What you can do today" or the section on first party modules (exact text).** "A first party module needs an entry in `engine/fps-datapath/realism/profiles.yaml` with the sessions, variables, and ports of its `fixtures/capture.sh`. `test_every_first_party_module_has_a_profile` fails without it, and the AF_XDP realism job (`engine/fps-datapath/realism/module-afxdp.sh MODULE AGENT OUT`) runs the module from it. A scenario that the agent cannot run on AF_XDP yet goes under `excluded` with its reason (TRN-36)." This changes no manifest field or scenario action, so third party authors are not affected.

**Testing page (exact text).** "`sudo engine/fps-datapath/realism/module-afxdp.sh MODULE target/debug/fps-agent OUT` records a module through the agent on AF_XDP and scores it like the kernel run of the module (every first party module with scenarios). It needs ip, ethtool, tcpdump, editcap, ndpiReader (4.14 for http2), tshark, PyYAML, and Zeek for the Zeek checks, and takes 8 to 32 seconds for a module. The unit tests are `python3 -m pytest -q engine/fps-datapath/realism`."

**`mk/trn.mk` (shared file, optional).** `mypy --strict` in `lint-trn` should name `$(TRN_AFXDP_REALISM_DIR)/finalize.py` next to `make_plan.py`. `shellcheck`, `ruff format --check`, and `ruff check` already cover the new files, and `test-trn` already runs the new tests. The tests need PyYAML and read `modules/`.

**`.github/workflows/trn.yml` (coordinator).** Add `modules/**` to the path filters of both triggers, because the tests of `profiles.yaml` read every module.

**Plan catalog.** TRN-36 changes nothing in the catalog. AGT-29 already depends on TRN-36 and reads the exclusion list above.

- `Cargo.lock`, `Cargo.toml`, Implementation Plan, sidebar, and the other wiki pages: unchanged.
