#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Installs Ubuntu packages on a GitHub hosted runner with bounded apt calls.
#
#   tools/ci/apt-install.sh [apt-get install option ...] package ...
#
# On 2026-11-10, runner image 20261101.360 let `apt-get update` stall for six
# hours in 14 jobs: the Azure mirror answered `Ign:`, apt fell back to
# archive.ubuntu.com, fetched the InRelease files, and then never finished the
# index downloads (actions/runner-images#15040). apt has no overall time limit,
# so a stalled transfer lasted until GitHub's 6-hour job limit. This script
# bounds every apt call with `timeout`, retries it, and after a failed update
# points apt straight at the Ubuntu archive instead of the mirror list.
#
# The script writes the apt settings below to /etc/apt/apt.conf.d/zzz-fps-ci.
# apt reads that directory in C-locale name order and the last setting wins,
# and the image's own zz-retries sets Retries 1 and Timeout 15, so the name
# sorts after it. Then `apt-get update` runs under a timeout with up to three
# attempts, and `apt-get install -y -q --no-install-recommends` the same way.
# When an update attempt fails or times out, the fallback rewrites
# /etc/apt/sources.list.d/ubuntu.sources so that its stanzas use
# https://archive.ubuntu.com/ubuntu, and https://security.ubuntu.com/ubuntu for
# the security suite, with the components and Signed-By as they are. A failed
# install attempt applies the same fallback once, refreshes the lists, and
# retries. Options before the package names go to apt-get install, and
# --install-recommends drops the default --no-install-recommends, so a step
# that installs with recommends keeps doing so. DEBIAN_FRONTEND is
# noninteractive, and the script uses sudo unless it already runs as root.
#
# Settings through the environment, which tools/ci/test_apt_install.sh uses:
#   FPS_APT_ATTEMPTS           attempts per apt command                (default 3)
#   FPS_APT_UPDATE_TIMEOUT_S   seconds for one apt-get update          (default 300)
#   FPS_APT_INSTALL_TIMEOUT_S  seconds for one apt-get install         (default 600)
#   FPS_APT_KILL_AFTER_S       seconds from the timeout to SIGKILL     (default 30)
#   FPS_APT_RETRY_DELAY_S      seconds between two attempts            (default 5)
#   FPS_APT_CONF               the apt.conf.d file to write
#   FPS_APT_SOURCES            the deb822 sources file of the fallback
set -euo pipefail

attempts=${FPS_APT_ATTEMPTS:-3}
update_timeout=${FPS_APT_UPDATE_TIMEOUT_S:-300}
install_timeout=${FPS_APT_INSTALL_TIMEOUT_S:-600}
kill_after=${FPS_APT_KILL_AFTER_S:-30}
retry_delay=${FPS_APT_RETRY_DELAY_S:-5}
conf=${FPS_APT_CONF:-/etc/apt/apt.conf.d/zzz-fps-ci}
sources=${FPS_APT_SOURCES:-/etc/apt/sources.list.d/ubuntu.sources}
archive=https://archive.ubuntu.com/ubuntu
security=https://security.ubuntu.com/ubuntu

usage() {
  echo "usage: tools/ci/apt-install.sh [apt-get install option ...] package ..." >&2
  exit 2
}

# The options come first and start with a dash. -o takes a value.
install_opts=(-y -q)
no_recommends=1
while (($# > 0)) && [[ $1 == -* ]]; do
  case $1 in
    --install-recommends) no_recommends=0 ;;
    --no-install-recommends) ;;
    -o)
      (($# >= 2)) || usage
      install_opts+=("$1" "$2")
      shift
      ;;
    --) shift; break ;;
    *) install_opts+=("$1") ;;
  esac
  shift
done
(($# > 0)) || usage
if ((no_recommends)); then
  install_opts+=(--no-install-recommends)
fi

if [[ $(id -u) == 0 ]]; then
  as_root() { "$@"; }
else
  as_root() { sudo "$@"; }
fi

# bounded SECONDS COMMAND... runs an apt command as root under timeout. It
# returns the exit code of the command, 124 when the timeout stopped it, and 137
# when only SIGKILL did.
bounded() {
  local seconds=$1
  shift
  as_root env DEBIAN_FRONTEND=noninteractive timeout -k "$kill_after" "$seconds" "$@"
}

# use_direct_archive rewrites the deb822 sources file once: every stanza with a
# URIs line gets the Ubuntu archive, and a stanza whose Suites name a -security
# suite gets the security archive. The other lines stay as they are.
fallback_done=0
use_direct_archive() {
  if ((fallback_done)); then
    return 0
  fi
  fallback_done=1
  if [[ ! -f $sources ]]; then
    echo "apt-install.sh: $sources does not exist, so the sources stay as they are"
    return 0
  fi
  echo "apt-install.sh: pointing $sources at $archive and $security instead of the mirror list"
  local rewritten
  rewritten=$(awk -v archive="$archive" -v security="$security" '
    BEGIN { RS = ""; ORS = "" }
    {
      if ($0 ~ /(^|\n)URIs:/) {
        uri = archive
        if ($0 ~ /(^|\n)Suites:[^\n]*-security/) uri = security
        sub(/^URIs:[^\n]*/, "URIs: " uri)
        gsub(/\nURIs:[^\n]*/, "\nURIs: " uri)
      }
      print $0 "\n\n"
    }' "$sources")
  printf '%s\n' "$rewritten" | as_root tee "$sources" >/dev/null
}

# describe EXIT_CODE says what ended an attempt.
describe() {
  case $1 in
    124 | 137) echo "ran past its time limit and was stopped" ;;
    *) echo "failed with exit code $1" ;;
  esac
}

apt_update() {
  local attempt rc
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    echo "apt-install.sh: apt-get update, attempt $attempt of $attempts, ${update_timeout}s limit"
    rc=0
    bounded "$update_timeout" apt-get update -q || rc=$?
    if ((rc == 0)); then
      return 0
    fi
    echo "apt-install.sh: apt-get update $(describe "$rc")"
    use_direct_archive
    if ((attempt < attempts)); then
      sleep "$retry_delay"
    fi
  done
  echo "apt-install.sh: FAIL apt-get update ran out of attempts ($attempts)" >&2
  return 1
}

apt_install() {
  local attempt rc
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    echo "apt-install.sh: apt-get install $*, attempt $attempt of $attempts, ${install_timeout}s limit"
    rc=0
    bounded "$install_timeout" apt-get install "${install_opts[@]}" "$@" || rc=$?
    if ((rc == 0)); then
      return 0
    fi
    echo "apt-install.sh: apt-get install $(describe "$rc")"
    # A stopped install can leave dpkg half configured, which the next attempt
    # refuses to touch until this runs.
    if command -v dpkg >/dev/null 2>&1; then
      bounded "$install_timeout" dpkg --configure -a || true
    fi
    if ((!fallback_done)); then
      use_direct_archive
      apt_update
    fi
    if ((attempt < attempts)); then
      sleep "$retry_delay"
    fi
  done
  echo "apt-install.sh: FAIL apt-get install ran out of attempts ($attempts)" >&2
  return 1
}

as_root mkdir -p "$(dirname "$conf")"
printf '%s\n' \
  'Acquire::Retries "3";' \
  'Acquire::http::Timeout "20";' \
  'Acquire::https::Timeout "20";' \
  'Acquire::ftp::Timeout "20";' |
  as_root tee "$conf" >/dev/null
if command -v apt-config >/dev/null 2>&1; then
  echo "apt-install.sh: effective apt settings"
  as_root apt-config dump Acquire::Retries Acquire::http::Timeout Acquire::https::Timeout Acquire::ftp::Timeout || true
fi

apt_update
apt_install "$@"
