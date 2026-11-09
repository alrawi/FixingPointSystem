#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Runs inside the build VM as root, from the Packer shell provisioner. It expects
# the staged tree in /tmp/fps-stage:
#   fps-controller, fps-agent   the binaries
#   modules/                    the module trees that the agent loads
#   files/                      the files of image/files, laid out as /
#   final/                      the files of image/final, laid out as /
set -euo pipefail

readonly STAGE=/tmp/fps-stage
export DEBIAN_FRONTEND=noninteractive

echo "== Wait for the first boot of the build VM to finish"
cloud-init status --wait || true

echo "== Check the kernel"
# AGT-06 (decision D12 item 3): the agent needs IP_LOCAL_PORT_RANGE, which
# arrived in Linux 6.3.
kernel="$(uname -r)"
if [[ "$(printf '6.3\n%s\n' "${kernel%%-*}" | sort -V | head -n 1)" != "6.3" ]]; then
	echo "kernel $kernel is older than 6.3, which the agent requires" >&2
	exit 1
fi
echo "kernel $kernel"

echo "== Install packages"
apt_opts=(-o DPkg::Lock::Timeout=300 -y)
apt-get "${apt_opts[@]}" update
apt-get "${apt_opts[@]}" install --no-install-recommends \
	ca-certificates curl iproute2 iputils-ping nftables qemu-guest-agent

echo "== Install the binaries, modules, and configuration"
id fps >/dev/null 2>&1 || useradd --system --home-dir /var/lib/fps --shell /usr/sbin/nologin --user-group fps
install -m 0755 "$STAGE/fps-controller" /usr/bin/fps-controller
install -m 0755 "$STAGE/fps-agent" /usr/bin/fps-agent
install -d -m 0755 /usr/share/fps/modules
cp -a "$STAGE/modules/." /usr/share/fps/modules/
chown -R root:root /usr/share/fps
cp -a "$STAGE/files/." /
chmod 0755 /usr/libexec/fps/*
chmod 0644 /etc/fps/*.env /etc/fps/controller.yaml /etc/fps/isolation.nft /etc/fps/notrack.nft

echo "== Validate the rules and units that we installed"
nft -c -f /etc/fps/isolation.nft
nft -c -f /etc/fps/notrack.nft || echo "notrack.nft was not checked, because the build kernel lacks nft_ct"
systemd-analyze verify /etc/systemd/system/fps-*.service || true

echo "== Remove the network files that the cloud image or cloud-init made"
rm -f /etc/netplan/*.yaml
rm -f /etc/systemd/network/10-netplan-*.network /run/systemd/network/10-netplan-*

echo "== Enable the services"
# fps-role.service enables or disables the controller and the agent again at every
# boot, from the role in the cloud-init data (all-in-one without it).
systemctl enable fps-firewall.service fps-role.service fps-tune.service fps-mgmt-net.service fps-mgmt-addr.service \
	fps-controller.service fps-agent.service qemu-guest-agent.service systemd-networkd.service

echo "== Smoke test the binaries"
/usr/bin/fps-controller -version
/usr/bin/fps-agent capabilities >/dev/null
echo "binaries run"
