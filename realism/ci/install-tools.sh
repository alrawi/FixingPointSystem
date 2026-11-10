#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Installs the harness and realism tools of the module realism job on an Ubuntu
# 24.04 runner. tshark and the harness tools come from Ubuntu, nDPI comes from
# source (see below), and Zeek comes from the package repository of the Zeek
# project on the openSUSE Build Service, because Ubuntu does not ship it. Both the
# realism and the rlm workflow call this script, so they share one repository, one
# Zeek version, and one nDPI release. Ubuntu ships nDPI 4.2.0, which has no HTTP/2
# protocol, so realism/ci/build-ndpi.sh builds the release that
# realism/ci/ndpi.version pins and links its ndpiReader into /usr/local/bin, ahead
# of the libndpi-bin package of the line below. The Zeek
# version is the package version in realism/ci/zeek.version, which is the only
# place that names it, and FPS_ZEEK_VERSION overrides it for a trial run. The
# script fails when the repository no longer offers that version, or when the
# installed program reports another one. Zeek lands in /opt/zeek/bin, where the
# realism runner looks for it when zeek is not on the search path.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
zeek_version=${FPS_ZEEK_VERSION:-$(tr -d '[:space:]' < "$here/zeek.version")}
if [[ ! $zeek_version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.+~]+)?$ ]]; then
  echo "install-tools.sh: $here/zeek.version holds '$zeek_version', not a package version such as 9.0.1-0" >&2
  exit 2
fi

echo "wireshark-common wireshark-common/install-setuid boolean false" | sudo debconf-set-selections
curl -fsSL https://download.opensuse.org/repositories/security:zeek/xUbuntu_24.04/Release.key \
  | sudo gpg --dearmor --yes -o /usr/share/keyrings/zeek.gpg
echo "deb [signed-by=/usr/share/keyrings/zeek.gpg] https://download.opensuse.org/repositories/security:/zeek/xUbuntu_24.04/ /" \
  | sudo tee /etc/apt/sources.list.d/zeek.list >/dev/null
sudo apt-get update -q
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends \
  iproute2 tcpdump ethtool libndpi-bin tshark "zeek-core=$zeek_version" python3-yaml
ndpiReader -h | head -1 || true
tshark --version | head -1
installed=$(/opt/zeek/bin/zeek --version)
echo "$installed"
if [[ $installed != *"version ${zeek_version%%-*}"* ]]; then
  echo "install-tools.sh: expected Zeek ${zeek_version%%-*}, found: $installed" >&2
  exit 1
fi

# nDPI comes last so that its ndpiReader is the one that the workflows find.
"$here/build-ndpi.sh"
