#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the pinned nDPI release from source and puts its ndpiReader first on the
# search path. Ubuntu 24.04 ships nDPI 4.2.0, which has no HTTP/2 protocol, so the
# http2 module cannot conform with it. realism/ci/ndpi.version is the only place
# that names the release: a tag and the full commit id of that tag, for example
# "4.14 90090b9a...". The script fetches that commit and no tag, because a tag can
# move and a commit id cannot, checks that the checkout has that id, builds the
# static library and ndpiReader, and installs the reader under /opt/ndpi, linked
# from /usr/local/bin so that it shadows the libndpi-bin package. Then it fails when
# the reader does not report the pinned release. FPS_NDPI_VERSION overrides the file
# for a trial run, with the same two fields, and FPS_NDPI_PREFIX the install prefix.
# The script does nothing but the link when the pinned reader is already installed,
# so a runner that restores /opt/ndpi from a cache skips the build.
#
# The build needs a C compiler, make, git, and the packages that the script
# installs. nDPI is LGPL-3.0, and the realism runner runs ndpiReader as a separate
# process, so the project never links it.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
pin=${FPS_NDPI_VERSION:-$(tr -s '[:space:]' ' ' < "$here/ndpi.version")}
read -r tag commit _ <<<"$pin"
if [[ ! ${tag:-} =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ || ! ${commit:-} =~ ^[0-9a-f]{40}$ ]]; then
  echo "build-ndpi.sh: '$pin' is not a release and a 40 digit commit id, as in $here/ndpi.version" >&2
  exit 2
fi
prefix=${FPS_NDPI_PREFIX:-/opt/ndpi}
reader=$prefix/bin/ndpiReader

# nDPI 4.14 prints its usage text for -h and exits with 1, so the exit code says
# nothing. The first line is the banner "Welcome to nDPI 4.14.0-...".
banner() {
  "$1" -h 2>&1 | head -1 || true
}
reports_pin() {
  [[ -x $1 ]] || return 1
  [[ $(banner "$1") =~ nDPI\ ${tag//./\\.}(\.|-|$) ]]
}

if ! reports_pin "$reader"; then
  "$here/../../tools/ci/apt-install.sh" \
    build-essential git autoconf automake libtool pkg-config gettext flex bison \
    libpcap-dev libjson-c-dev
  src=$(mktemp -d)
  trap 'rm -rf "$src"' EXIT
  git -C "$src" init -q
  git -C "$src" fetch -q --depth 1 https://github.com/ntop/nDPI.git "$commit"
  git -C "$src" checkout -q FETCH_HEAD
  found=$(git -C "$src" rev-parse HEAD)
  if [[ $found != "$commit" ]]; then
    echo "build-ndpi.sh: fetched $found, expected $commit" >&2
    exit 1
  fi
  (
    cd "$src"
    ./autogen.sh >/dev/null
    ./configure --prefix="$prefix" >/dev/null
    make -j"$(nproc)" -C src/lib libndpi.a >/dev/null
    make -j"$(nproc)" -C example ndpiReader >/dev/null
  )
  sudo install -D -m 0755 "$src/example/ndpiReader" "$reader"
fi
sudo ln -sf "$reader" /usr/local/bin/ndpiReader

installed=$(banner "$reader")
echo "ndpiReader: $reader: $installed"
if ! reports_pin "$reader"; then
  echo "build-ndpi.sh: expected nDPI $tag, found: $installed" >&2
  exit 1
fi
