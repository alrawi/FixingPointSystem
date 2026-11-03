#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Installs the harness and realism tools of the module realism job on an Ubuntu
# 24.04 runner. nDPI, tshark, and the harness tools come from Ubuntu, and Zeek
# comes from the package repository of the Zeek project on the openSUSE Build
# Service, because Ubuntu does not ship it. The rlm workflow uses the same
# repository. Zeek lands in /opt/zeek/bin, where the realism runner looks for it
# when zeek is not on the search path.
set -euo pipefail

echo "wireshark-common wireshark-common/install-setuid boolean false" | sudo debconf-set-selections
curl -fsSL https://download.opensuse.org/repositories/security:zeek/xUbuntu_24.04/Release.key \
  | sudo gpg --dearmor --yes -o /usr/share/keyrings/zeek.gpg
echo "deb [signed-by=/usr/share/keyrings/zeek.gpg] https://download.opensuse.org/repositories/security:/zeek/xUbuntu_24.04/ /" \
  | sudo tee /etc/apt/sources.list.d/zeek.list >/dev/null
sudo apt-get update -q
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends \
  iproute2 tcpdump ethtool libndpi-bin tshark zeek-core python3-yaml
ndpiReader -h | head -1 || true
tshark --version | head -1
/opt/zeek/bin/zeek --version
