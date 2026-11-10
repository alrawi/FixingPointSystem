## [2026-11-10] ingest | TRN-16 Per profile captures scored with the transport check

`engine/fps-backend-kernel/tests/netns/profiles.sh` now scores the client capture of each OS profile with `python3 -m realism score --os-profile NAME --client-hops 0 --server-hops 1`, strict for linux6 and not strict for windows11 and macos14 (their option order, and for macos14 the SYN window, stay in the report as deviations with `enforced: false`). The test reads the `transport_behavior` entry of the report and not the exit code, because the conformance check of the runner always runs and fails on the probe's protocol, which nDPI reports as Unknown. An entry that is missing counts as a failure.

A capture recorded with the wrong profile fails. After the baseline session the test scores each capture with the next profile of its list and the baseline capture as linux6, and each of these scores must give a failed `transport_behavior` entry. The sandbox ran as root with working namespaces, so the whole test ran here: all three profiles pass (windows11 and macos14 not strict, linux6 strict), the four wrong profile scores fail, and with `ttl: 64` written into `windows11.yaml` the windows11 capture fails with `the profile expects 64, the capture shows 128` (the file was restored). The reports are copied to `FPS_KEEP_CAPTURES` next to the captures, so the artifact of the `capture` job holds them.

Finding, the window scale 10 of the harness fixtures: a new network namespace copies `net.ipv4.tcp_rmem` of the host, which is `4096 131072 33554432` here, and the scale is the number of halvings that bring the limit under 65536, which is 10 for 33554432 and 7 for the 6291456 of linux6. A fresh harness without a profile gives 10 in the SYN and the SYN-ACK (measured). It is the inherited `tcp_rmem` and not `net.core.rmem_max` (4194304 here, below the profile limit). Sessions of the capture test are not affected, because `profiles.sh` writes the profile `tcp_rmem` in both namespaces first. The fixtures stay as they are, and `realism/examples/transport.labels.yaml` keeps `expect: {window_scale: 10}` for them. A side finding: the baseline session of `profiles.sh` runs after macos14 and so keeps its `tcp_rmem` maximum of 4194304, which gives window scale 7, not the 10 of an unconfigured namespace. The test checks only TTL and MSS there, so nothing breaks.

### Shared page notes

- [trn.yml workflow](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/trn.yml) (coordinator file): the `capture` job needs ndpiReader and PyYAML, and nothing else, because the test calls `python3 -m realism` from the repository root (no `pip install ./realism`, no tshark, no Zeek; the runner stdlib only, and the labels file is JSON). Change the install step of that job to:

  ```yaml
      - name: Install the harness tools
        run: tools/ci/apt-install.sh iproute2 tcpdump ethtool libndpi-bin python3-yaml
  ```

  The step runs through `tools/ci/apt-install.sh`, so no bare apt-get. Ubuntu ships nDPI 4.2.0 as `libndpi-bin`, and that is enough, because the test ignores the conformance verdict. `sudo make e2e-trn` uses the system `python3`, where `python3-yaml` is the module that the profile files need, so do not add a `pip install` or `setup-python` for it. The `paths:` filters of the workflow already include `engine/fps-backend-kernel/**`, so the job runs for this change; the filters need `realism/**` if the coordinator wants the capture job to run when the runner changes (optional).
- [Testing](Testing.md): none. [Home](Home.md) and [Implementation Plan](Implementation-Plan.md): none. [Transport](Transport.md) is updated (section "Scoring with the transport check").
- [Realism](Realism.md): the statement of RLM-09 that the namespaces allow a larger receive buffer than the profile holds is right, but the cause is the inherited `tcp_rmem`, not `net.core.rmem_max`. A sentence there can say so.
