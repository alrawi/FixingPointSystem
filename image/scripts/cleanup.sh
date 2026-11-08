#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Runs inside the build VM as root after provision.sh. It makes the disk ready to
# clone: every VM that boots the image gets its own machine id, ssh host keys,
# and cloud-init run. The packer user goes away with the shutdown command of the
# template, because this session still uses it.
set -euo pipefail

readonly STAGE=/tmp/fps-stage

echo "== Install the files that must not exist while the build still uses ssh"
cp -a "$STAGE/final/." /
chmod 0644 /etc/ssh/sshd_config.d/10-fps.conf
# The build seed set ssh_pwauth, and sshd reads our file first, but the file of
# cloud-init has no place in the finished image.
rm -f /etc/ssh/sshd_config.d/50-cloud-init.conf

echo "== Clean package caches and logs"
apt-get clean
rm -rf /var/lib/apt/lists/*
journalctl --rotate >/dev/null 2>&1 || true
journalctl --vacuum-time=1s >/dev/null 2>&1 || true
rm -rf /var/log/*.log /var/log/apt /var/log/installer /var/tmp/* 2>/dev/null || true
rm -rf "$STAGE"

echo "== Reset the identity of the image"
rm -f /etc/ssh/ssh_host_*
truncate -s 0 /etc/machine-id
rm -f /var/lib/dbus/machine-id
ln -sf /etc/machine-id /var/lib/dbus/machine-id
rm -rf /var/lib/systemd/random-seed /var/lib/dhcp/* /var/lib/systemd/network/*
# Forget the build run, so cloud-init runs again on the first boot of each VM.
cloud-init clean --logs --seed
rm -f /root/.bash_history

echo "== Trim free space so the qcow2 stays small"
fstrim -av || true
sync
