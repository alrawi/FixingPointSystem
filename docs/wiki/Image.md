# Image

This page describes the appliance image of the IMG workstream. IMG-01 builds the qcow2 base image with Packer. The directory [`image/`](https://github.com/alrawi/FixingPointSystem/tree/main/image) holds the sources, and its [README](https://github.com/alrawi/FixingPointSystem/blob/main/image/README.md) gives the exact build and boot test steps. IMG-02 adds the role selector and the tuning profiles. IMG-03 adds the format conversion. IMG-04 adds the CI boot test, which the section [CI boot test](#ci-boot-test) describes. IMG-05 adds container images and a compose lab, which the section [Containers](#containers) describes.

## Build

`image/build.sh` builds the controller with Go (`CGO_ENABLED=0`) and the agent with `cargo build --release --target x86_64-unknown-linux-musl`, so that both are static and the host's C library does not matter, stages the modules `http1` and `dns`, and runs Packer on `image/fps.pkr.hcl`. The Packer QEMU builder boots the official Ubuntu 24.04 LTS cloud image (`disk_image = true`) with a throwaway ssh user from a NoCloud seed that Packer renders. Three steps follow inside the VM.

1. `scripts/provision.sh` checks that the kernel is Linux 6.3 or newer, installs `nftables`, `iproute2`, and the guest agent, installs the binaries and the tree of `image/files/`, enables the services, and smoke tests both binaries.
2. `scripts/cleanup.sh` installs the key only ssh policy, clears package caches, ssh host keys, the machine id, and cloud-init state, and trims free space.
3. The shutdown command removes the build user and powers the VM off, and Packer compresses the disk to `image/output/fps-base.qcow2`.

When `go` is missing or older than `controller/go.mod` asks for, the script downloads a pinned Go toolchain into `image/.cache` and checks its checksum against the go.dev download index.

## Services

| Unit | Role |
|---|---|
| `fps-firewall.service` | Loads `/etc/fps/isolation.nft` before the network comes up |
| `fps-mgmt-net.service` | Renames the management NIC to `mgmt0` before `systemd-networkd` starts |
| `fps-mgmt-addr.service` | Waits for the DHCP address of `mgmt0` and writes it to `/run/fps/mgmt.env` |
| `fps-role.service` | Reads the role from `/etc/fps/role.env` (cloud-init data), marks the units of the role, enables them, and disables the others |
| `fps-tune.service` | Applies the tuning profile of the role, and asks for the one reboot that kernel options need |
| `fps-controller.service` | Runs `fps-controller` as user `fps` with `-listen ${FPS_MGMT_ADDR}:8443 -grpc-listen ${FPS_MGMT_ADDR}:8444 -enroll-listen ${FPS_MGMT_ADDR}:8445` |
| `fps-agent.service` | Runs `fps-agent connect` against `https://${FPS_MGMT_ADDR}:8444` with agent id `local`, over mutual TLS. `fps-enroll-token` runs first (see [Mutual TLS and enrollment](#mutual-tls-and-enrollment)) |

Both service units start only in their roles (see [Roles and tuning](#roles-and-tuning)). The controller reads its token from `/etc/fps/admin.token`. A cloud-init `write_files` entry may provide the file, and otherwise `fps-init-token` creates a random token on the first start. The agent restarts after a crash or after it gave up redialing (exit code 3), and it does not restart after exit code 2, which a usage error or a kernel before Linux 6.3 causes.

## Roles and tuning

The role comes from cloud-init user-data. The seed writes `/etc/fps/role.env` with `FPS_ROLE` set to `controller`, `agent`, or `all-in-one`, and with `FPS_CONTROLLER_URI` for the agent. Without the file the role is `all-in-one`, so a VM from the first image behaves as before. At every boot `fps-role.service` writes one marker per unit of the role under `/run/fps/role.d/`, enables those units and disables the others, and records its work in `/etc/fps/role.applied`. The units of the controller and the agent hold `ConditionPathExists` on their marker, so a unit outside the role never starts, and an invalid role file starts neither. We use conditions on top of enable and disable because the first boot must not depend on the order in which systemd reads unit links.

| Role | Starts | Tuning |
|---|---|---|
| `all-in-one` | controller and agent | The agent sysctls and the loopback notrack table, with no kernel option and no reboot |
| `controller` | controller | None, because the controller is light |
| `agent` | agent, dialing `FPS_CONTROLLER_URI` | CPU isolation, hugepages, sysctls, notrack, and one reboot |

The agent profile follows the size of the VM. The housekeeping CPUs are `CPUS/4` rounded down (at least 1) counted from CPU 0, and a VM with fewer than 4 CPUs isolates none. The remaining CPUs go on the kernel command line as `isolcpus`, `nohz_full`, and `rcu_nocbs`, and `fps-agent-start` pins the agent to them with `taskset`, so the agent runs on quiet CPUs while the system and the controller use the housekeeping CPUs. A quarter of the memory becomes 2 MiB hugepages through `hugepages=N` on the command line, because a reservation at boot does not fail on fragmented memory. The sysctls are a source port range of 1024 to 65535, `somaxconn` 65535, deeper SYN and device backlogs, and file limits of 4194304 (the unit sets `LimitNOFILE` as well). The loopback skips connection tracking in the table `inet fps_notrack`, the table that the benchmark host setup in `bench/README.md` uses, and only when `nf_conntrack` is loaded.

Kernel options need a reboot, so the step runs once and is idempotent. `fps-tune` writes `/etc/default/grub.d/60-fps-tune.cfg`, runs `update-grub` when that file changes, and compares `/proc/cmdline` with the profile. When the options are missing it requests one reboot and writes `/run/fps/tune-reboot`, which holds the agent back until the reboot. When the options run it does nothing. When they are still missing after the reboot it records `status=failed` and does not reboot again. `/etc/fps/tune.applied` holds what was applied, the sizes, and the status. A one step change in the reported memory does not change the profile, so the options never flap between two reboots. The [README](https://github.com/alrawi/FixingPointSystem/blob/main/image/README.md) lists the keys and the recorded fields.

## Management interface and isolation

The management interface is the Ethernet device with the lowest PCI address, or the device whose MAC address `FPS_MGMT_MAC` names in `/etc/fps/image.env`. It becomes `mgmt0` and asks for an address with DHCP. Every other Ethernet device is a test interface. It gets no address, no router advertisement handling, and no route from the image. The agent and the host setup of [Hosts](Hosts.md) configure it when a run needs it.

| Layer | Rule |
|---|---|
| Listeners | The controller binds the REST and gRPC ports to the address of `mgmt0` only |
| Sysctl | `net.ipv4.ip_forward`, `net.ipv6.conf.all.forwarding`, and the `default` forwarding setting stay at 0 |
| nftables forward hook | `iifname "mgmt0"` and `oifname "mgmt0"` packets drop, so nothing routes between the management and a test interface even when forwarding turns on |
| nftables input hook | Connections to ports 22, 8443, 8444, and 8445 drop on every interface except `mgmt0` and the loopback, because Linux answers for a local address on any interface |

Both hooks run at priority -100 in the table `inet fps_isolation`. The rules apply to the namespace of the host. The kernel test ports that [Hosts](Hosts.md) moves into `fps-port-*` namespaces never share a routing table with `mgmt0`.

## Mutual TLS and enrollment

Since IMG-06 the agent and the controller speak mutual TLS and `controller.yaml` has no `insecure_plain_grpc`. The controller serves gRPC on 8444 and the HTTPS enrollment endpoint (`POST /enroll`, `POST /renew`) on 8445, both on the management address, and keeps its CA in `/var/lib/fps/pki`. The agent keeps its key (0600) and certificate in `/var/lib/fps-agent/pki`, in a state directory with mode 0700.

The enrollment token is a secret that is valid once. It never appears on a command line, in a unit file, or in a log. `fps-enroll-token` runs as root before the agent (`ExecStartPre=+`) and decides:

1. A token that cloud-init wrote to `/etc/fps/enroll.token` (mode 0600) moves to `/var/lib/fps-agent/enroll.token`, and the original is deleted. It replaces an existing certificate, which re-enrolls a revoked agent.
2. Otherwise, in the all-in-one role, when the agent has neither `agent.crt` nor a token file, the helper waits up to 30 seconds for the admin socket `/var/lib/fps/pki/admin.sock` and runs `fps-controller token -agent-id <id> -ttl 10m -config /etc/fps/controller.yaml` with umask 077 and standard output redirected into the token file. The agent enrolls, deletes the file, and a reboot does not enroll again.
3. Otherwise it does nothing. An agent role without a token or a certificate exits with code 2 (`no enrollment token is available`), which `RestartPreventExitStatus=2` leaves stopped.

`fps-agent-start` passes `--cert-dir /var/lib/fps-agent/pki --enroll-token-file /var/lib/fps-agent/enroll.token` for an `https://` controller, `--tls-ca /var/lib/fps/pki/ca.crt` (all-in-one) or `--tls-ca /etc/fps/controller-ca.pem` or `--tls-ca-sha256 $FPS_CA_SHA256` (agent role), and `--enroll-address` from `FPS_ENROLL_ADDRESS` or `<mgmt address>:8445` in the all-in-one role. An `http://` URI gets none of these flags.

The agent role takes its trust and token from the cloud-init data that `fps-role` reads: `/etc/fps/enroll.token` and `/etc/fps/controller-ca.pem` through `write_files`, and the `role.env` keys `FPS_CA_SHA256` and `FPS_ENROLL_ADDRESS`. `fps-role` refuses a token inside `role.env`, which every user can read. The operator mints the token on the controller with `fps-controller token -agent-id <id>`. The README of the image has the full example. The controller certificate names the management address, so an agent that dials a DNS name or another address needs `tls_names` in `controller.yaml`. `agent_auto_enroll` stays on, so that a restart of the controller, which empties its in-memory inventory, still admits the agents that hold valid certificates.

## Cloud-init defaults

`/etc/cloud/cloud.cfg.d/90-fps.cfg` lists the datasources NoCloud, ConfigDrive, and None, disables cloud-init's network configuration, and defines the default user `fps-admin` with key login only. The image ships no password. A VM without a seed has no login, so every deployment supplies a seed with an ssh key. `image/seed-example/` holds an example.

## Formats

`image/convert.sh` turns the qcow2 into the formats that other hypervisors and clouds import. IMG-03 added it. It reads `image/output/fps-base.qcow2` by default and writes next to it, or into `--out-dir`.

| Format | File | Target |
|---|---|---|
| raw | `fps-base.raw` | `dd`, KVM, cloud importers that take raw |
| VHDX | `fps-base.vhdx` | Hyper-V |
| fixed VHD | `fps-base.vhd` | Azure, which demands a size that is a multiple of 1 MiB plus the 512 byte footer |
| OVA | `fps-base.ova` | VMware, VirtualBox, and other OVF importers |

The script rounds the virtual size up to a multiple of 1 MiB, makes a raw disk of that size, and converts it with `qemu-img convert -O vpc -o subformat=fixed,force_size=on`. Without `force_size=on`, qemu-img rounds the size to a disk geometry and misses the 1 MiB rule. The script checks that the VHD size minus 512 is a multiple of 1 MiB and fails when it is not. The raw disk is deleted afterwards unless raw is in `--formats` or `--keep-raw` is set. When the output directory lacks the space for a format that the default list included, the script warns and skips it; a format named with `--formats` that does not fit is an error.

The OVA is a tar archive with the members in this order: `<name>.ovf`, the streamOptimized `<name>.vmdk`, and `<name>.mf`, which holds one `SHA256(<member>)= <hex>` line for the .ovf and the .vmdk. The OVF 1.0 descriptor names the CPUs (default 4), the memory (default 8192 MiB), one disk on a SCSI controller, and two E1000 NICs: the first is the management NIC that becomes `mgmt0`, the second a test NIC. `--cpus`, `--memory-mb`, and `--nics 1` change them. The agent role isolates CPUs only with at least 4 CPUs (see Limits). The script runs `qemu-img check` on the qcow2, the VHDX, and the VMDK, and `qemu-img info` on raw and VHD, which have no check. `SHA256SUMS` covers every output of the run.

## Tests

| Command | Level | Needs |
|---|---|---|
| `make lint-img` | Lint of the scripts with `bash -n`, ShellCheck, and `packer fmt -check` when installed | Nothing |
| `make test-img` | Static checks of the sources, including `nft -c`, and the offline tests of the role selector of the tuning sizes and reboot logic, and of the enrollment helper and the agent arguments | Nothing |
| `image/tests/convert-test.sh` (part of `make test-img`) | Converts a tiny qcow2 with `convert.sh` and checks the VHD rule, the OVA member order, the manifest, and the checksums. Prints SKIP without `qemu-img`, and fails when `FPS_REQUIRE_IMAGE_TOOLS=1` | `qemu-img` |
| `sudo make e2e-img` | Namespace test of the shipped rules: a router forwards without them, and drops traffic between `mgmt0` and `test0` with them | root, `ip`, `nft`, `curl`, `python3` |
| `image/boot-test.sh [--role controller\|agent\|all-in-one\|all]` | Boots the built qcow2 under QEMU/KVM with a management NIC and a test NIC, seeds the role, and prints PASS or FAIL per check. The agent role boots with 4 CPUs and 4 GiB and waits for its tuning reboot | KVM, a built image |
| `image/boot-test.sh --run` | The all-in-one boot test plus the short HTTP and DNS run of `image/tests/vm-run.sh` inside the VM (IMG-04) | KVM or `FPS_ALLOW_TCG=1`, a built image |

The boot test seeds the role through NoCloud and checks, per role, which units are active, enabled, and running as processes, and the applied tuning: `/proc/cmdline`, the isolated CPUs, the hugepage count, the agent pinning, the sysctls, and the notrack table. It also checks `GET /v1/capabilities` through a host port forward on the management NIC, the listeners of the controller, the services, the registration of the local agent over mutual TLS (state online), the absence of `insecure_plain_grpc`, the agent arguments, the sysctl values, and the namespace test with the real rules inside the VM.

## CI boot test

The workflow [`image.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/image.yml) (IMG-04) builds the image and boots it on every pull request and every push to `main` that touches `image/`, `engine/`, `controller/`, the `http1` or `dns` module, `Cargo.toml`, or `Cargo.lock`. A manual run (`workflow_dispatch`) has the input `force_tcg`, which tests the fallback.

The job runs on a GitHub hosted `ubuntu-24.04` runner and nowhere else. The repository is public, so the workflow never names a self hosted runner label, holds the read only token (`contents: read`), and uses no secret. The steps are

1. Choose the accelerator. GitHub hosted Linux runners expose `/dev/kvm`. The step adds the standard udev rule (`KERNEL=="kvm", GROUP="kvm", MODE="0666"`) and reloads it, so the runner user may write the device. The accelerator is `kvm` when `/dev/kvm` is writable, and `tcg` (software emulation) otherwise or when `force_tcg` is set.
2. Install the host packages and Packer (the runner image already has it, and the step installs it from the HashiCorp repository otherwise), the Rust toolchain with the musl target, and Go from `controller/go.mod`.
3. Restore the caches: cargo (`Swatinem/rust-cache`), Go modules (`actions/setup-go`), the Packer plugins (keyed by the hash of `image/fps.pkr.hcl`), and the Ubuntu cloud image (keyed by its SHA-256 from the release `SHA256SUMS`, which the build also passes as the pinned checksum).
4. Run `image/build.sh`, which builds the static controller and the static musl agent, and runs Packer. With TCG the build sets `FPS_ALLOW_TCG=1` and the Packer ssh timeout `FPS_SSH_TIMEOUT=60m`.
5. Run `image/boot-test.sh --role all-in-one --run`. It boots the image with two NICs and checks the all-in-one role (the 43 checks of the boot test), and then runs `image/tests/vm-run.sh` inside the VM. With TCG the boot wait grows and the run uses 10 sessions per second.
6. Write the job summary with `image/ci-summary.sh`, and upload the artifact `image-boot-test`.

The short run is a reduced variant of the [Gate 1](Gates.md) suite. The Gate 1 suite needs the namespace harness, two agents, and the realism tools, which the VM does not have. The run inside the VM uses what the image has. It calls the controller REST API on the management address with the admin token, and the agent `local`, which enrolled over mutual TLS and is the one agent of the run. It installs the `http1` and `dns` module trees that the agent loads, stores a test configuration with the Gate 1 mix entries (`http1/single_fetch` and `dns/lookup_a`) on loopback pools, runs it for 12 seconds (2 seconds of ramp and 10 of steady phase) at 50 sessions per second, and checks that the run is `completed`, that the verdict is `passed` (`sessions.failed_ratio <= 0.01`), and that both applications counted succeeded sessions. The environment tunes it: `FPS_RUN_RATE`, `FPS_RUN_STEADY_S`, and `FPS_RUN_WAIT_S`.

### The agent session gate (IMG-07)

The job failed twice on 2026-11-10 (jobs 129666780207 on the MOD-06 branch and 129934010691 on the MOD-22 branch) and passed on rerun with the same code. Both runs used KVM, the VM booted in about 10 s (cloud-init reports 9.5 s), and the VM journal and the host log agree to the second, so a slow TCG guest and a clock step at boot do not explain it. Both sides also time their silence with monotonic clocks, which a wall clock step does not move. The two journals show the cause:

| Time (job 129666780207) | Controller | Agent |
|---|---|---|
| 06:22:37.579 | `agent online`, session 1 | |
| 06:22:42.624 | `agent offline`, session 1, `heartbeat timeout` | `session lost`, session 1, `opened: false` |
| 06:22:42.768 | `agent online`, session 2 | |
| 06:22:48.624 | `agent offline`, session 2, `heartbeat timeout` | `session lost`, session 2, `opened: false` |
| 06:22:48.835 | `agent online`, session 3 | |
| 06:22:53.911 | the run sends `prepare` on session 3 (5.08 s old) | |
| 06:22:53.912 | `agent offline`, session 3, `stream closed` | `session open` and `session lost` in the same millisecond, `the controller stayed silent for the heartbeat timeout` |

The agent never opened sessions 1 and 2 (`opened: false`), although the controller listed it as `online` all along. The cause is a gap in the handshake between the two. `controller/internal/agents/service.go` reads the open message in `Session`, marks the agent online, and sends nothing more until it has a command or a heartbeat to acknowledge. Go's gRPC server sends the response headers with its first message, and the Rust agent (`engine/fps-agent/src/session.rs`, function `session`) waits for them in `client.session(...)` before it enters its loop and sends the first heartbeat. The fake controller of the agent tests is a tonic server, which sends the headers at once, so no test saw it. On an idle appliance every session is therefore silent: the controller's one second sweep ends it between 5 and 6 s after the open message (the 6.000 s period of the table), and the agent redials. The first command opens the session, because it is the first message. A command that arrives in the last second before the sweep finds the agent's watchdog already fired, and the agent drops the session in the same millisecond. That is the failure, 5.02 s and 5.08 s after the session began in the two jobs, and it hits a run that starts at a random phase of the 6 s cycle, so most runs pass.

The fix of the cause belongs to the controller, and the boot test cannot make a unit order or a timeout change it: the controller calls `stream.SendHeader(metadata.MD{})` right after `open` returns (a CTL change, with a test that uses a client which, like tonic, waits for the headers). The boot test does not hide the gap in the meantime. `image/tests/vm-run.sh` starts the first command only on a session that is proven fine: one that the agent opened and held for 3 s, or one that the controller registered at most 2 s ago and the agent has not opened, where the first command opens it long before the 5 s timeout. It decides from the two journals with the monotonic clock, waits up to 30 s, and fails when no such session appears, which is the case when the agent cannot reach the controller. `image/tests/session-gate-test.sh` replays the journals of the failed job: the gate refuses the moment at which the run failed and accepts the moments a second after each session began. When the controller sends the headers, sessions open at once and the gate returns `stable`.

Two more settings prove the acceptance of IMG-07. `FPS_TEST_REPEAT=10` boots the image ten times in a row and stops at the first failure. `FPS_TEST_NEGATIVE=agent-unreachable` boots once more with the agent pointed at the loopback, where no controller listens, and passes only when that boot fails at the agent checks while the controller API and ssh work. `image/tests/boot-loop-test.sh` tests both with a fake boot. The workflow stays a coordinator file, so the loop needs these two variables in the step that runs `image/boot-test.sh`, as the pull request of IMG-07 describes.

The artifact `image-boot-test` holds `boot-test.log` (every check and the result line), `serial-all-in-one.log` (the VM console), `vm-checks-all-in-one.out`, `vm-run.out`, `build.log`, `fps-base.qcow2.sha256`, and the directory `run/` with the test configuration, the run summary, the run report in JSON and CSV, and the application metrics. The job summary shows the accelerator, the result line, the failed checks, and the sessions per application.

## Containers

IMG-05 builds container images for Docker and Kubernetes labs. The directory [`image/containers/`](https://github.com/alrawi/FixingPointSystem/tree/main/image/containers) holds the sources, and its [README](https://github.com/alrawi/FixingPointSystem/blob/main/image/containers/README.md) gives the commands.

| Image | Build | Content |
|---|---|---|
| Controller | `Dockerfile.controller`, target `controller` | The static controller (Go, `CGO_ENABLED=0`, as in `image/build.sh`) on an empty root file system, user 10001, `/etc/fps/controller.yaml`, state in `/var/lib/fps` |
| Agent | `Dockerfile.agent` | The static musl agent (`x86_64-unknown-linux-musl`, as in `image/build.sh`) on an empty root file system, user 10002, the `http1` and `dns` module trees, state in `/var/lib/fps-agent` |
| Lab tools | `Dockerfile.controller`, target `tools` | A shell, curl, jq, the controller binary, the module trees, and the three lab scripts. It is for the compose lab only |

Both build stages are multi stage: a pinned `golang` or `rust` image compiles the binary, and the final stage is `scratch`, so the images hold no shell, no package manager, and no libc. Every base image is pinned by tag and digest, and `image/containers/static-check.sh` fails on one that is not. The CI runs Hadolint on both Dockerfiles.

The compose file `image/containers/compose.yaml` runs an all in one lab: one controller and two agents on a bridge network. The steps run in order. `prep` writes the admin API token into a volume. The controller starts and creates its CA and its admin socket. `enroll` mints one enrollment token per agent through the admin socket (`fps-controller token`) and copies the public CA certificate into a volume. The agents start, enroll over HTTPS on 8445, and register over mutual TLS on 8444, as the appliance does ([Mutual TLS and enrollment](#mutual-tls-and-enrollment)). The lab never sets `insecure_plain_grpc`, and `agent_auto_enroll` is false, so only identities that carry a minted token join.

No token is an argument or an environment variable, so none shows in `docker inspect`. Each token is a file in a volume: the admin token in `secrets` (mode 0600, user 10001), and the enrollment token of an agent in a volume that only that agent mounts for writing (mode 0640, group of the agents, deleted by the agent after use). The volume with the CA key and the admin socket is mounted by the controller and the `enroll` step only. The admin socket has mode 0600 and belongs to the controller user, so `enroll` runs as that user. After the test, `run-lab.sh` reads the admin token from its volume and checks that it appears in no `docker inspect` output and in no log.

Every container drops all capabilities, sets `no-new-privileges`, and has a read only root file system. The agent needs no capability in the lab, because the test pools are `/30` networks of one host each, whose host is the address of the container (the agent numbers a pool from network plus one, so the containers sit at addresses that are 1 modulo 4), the scenarios listen above port 1024, and the lab starts no capture. The README lists each capability that a lab with a real test interface may need, with its reason: `NET_ADMIN` for interface and address setup, `NET_RAW` for capture, `NET_BIND_SERVICE` for ports below 1024, and `IPC_LOCK` for hugepage mappings (the engine maps none yet).

The test (`lab/run-test.sh`) follows the [Gate 1](Gates.md) suite: one test configuration, stored once per agent, runs as two runs, the server run on agent2 (`--side server`) and the client run on agent1 (`--side client`), because the compiler gives an agent the host pools of its own ports only. The mix is `http1/single_fetch` on TCP port 18080 and `dns/lookup_a` on UDP port 15353. The run passes when the client run is `completed` with the verdict `passed` (`sessions.failed_ratio <= 0.01`) and when both agents counted succeeded sessions of both applications on their own side, so the traffic crossed from one container to the other.

The workflow [`containers.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/containers.yml) runs on every pull request and every push to `main` that touches `image/containers/`, `engine/`, `controller/`, the `http1` or `dns` module, `Cargo.toml`, or `Cargo.lock`. It runs on a GitHub hosted `ubuntu-24.04` runner only, holds the read only token (`contents: read`), uses no secret, and never logs in to a registry or pushes an image. The steps are the static checks, Hadolint, and `image/containers/run-lab.sh`, which builds the three images, starts the lab, runs the test, checks for leaked tokens, and writes the logs. The job summary shows the result lines, and the artifact `container-lab` holds `compose.log`, one log per service, `inspect.json`, and `results/` with the test configurations, the run summaries, the report, and the metrics.

## Limits

1. The REST API on 8443 is plain HTTP with a bearer token, so the management network must be trusted for the API. The control channel to the agents (8444) and the enrollment (8445) use TLS.
2. `agent_auto_enroll` is on in every role, so the controller role accepts agents from the management network without enrollment. The role selector does not change it, and the enrollment packages (CTL-27 and AGT-08) decide how it narrows.
3. The agent runs as root, because the host setup moves interfaces into namespaces. The role selector does not narrow this.
4. The DHCP address of `mgmt0` is read once at start. A changed lease needs a restart of `fps-mgmt-addr.service` and the two services.
5. The agent role isolates CPUs only on a VM with at least 4 CPUs. Hugepages stay reserved for the life of the VM, so an agent VM with little memory loses a quarter of it to them.
6. The container images build for x86_64 only. The compose lab runs on one Docker host and one bridge network, so it shows the control and the application path and not the throughput of a real test interface. A host pool of the lab holds one host, because the pool address must be the address of the container.
