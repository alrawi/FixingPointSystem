# Image

This page describes the appliance image of the IMG workstream. IMG-01 builds the qcow2 base image with Packer. The directory [`image/`](https://github.com/alrawi/FixingPointSystem/tree/main/image) holds the sources, and its [README](https://github.com/alrawi/FixingPointSystem/blob/main/image/README.md) gives the exact build and boot test steps. Later packages add the role selector (IMG-02), format conversion (IMG-03), and the CI boot test (IMG-04).

## Build

`image/build.sh` builds the controller with Go, builds the agent with `cargo build --release`, stages the modules `http1` and `dns`, and runs Packer on `image/fps.pkr.hcl`. The Packer QEMU builder boots the official Ubuntu 24.04 LTS cloud image (`disk_image = true`) with a throwaway ssh user from a NoCloud seed that Packer renders. Three steps follow inside the VM.

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
| `fps-controller.service` | Runs `fps-controller` as user `fps` with `-listen ${FPS_MGMT_ADDR}:8443 -grpc-listen ${FPS_MGMT_ADDR}:8444` |
| `fps-agent.service` | Runs `fps-agent connect` against `http://${FPS_MGMT_ADDR}:8444` with agent id `local` |

The controller reads its token from `/etc/fps/admin.token`. A cloud-init `write_files` entry may provide the file, and otherwise `fps-init-token` creates a random token on the first start. The agent restarts after a crash or after it gave up redialing (exit code 3), and it does not restart after exit code 2, which a usage error or a kernel before Linux 6.3 causes.

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
| `make test-img` | Static checks of the sources, including `nft -c` | Nothing |
| `sudo make e2e-img` | Namespace test of the shipped rules: a router forwards without them, and drops traffic between `mgmt0` and `test0` with them | root, `ip`, `nft`, `curl`, `python3` |
| `image/boot-test.sh` | Boots the built qcow2 under QEMU/KVM with a management NIC and a test NIC and prints PASS or FAIL per check | KVM, a built image |

The boot test checks `GET /v1/capabilities` through a host port forward on the management NIC, the listeners of the controller, the services, the registration of the local agent, the sysctl values, and the namespace test with the real rules inside the VM.

## Limits

1. The controller serves plain HTTP and plain gRPC until the TLS and mutual TLS packages land, so the management network must be trusted.
2. `agent_auto_enroll` is on, because the only agent is the one on the same VM. The role selector of IMG-02 can set it per role.
3. The agent runs as root, because the host setup moves interfaces into namespaces. IMG-02 narrows this with the role selector.
4. The DHCP address of `mgmt0` is read once at start. A changed lease needs a restart of `fps-mgmt-addr.service` and the two services.
