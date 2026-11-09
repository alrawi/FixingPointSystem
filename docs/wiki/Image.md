# Image

This page describes the appliance image of the IMG workstream. IMG-01 builds the qcow2 base image with Packer. The directory [`image/`](https://github.com/alrawi/FixingPointSystem/tree/main/image) holds the sources, and its [README](https://github.com/alrawi/FixingPointSystem/blob/main/image/README.md) gives the exact build and boot test steps. IMG-02 adds the role selector and the tuning profiles. Later packages add format conversion (IMG-03) and the CI boot test (IMG-04).

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
| `fps-controller.service` | Runs `fps-controller` as user `fps` with `-listen ${FPS_MGMT_ADDR}:8443 -grpc-listen ${FPS_MGMT_ADDR}:8444` |
| `fps-agent.service` | Runs `fps-agent connect` against `http://${FPS_MGMT_ADDR}:8444` with agent id `local` |

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
| nftables input hook | Connections to ports 22, 8443, and 8444 drop on every interface except `mgmt0` and the loopback, because Linux answers for a local address on any interface |

Both hooks run at priority -100 in the table `inet fps_isolation`. The rules apply to the namespace of the host. The kernel test ports that [Hosts](Hosts.md) moves into `fps-port-*` namespaces never share a routing table with `mgmt0`.

## Cloud-init defaults

`/etc/cloud/cloud.cfg.d/90-fps.cfg` lists the datasources NoCloud, ConfigDrive, and None, disables cloud-init's network configuration, and defines the default user `fps-admin` with key login only. The image ships no password. A VM without a seed has no login, so every deployment supplies a seed with an ssh key. `image/seed-example/` holds an example.

## Tests

| Command | Level | Needs |
|---|---|---|
| `make lint-img` | Lint of the scripts with `bash -n`, ShellCheck, and `packer fmt -check` when installed | Nothing |
| `make test-img` | Static checks of the sources, including `nft -c`, and the offline tests of the role selector and of the tuning sizes and reboot logic | Nothing |
| `sudo make e2e-img` | Namespace test of the shipped rules: a router forwards without them, and drops traffic between `mgmt0` and `test0` with them | root, `ip`, `nft`, `curl`, `python3` |
| `image/boot-test.sh [--role controller\|agent\|all-in-one\|all]` | Boots the built qcow2 under QEMU/KVM with a management NIC and a test NIC, seeds the role, and prints PASS or FAIL per check. The agent role boots with 4 CPUs and 4 GiB and waits for its tuning reboot | KVM, a built image |

The boot test seeds the role through NoCloud and checks, per role, which units are active, enabled, and running as processes, and the applied tuning: `/proc/cmdline`, the isolated CPUs, the hugepage count, the agent pinning, the sysctls, and the notrack table. It also checks `GET /v1/capabilities` through a host port forward on the management NIC, the listeners of the controller, the services, the registration of the local agent, the sysctl values, and the namespace test with the real rules inside the VM.

## Limits

1. The controller serves plain HTTP and plain gRPC until the TLS and mutual TLS packages land, so the management network must be trusted.
2. `agent_auto_enroll` is on in every role, so the controller role accepts agents from the management network without enrollment. The role selector does not change it, and the enrollment packages (CTL-27 and AGT-08) decide how it narrows.
3. The agent runs as root, because the host setup moves interfaces into namespaces. The role selector does not narrow this.
4. The DHCP address of `mgmt0` is read once at start. A changed lease needs a restart of `fps-mgmt-addr.service` and the two services.
5. The agent role isolates CPUs only on a VM with at least 4 CPUs. Hugepages stay reserved for the life of the VM, so an agent VM with little memory loses a quarter of it to them.
