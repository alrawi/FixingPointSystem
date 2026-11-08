# FixingPointSystem appliance image

This directory builds the qcow2 appliance of work package IMG-01. The image is the official Ubuntu 24.04 LTS cloud image plus the controller, the agent, their systemd units, cloud-init defaults, and nftables rules that keep the management interface apart from the test interfaces. The wiki page [Image](../docs/wiki/Image.md) explains the design.

## What the image contains

| Part | Where | Notes |
|---|---|---|
| Controller | `/usr/bin/fps-controller`, `fps-controller.service` | Runs as user `fps`. Listens on the management address only: REST on 8443, agent gRPC on 8444 |
| Agent | `/usr/bin/fps-agent`, `fps-agent.service` | Runs as root, dials the controller on the management address with agent id `local` |
| Modules | `/usr/share/fps/modules/{http1,dns}` | The module trees that the agent loads |
| Management interface | `fps-mgmt-net.service`, `/etc/systemd/network/10-fps-mgmt.network` | The first NIC (lowest PCI address) becomes `mgmt0` and takes DHCP. Every other NIC is a test NIC without any address |
| Isolation | `fps-firewall.service`, `/etc/fps/isolation.nft`, `/etc/sysctl.d/99-fps-isolation.conf` | `ip_forward` off, and nftables drops everything forwarded into or out of `mgmt0` and every connection to ports 22, 8443, and 8444 that arrives on another interface |
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

## Boot test

```bash
image/boot-test.sh
```

The script boots a throwaway overlay of `image/output/fps-base.qcow2` under QEMU/KVM with two NICs. The first is the management NIC on user networking, with host port forwards `127.0.0.1:18443` to the controller API and `127.0.0.1:12222` to ssh. The second is a test NIC on a separate user network without outside access. A generated NoCloud seed supplies an ssh key for `fps-admin` and a known API token. The script waits up to 300 seconds for the API, then runs the checks, kills the VM, and deletes its temporary files. The image file stays unchanged. Change the ports with `FPS_TEST_API_PORT` and `FPS_TEST_SSH_PORT` when they are in use, and the wait with `FPS_TEST_BOOT_WAIT`.

Each check prints `PASS <what>` or `FAIL <what>`, and the last line is `RESULT: PASS (N checks)` with exit code 0, or `RESULT: FAIL (...)` with exit code 1. On a failure the script also prints the last 80 lines of the VM serial console. The checks are

| Check | Proves |
|---|---|
| `GET /v1/capabilities` on the management interface answers 200 with a JSON object | The acceptance item of IMG-01, through the port forward on the management NIC |
| `GET /v1/capabilities` without a token answers 401 | The token check is on |
| ssh as `fps-admin` with the seed key works | cloud-init NoCloud applied the seed |
| Kernel is Linux 6.3 or newer | The AGT-06 requirement holds |
| `mgmt0` has an IPv4 address, and the test NIC has none | The first NIC is the management interface and the other stays bare |
| Ports 8443 and 8444 listen on the management address only | The controller does not listen on the loopback, a wildcard, or a test NIC |
| The loopback does not answer on 8443, and the management address answers `/healthz` | Same, from inside |
| `fps-controller.service` and `fps-agent.service` are active, and the local agent registered | Both services start and talk over the management interface |
| `net.ipv4.ip_forward` and `net.ipv6.conf.all.forwarding` are 0 | Forwarding is off |
| Table `inet fps_isolation` is loaded and `fps-firewall.service` is active | The rules apply at boot |
| Namespace test: the router forwards without rules (control), and after the shipped rules load, pings and port 8443 between `mgmt0` and a test interface are dropped, while the management side still answers | The shipped `/etc/fps/isolation.nft` blocks routing between the management and test interfaces |

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
| `tests/` | The static checks, the namespace test, and the checks that run inside the VM |

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
