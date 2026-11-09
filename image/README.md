# FixingPointSystem appliance image

This directory builds the qcow2 appliance of work package IMG-01 and holds the role selector and the tuning profiles of IMG-02. The image is the official Ubuntu 24.04 LTS cloud image plus the controller, the agent, their systemd units, cloud-init defaults, and nftables rules that keep the management interface apart from the test interfaces. The wiki page [Image](../docs/wiki/Image.md) explains the design.

## What the image contains

| Part | Where | Notes |
|---|---|---|
| Controller | `/usr/bin/fps-controller`, `fps-controller.service` | Runs as user `fps`. Listens on the management address only: REST on 8443, agent gRPC with mutual TLS on 8444, agent enrollment (HTTPS) on 8445 |
| Agent | `/usr/bin/fps-agent`, `fps-agent.service` | Runs as root, dials the controller on the management address with agent id `local` |
| Modules | `/usr/share/fps/modules/{http1,dns}` | The module trees that the agent loads |
| Management interface | `fps-mgmt-net.service`, `/etc/systemd/network/10-fps-mgmt.network` | The first NIC (lowest PCI address) becomes `mgmt0` and takes DHCP. Every other NIC is a test NIC without any address |
| Isolation | `fps-firewall.service`, `/etc/fps/isolation.nft`, `/etc/sysctl.d/99-fps-isolation.conf` | `ip_forward` off, and nftables drops everything forwarded into or out of `mgmt0` and every connection to ports 22, 8443, and 8444 that arrives on another interface |
| Role selector | `fps-role.service`, `/etc/fps/role.env` | cloud-init user-data picks `controller`, `agent`, or `all-in-one` (the default). Only the units of the role start. See [Roles and tuning](#roles-and-tuning) |
| Tuning | `fps-tune.service`, `/etc/fps/tune.applied` | Hugepages, CPU isolation, sysctls, and the loopback notrack table per role |
| cloud-init | `/etc/cloud/cloud.cfg.d/90-fps.cfg` | NoCloud (a disk labeled `cidata`), ConfigDrive, or no datasource. Default user `fps-admin`, key login only, no password anywhere |

The agent needs Linux 6.3 or newer (decision D12 item 3). Ubuntu 24.04 ships Linux 6.8, and the build fails when the kernel of the image is older than 6.3.

## Prerequisites

Any x86_64 Linux host with KVM works. Ubuntu 24.04 and 26.04 are the tested or expected hosts, and the package names below are those of Ubuntu. The host's C library does not matter, because `build.sh` builds the agent as a static binary for `x86_64-unknown-linux-musl` and the controller with `CGO_ENABLED=0`, and checks with `file` that both are statically linked. The image itself is Ubuntu 24.04. The Rust musl target needs `rustup`, because distro `cargo` cannot add it. `shellcheck` is optional and only `make lint-img` uses it.

1. Install the packages.

   ```bash
   sudo apt-get update
   sudo apt-get install -y qemu-system-x86 qemu-utils cloud-image-utils xorriso \
     curl openssh-client openssl python3 build-essential git gnupg file musl-tools
   ```

2. Install Packer 1.9 or newer from the HashiCorp repository, unless `packer version` already works.

   ```bash
   curl -fsSL https://apt.releases.hashicorp.com/gpg | sudo gpg --dearmor -o /usr/share/keyrings/hashicorp-archive-keyring.gpg
   echo "deb [signed-by=/usr/share/keyrings/hashicorp-archive-keyring.gpg] https://apt.releases.hashicorp.com $(lsb_release -cs) main" | sudo tee /etc/apt/sources.list.d/hashicorp.list
   sudo apt-get update && sudo apt-get install -y packer
   packer version
   ```

3. Install Rust with rustup, not with the distro cargo package, and add the musl target. `build.sh` needs `rustup target list --installed` to show `x86_64-unknown-linux-musl`, and `musl-gcc` (from `musl-tools`, installed in step 1) for dependencies that compile C code.

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   . "$HOME/.cargo/env"
   rustup target add x86_64-unknown-linux-musl
   rustup target list --installed
   ```

4. Give your user access to KVM.

   ```bash
   sudo usermod -aG kvm "$USER"
   ```

   Then log out and back in, and check with `id` that the output lists `kvm`, and with `test -w /dev/kvm && echo "kvm ok"`. If `usermod` is not on the path of `sudo`, use `sudo gpasswd -a "$USER" kvm` instead. To skip the log out for one shell, run the commands under `sg kvm -c '...'`.

5. Optional: install ShellCheck (`sudo apt-get install -y shellcheck`) for `make lint-img`. Without it that target skips the ShellCheck run.

Go needs no installation. `build.sh` uses your `go` when it is at least the version of `controller/go.mod`, and otherwise downloads a pinned Go toolchain (default 1.24.7, checksum checked against the go.dev download index) into `image/.cache/go`.

## Build the image

Run from the repository root, on the branch that you want to test.

```bash
git fetch origin wp/IMG-01 && git checkout wp/IMG-01
image/build.sh
```

The script runs seven steps and prints a heading `== ...` for each: check the prerequisites (including the musl target), find or install Go, build the controller, build the agent, stage the modules, run Packer (`packer init`, `packer validate`, `packer build`), and check the result with `qemu-img check`. The first run downloads the Ubuntu cloud image (about 600 MB), the Packer QEMU plugin, the Rust dependencies, and the Go modules, and it compiles the agent, so expect 10 to 25 minutes. A rerun reuses the caches in `image/.cache`.

On success the last lines are

```
PASS image built: .../image/output/fps-base.qcow2
```

and `image/output/fps-base.qcow2.sha256` holds the checksum. On any problem the script prints a line that starts with `FAIL` and exits with a nonzero code. Set `FPS_PACKER_LOG=1` to see the Packer debug log. Without KVM, `FPS_ALLOW_TCG=1` runs the build under software emulation, which is slow.

## Roles and tuning

A VM runs one of three roles. The seed chooses it with a file `/etc/fps/role.env`, which `write_files` in the NoCloud user-data writes. Without the file the role is `all-in-one`, which is the behavior of the first image.

```yaml
#cloud-config
ssh_authorized_keys:
  - ssh-ed25519 AAAA... you@example
write_files:
  - path: /etc/fps/role.env
    permissions: "0644"
    content: |
      FPS_ROLE=agent
      FPS_CONTROLLER_URI=https://192.0.2.10:8444
      FPS_CA_SHA256=<64 hex digits, the ca_sha256 that the controller logs>
  # The token that `fps-controller token -agent-id <id>` printed on the controller.
  - path: /etc/fps/enroll.token
    owner: root:root
    permissions: "0600"
    content: <the token>
```

| Role | Services | Tuning |
|---|---|---|
| `all-in-one` (default) | `fps-controller`, `fps-agent` (agent id `local`) | The agent sysctls and the loopback notrack table. No kernel option, no reboot |
| `controller` | `fps-controller` only | None |
| `agent` | `fps-agent` only, dialing `FPS_CONTROLLER_URI` (required) | CPU isolation, hugepages, the agent sysctls, the notrack table, and one reboot on the first boot |

The keys of `role.env` are `FPS_ROLE`, `FPS_CONTROLLER_URI`, and optionally `FPS_AGENT_ID` (default: the host name in the agent role), `FPS_AGENT_LABELS` (default `role=agent`), and `FPS_AGENT_EXTRA_ARGS`, plus the two keys of mutual TLS below. The script reads them with a parser and never sources the file. An invalid file fails `fps-role.service` and starts neither service.

How it works at every boot: `fps-role.service` (after cloud-init) writes a marker `/run/fps/role.d/<unit>` for each unit of the role, enables those units and disables the others, and records the result in `/etc/fps/role.applied`. The two service units hold `ConditionPathExists` on their marker, so a unit outside the role stays inactive even before the enable and disable take effect. `fps-tune.service` then applies the profile of the role.

The agent profile is sized from the VM, and `/usr/libexec/fps/fps-tune plan ROLE CPUS MEM_MIB` prints it without changing anything.

| Setting | Rule | 4 CPUs, 4 GiB | 16 CPUs, 64 GiB |
|---|---|---|---|
| Housekeeping CPUs | `CPUS/4` rounded down, at least 1, from CPU 0. Fewer than 4 CPUs: no isolation | `0` | `0-3` |
| Isolated CPUs | The rest: `isolcpus`, `nohz_full`, and `rcu_nocbs` on the kernel command line, and `taskset` pins the agent to them | `1-3` | `4-15` |
| Hugepages | 25 percent of the memory (rounded to 256 MiB) as 2 MiB pages, reserved at boot with `hugepages=N`. Under 1 GiB: none | 512 | 8192 |

Sysctls of the agent and all-in-one roles, in `/etc/sysctl.d/90-fps-agent.conf`: `net.ipv4.ip_local_port_range` 1024 65535, `net.core.somaxconn` 65535, `net.ipv4.tcp_max_syn_backlog` and `net.core.netdev_max_backlog` 65535, `net.ipv4.tcp_tw_reuse` 1, and `fs.file-max` and `fs.nr_open` 4194304. The agent unit has `LimitNOFILE=4194304`. The loopback skips connection tracking in the table `inet fps_notrack` (`/etc/fps/notrack.nft`), loaded only when `nf_conntrack` is loaded, because a full conntrack table drops test traffic on the benchmark host (see `bench/README.md`), and a notrack rule would load the module otherwise.

The kernel options go into `/etc/default/grub.d/60-fps-tune.cfg`, and `update-grub` runs when that file changes. They need one reboot, which `fps-tune` requests with `systemctl reboot` and which the VM does by itself during its first boot. The step is idempotent: the script compares the options on `/proc/cmdline` with the profile, so a VM that already runs them does nothing. It asks for a reboot once for each profile. If the options are still missing after that reboot, it records `status=failed` and does not reboot again, and `fps-tune.service` shows as failed. Delete `/etc/fps/tune.applied` to try again. While the reboot is pending, `/run/fps/tune-reboot` exists and the agent does not start. A change of role in `role.env` on a running VM takes effect at the next boot, and a switch away from the agent role removes the options with one more reboot.

`/etc/fps/tune.applied` records what happened: `role`, `status` (`none`, `active`, `reboot-pending`, or `failed`), the CPU and memory sizes, the CPU lists, `hugepages_wanted` and `hugepages_allocated`, the options, `sysctl`, `notrack`, and the time. Restart `fps-tune.service` after a firewall or container tool loads `nf_conntrack` later, so that the notrack table follows.

Test the logic without a VM with `make test-img`, which runs `image/tests/role-test.sh` and `image/tests/tune-test.sh`.

### Mutual TLS and enrollment

The controller and its agents talk over mutual TLS (IMG-06). The controller keeps its CA in `/var/lib/fps/pki`, serves gRPC on 8444 and the HTTPS enrollment endpoint on 8445, both on the management address, and `/etc/fps/controller.yaml` has no `insecure_plain_grpc`. An agent holds a certificate that the CA signed after the agent showed a one time token. The key and the certificate live in `/var/lib/fps-agent/pki` (mode 0700), and the agent renews the certificate on its own.

- **all-in-one.** At the start of `fps-agent.service`, `/usr/libexec/fps/fps-enroll-token` checks the agent state. When the agent has neither a certificate nor a token file, it waits up to 30 seconds for the admin socket `/var/lib/fps/pki/admin.sock` and runs `fps-controller token -agent-id <id> -ttl 10m` as root, with the output redirected into `/var/lib/fps-agent/enroll.token` (umask 077). The agent trusts `/var/lib/fps/pki/ca.crt`, enrolls at `<mgmt address>:8445`, and deletes the token file, so a reboot does not enroll again. Nothing to configure.
- **agent.** The seed carries the token and the trust anchor of the controller. Mint the token on the controller VM: `sudo fps-controller token -agent-id <id> -ttl 30m -config /etc/fps/controller.yaml`. It prints the token alone on standard output (the expiry goes to standard error), and the token is valid once. Read the CA from `/var/lib/fps/pki/ca.crt` on the controller or use its fingerprint from the line `certificate authority open` in the controller log (`ca_sha256`).

| Seed data | Where | Meaning |
|---|---|---|
| `/etc/fps/enroll.token` | `write_files`, mode 0600 | The one time token. `fps-enroll-token` moves it to `/var/lib/fps-agent/enroll.token` and removes the original. A token that exists replaces a valid certificate, which re-enrolls a revoked agent |
| `/etc/fps/controller-ca.pem` | `write_files`, mode 0644 | The CA certificate of the controller, passed as `--tls-ca` |
| `FPS_CA_SHA256` | `role.env` | The SHA-256 fingerprint of the CA, passed as `--tls-ca-sha256`, used when no CA file is present. The first exchange then has the weaker trust that [Agent](../docs/wiki/Agent.md#trust) describes |
| `FPS_ENROLL_ADDRESS` | `role.env`, optional | `host:port` of the enrollment endpoint. The default is the controller host on port 8445 |

A token is never written into `role.env` (every user can read it, and `fps-role` refuses a `FPS_ENROLL_TOKEN` key), never given on a command line, and never put in a unit file or log. A controller URI with a DNS name or another address than the management address needs `tls_names` in `/etc/fps/controller.yaml`, because the controller certificate names the management address only. A bad CA or a refused token makes the agent exit with code 2, and systemd does not restart it: write a new token and run `systemctl restart fps-agent`. An `http://` controller URI still works against a controller in its plain development mode and gets no TLS flags.

## Boot test

```bash
image/boot-test.sh                   # the all-in-one role, the default
image/boot-test.sh --role controller
image/boot-test.sh --role agent      # boots with 4 CPUs and 4 GiB and waits for the tuning reboot
image/boot-test.sh --role all        # the three roles, one after the other
```

The script boots a throwaway overlay of `image/output/fps-base.qcow2` under QEMU/KVM with two NICs. The first is the management NIC on user networking, with host port forwards `127.0.0.1:18443` to the controller API and `127.0.0.1:12222` to ssh. The second is a test NIC on a separate user network without outside access. A generated NoCloud seed supplies an ssh key for `fps-admin` and a known API token. The script waits up to 300 seconds for the API, then runs the checks, kills the VM, and deletes its temporary files. The image file stays unchanged. The `--role` option seeds the role through the NoCloud data (see [Roles and tuning](#roles-and-tuning)), and the checks that follow depend on it. The agent role needs about twice the time, because the VM reboots once by itself to apply the kernel options, and the test waits until `fps-tune` records `status=active`. Change the CPUs and memory with `FPS_TEST_SMP` and `FPS_TEST_MEM`, the ports with `FPS_TEST_API_PORT` and `FPS_TEST_SSH_PORT` when they are in use, and the wait with `FPS_TEST_BOOT_WAIT`.

Each check prints `PASS <what>` or `FAIL <what>`, and the last line is `RESULT: PASS (N checks)` with exit code 0, or `RESULT: FAIL (...)` with exit code 1. On a failure the script also prints the last 80 lines of the VM serial console. The checks are

| Check | Proves |
|---|---|
| `GET /v1/capabilities` on the management interface answers 200 with a JSON object | The acceptance item of IMG-01, through the port forward on the management NIC |
| `GET /v1/capabilities` without a token answers 401 | The token check is on |
| ssh as `fps-admin` with the seed key works | cloud-init NoCloud applied the seed |
| Kernel is Linux 6.3 or newer | The AGT-06 requirement holds |
| `mgmt0` has an IPv4 address, and the test NIC has none | The first NIC is the management interface and the other stays bare |
| Ports 8443, 8444, and 8445 listen on the management address only | The controller does not listen on the loopback, a wildcard, or a test NIC |
| The loopback does not answer on 8443, and the management address answers `/healthz` | Same, from inside |
| `fps-controller.service` and `fps-agent.service` are active, and the local agent is online (it registered over mutual TLS) | Both services start and talk over the management interface |
| `net.ipv4.ip_forward` and `net.ipv6.conf.all.forwarding` are 0 | Forwarding is off |
| Table `inet fps_isolation` is loaded and `fps-firewall.service` is active | The rules apply at boot |
| Namespace test: the router forwards without rules (control), and after the shipped rules load, pings and port 8443 between `mgmt0` and a test interface are dropped, while the management side still answers | The shipped `/etc/fps/isolation.nft` blocks routing between the management and test interfaces |

The table above is the all-in-one role. Its checks run in every role, except that the controller role has no agent checks and the agent role has no controller checks. The checks that depend on the role are

| Role | Extra checks |
|---|---|
| all-in-one | Both services run with a process. `role.applied` and `tune.applied` name the role. No kernel option of the profile on `/proc/cmdline`, no grub drop-in, status `none`, and the agent sysctls hold |
| controller | `fps-controller` runs. `fps-agent` is inactive, disabled, and has no process. Nothing of the tuning is present: no kernel option, no hugepages, no sysctl file, the default `somaxconn`, no notrack table |
| agent | `fps-agent` runs and `fps-controller` is inactive, disabled, and has no process. Nothing listens on 8443, 8444, or 8445, and the forwarded API port answers nothing. `/proc/cmdline` holds the options of `fps-tune plan`, `/sys/devices/system/cpu/isolated` and `nohz_full` show `1-3`, `nr_hugepages` and `HugePages_Total` show 512, the agent is pinned to `1-3`, the sysctls and the agent file limit hold, the notrack table exists when `nf_conntrack` is loaded, and a second `fps-tune apply` changes nothing and does not reboot |

Mutual TLS adds these checks (IMG-06). In the controller and all-in-one roles, `controller.yaml` has no `insecure_plain_grpc` and the controller log has no `INSECURE` line. In the agent and all-in-one roles, the agent command line has `--controller https://`, `--cert-dir`, and a trust anchor, and holds no token. In the all-in-one role, `agent.crt` exists, the token file is gone, the key has mode 0600, and `GET /v1/agents` shows `local` with the state `online`.

The agent role has no controller in its VM. With `FPS_TEST_CONTROLLER_BIN=/path/to/fps-controller image/boot-test.sh --role agent`, the script runs that controller on the host (ports 18444, 18445, and 18446 on the loopback, reached from the VM as 10.0.2.2), mints a token into a file with `fps-controller token`, seeds the token and the CA, and checks that `GET /v1/agents` on the host shows `edge-1` online and that the controller log shows an enrollment and no `INSECURE`. Without the variable, the seed carries a placeholder token and a fingerprint, and the check proves only that the agent runs against `https://` and keeps redialing the absent controller.

The namespace test is `image/tests/isolation-netns.sh`. It builds a router with interfaces named `mgmt0` and `test0` in network namespaces inside the VM, and it can also run on any Linux host as root: `sudo image/tests/isolation-netns.sh`.

## Run the image by hand

```bash
cd image/seed-example
# Put your own public key into user-data first.
cloud-localds ../seed.img user-data meta-data
cd ..
qemu-system-x86_64 -machine q35,accel=kvm -cpu host -smp 4 -m 8192 -nographic \
  -drive file=output/fps-base.qcow2,if=virtio -drive file=seed.img,if=virtio,format=raw \
  -netdev user,id=mgmt,hostfwd=tcp:127.0.0.1:8443-:8443,hostfwd=tcp:127.0.0.1:2222-:22 \
  -device virtio-net-pci,netdev=mgmt \
  -netdev user,id=t1,restrict=on -device virtio-net-pci,netdev=t1 \
  -netdev user,id=t2,restrict=on -device virtio-net-pci,netdev=t2
ssh -p 2222 fps-admin@127.0.0.1 sudo cat /etc/fps/admin.token
```

Without a seed the VM has no login, because the image ships no password. The first NIC on the command line is the management NIC. To choose another NIC, set `FPS_MGMT_MAC` in `/etc/fps/image.env`.

## Layout

| Path | Purpose |
|---|---|
| `fps.pkr.hcl` | The Packer template with the QEMU builder |
| `build.sh`, `boot-test.sh` | The build and the boot test |
| `files/` | Files that land in the image, laid out as `/` |
| `final/` | Files that the last build step installs, laid out as `/` |
| `scripts/provision.sh`, `scripts/cleanup.sh` | The steps that run inside the build VM |
| `cloud-init/build-user-data.tftpl` | Seed of the build VM, with a random one time password |
| `seed-example/` | Example NoCloud seed for a VM |
| `tests/` | The static checks, the offline tests of the role selector and the tuning (`role-test.sh`, `tune-test.sh`), the namespace test, and the checks that run inside the VM |

Run the checks that need no KVM with `make lint-img test-img`, and the namespace test with `sudo make e2e-img`. `nft -c` needs root on many hosts, so `make test-img` prints a NOTE and skips that one check without root. Run `sudo make test-img` to include it.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `packer init` cannot download the plugin | The host needs outbound HTTPS to github.com |
| `FAIL /dev/kvm is not writable` | Run `sudo usermod -aG kvm "$USER"` (or `sudo gpasswd -a "$USER" kvm`), log out and back in, and check with `id` |
| `FAIL the Rust target x86_64-unknown-linux-musl is not installed` | `rustup target add x86_64-unknown-linux-musl` |
| `FAIL musl-gcc is missing` | `sudo apt install musl-tools` |
| `FAIL rustup is missing` | Install rustup (Prerequisites, step 3). Distro cargo cannot add the musl target |
| Packer waits for ssh until the timeout | Run with `FPS_PACKER_LOG=1` and set `headless = false` in `fps.pkr.hcl` locally to watch the console |
| `qemu-system-x86_64: Could not set up host forwarding rule` | Another process uses the port. Set `FPS_TEST_API_PORT` or `FPS_TEST_SSH_PORT` |
| `make lint-img` prints `shellcheck is not installed, skipped` | Optional: `sudo apt-get install -y shellcheck` |
| `nft -c` is skipped in `make test-img` | It needs root. Run `sudo make test-img` |
