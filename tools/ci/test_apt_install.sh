#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Tests tools/ci/apt-install.sh without root and without network.
#
#   tools/ci/test_apt_install.sh
#
# The test puts stubs of apt-get, dpkg, and sudo first on the PATH and points the
# helper's config and sources paths at a temporary directory. The sudo stub runs
# its command as the test user. The apt-get stub records every call in a log and
# takes its behavior from the next line of a plan file: `ok` exits 0, `fail`
# exits 100, and `sleep N` sleeps N seconds, so that the real `timeout` of the
# helper has to stop it. Time limits are one or two seconds and the retry delay
# is zero. The script prints one PASS or FAIL line per check and exits with 1
# when any check fails. It needs bash, awk, and the coreutils timeout.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
helper=$here/apt-install.sh
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0
pass() { echo "PASS $1"; }
fail() {
  echo "FAIL $1"
  fails=$((fails + 1))
}

mkdir -p "$tmp/bin"
cat > "$tmp/bin/sudo" <<'EOF'
#!/usr/bin/env bash
exec "$@"
EOF
cat > "$tmp/bin/apt-get" <<'EOF'
#!/usr/bin/env bash
echo "$*" >> "$FPS_TEST_LOG"
line=$(head -n 1 "$FPS_TEST_PLAN")
tail -n +2 "$FPS_TEST_PLAN" > "$FPS_TEST_PLAN.next"
mv "$FPS_TEST_PLAN.next" "$FPS_TEST_PLAN"
case ${line:-ok} in
  ok) exit 0 ;;
  fail) exit 100 ;;
  sleep*) sleep "${line#sleep }"; exit 0 ;;
  *) echo "apt-get stub: unknown plan line '$line'" >&2; exit 99 ;;
esac
EOF
cat > "$tmp/bin/dpkg" <<'EOF'
#!/usr/bin/env bash
echo "dpkg $*" >> "$FPS_TEST_LOG"
EOF
chmod +x "$tmp/bin/sudo" "$tmp/bin/apt-get" "$tmp/bin/dpkg"

# The sources file of an ubuntu-24.04 runner, where the image replaced the
# archive with its mirror list.
read -r -d '' sources_before <<'EOF' || true
## Ubuntu distribution repository
##
## The following settings can be adjusted to configure which packages to use from Ubuntu.
Types: deb
URIs: mirror+file:/etc/apt/apt-mirrors.txt
Suites: noble noble-updates noble-backports
Components: main universe restricted multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

## Ubuntu security updates. Aside from URIs and Suites,
## this should mirror your choices in the previous section.
Types: deb
URIs: mirror+file:/etc/apt/apt-mirrors.txt
Suites: noble-security
Components: main universe restricted multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
read -r -d '' sources_after <<'EOF' || true
## Ubuntu distribution repository
##
## The following settings can be adjusted to configure which packages to use from Ubuntu.
Types: deb
URIs: https://archive.ubuntu.com/ubuntu
Suites: noble noble-updates noble-backports
Components: main universe restricted multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

## Ubuntu security updates. Aside from URIs and Suites,
## this should mirror your choices in the previous section.
Types: deb
URIs: https://security.ubuntu.com/ubuntu
Suites: noble-security
Components: main universe restricted multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF

# run NAME PLAN_LINES... runs the helper with the arguments in $args, the plan
# lines, and a fresh log, conf, and sources file. It leaves the exit code in
# $rc, the output in $out, and the call log in $log.
args=()
run() {
  local name=$1
  shift
  local dir=$tmp/$name
  mkdir -p "$dir"
  log=$dir/calls.log
  : > "$log"
  printf '%s\n' "$@" > "$dir/plan"
  printf '%s\n' "$sources_before" > "$dir/ubuntu.sources"
  rc=0
  out=$(
    PATH="$tmp/bin:$PATH" \
      FPS_TEST_LOG=$log FPS_TEST_PLAN=$dir/plan \
      FPS_APT_CONF="$dir/apt.conf.d/zzz-fps-ci" FPS_APT_SOURCES="$dir/ubuntu.sources" \
      FPS_APT_UPDATE_TIMEOUT_S=1 FPS_APT_INSTALL_TIMEOUT_S=1 FPS_APT_KILL_AFTER_S=1 \
      FPS_APT_RETRY_DELAY_S=0 \
      "$helper" "${args[@]}" 2>&1
  ) || rc=$?
  conf_file=$dir/apt.conf.d/zzz-fps-ci
  sources_file=$dir/ubuntu.sources
}
calls() { grep -c "^$1" "$log" || true; }

# 1. A stub that succeeds installs once, and the apt settings are in place.
args=(iproute2 ethtool)
run success
if ((rc == 0)) && [[ $(calls "update -q") == 1 && $(calls "install") == 1 ]]; then
  pass "a succeeding apt-get updates once and installs once"
else
  fail "a succeeding apt-get updates once and installs once (exit $rc, calls: $(tr '\n' ';' < "$log"))"
fi
if [[ $(tail -n 1 "$log") == "install -y -q --no-install-recommends iproute2 ethtool" ]]; then
  pass "the install line carries -y -q --no-install-recommends and the packages"
else
  fail "the install line carries -y -q --no-install-recommends and the packages (got: $(tail -n 1 "$log"))"
fi
if [[ -f $conf_file ]] &&
  grep -qx 'Acquire::Retries "3";' "$conf_file" &&
  grep -qx 'Acquire::http::Timeout "20";' "$conf_file" &&
  grep -qx 'Acquire::https::Timeout "20";' "$conf_file" &&
  grep -qx 'Acquire::ftp::Timeout "20";' "$conf_file"; then
  pass "the apt.conf.d file holds Retries 3 and the three 20 s timeouts"
else
  fail "the apt.conf.d file holds Retries 3 and the three 20 s timeouts"
fi
if [[ $(cat "$sources_file") == "$sources_before" ]]; then
  pass "a successful update leaves the sources file alone"
else
  fail "a successful update leaves the sources file alone"
fi

# 2. An update that sleeps past the limit is stopped and retried.
args=(iproute2)
start=$SECONDS
run stall "sleep 30" ok ok
elapsed=$((SECONDS - start))
if ((rc == 0)) && [[ $(calls "update -q") == 2 && $(calls "install") == 1 ]] && ((elapsed < 15)); then
  pass "an update that sleeps past the limit is stopped and retried (${elapsed}s)"
else
  fail "an update that sleeps past the limit is stopped and retried (exit $rc, ${elapsed}s, calls: $(tr '\n' ';' < "$log"))"
fi
if grep -q "apt-get update ran past its time limit and was stopped" <<<"$out"; then
  pass "the output says that the update was stopped"
else
  fail "the output says that the update was stopped"
fi

# 3. A failed update rewrites the sources file to the direct archives.
run fallback fail ok ok
if ((rc == 0)) && [[ $(cat "$sources_file") == "$sources_after" ]]; then
  pass "a failed update points the sources at archive.ubuntu.com and security.ubuntu.com"
else
  fail "a failed update points the sources at archive.ubuntu.com and security.ubuntu.com (exit $rc)"
  diff <(printf '%s\n' "$sources_after") "$sources_file" || true
fi

# 4. Three failed updates end with a nonzero exit and the named message.
run exhausted fail fail fail
if ((rc != 0)) && [[ $(calls "update -q") == 3 && $(calls "install") == 0 ]]; then
  pass "three failed updates end with exit $rc and no install"
else
  fail "three failed updates end with a nonzero exit and no install (exit $rc, calls: $(tr '\n' ';' < "$log"))"
fi
if grep -q "FAIL apt-get update ran out of attempts (3)" <<<"$out"; then
  pass "the message names apt-get update and the three attempts"
else
  fail "the message names apt-get update and the three attempts"
fi

# 5. A failed install applies the fallback once, refreshes the lists, and
#    retries. Three failures name the install step.
run install-fail ok fail ok fail fail
if ((rc != 0)) && [[ $(calls "update -q") == 2 && $(calls "install") == 3 ]] &&
  [[ $(cat "$sources_file") == "$sources_after" ]]; then
  pass "a failed install falls back, refreshes the lists once, and retries"
else
  fail "a failed install falls back, refreshes the lists once, and retries (exit $rc, calls: $(tr '\n' ';' < "$log"))"
fi
if grep -q "FAIL apt-get install ran out of attempts (3)" <<<"$out"; then
  pass "the message names apt-get install and the three attempts"
else
  fail "the message names apt-get install and the three attempts"
fi

# 6. Options pass through, and --install-recommends drops --no-install-recommends.
args=(--install-recommends -o Dpkg::Use-Pty=0 tshark)
run options
if ((rc == 0)) && [[ $(tail -n 1 "$log") == "install -y -q -o Dpkg::Use-Pty=0 tshark" ]]; then
  pass "--install-recommends keeps recommends and other options pass through"
else
  fail "--install-recommends keeps recommends and other options pass through (exit $rc, got: $(tail -n 1 "$log"))"
fi

# 7. No package is a usage error.
args=()
rc=0
PATH="$tmp/bin:$PATH" "$helper" >/dev/null 2>&1 || rc=$?
if ((rc == 2)); then
  pass "no package name is a usage error"
else
  fail "no package name is a usage error (exit $rc)"
fi

if ((fails > 0)); then
  echo "RESULT: FAIL ($fails checks)"
  exit 1
fi
echo "RESULT: PASS"
