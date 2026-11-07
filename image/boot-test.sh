#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Boots the qcow2 appliance under QEMU and checks it from the host and from inside.
#
#   image/boot-test.sh [IMAGE]       IMAGE defaults to image/output/fps-base.qcow2
#
# The VM gets two NICs. The first is the management NIC on user networking, with a
# host port forward to the controller API (127.0.0.1:$FPS_TEST_API_PORT) and to ssh
# (127.0.0.1:$FPS_TEST_SSH_PORT). The second is a test NIC on a separate user
# network without outside access. The script boots a throwaway overlay of the
# image, so the image itself stays unchanged, and it removes the VM and every
# temporary file when it ends. It prints one PASS or FAIL line per check, and it
# exits with 0 only when every check passes.
#
# Settings through the environment:
#   FPS_TEST_API_PORT    host port for the controller API       (default 18443)
#   FPS_TEST_SSH_PORT    host port for ssh                      (default 12222)
#   FPS_TEST_BOOT_WAIT   seconds to wait for the API            (default 300)
#   FPS_ALLOW_TCG=1      use software emulation when /dev/kvm is missing (slow)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${1:-$root/image/output/fps-base.qcow2}"
api_port="${FPS_TEST_API_PORT:-18443}"
ssh_port="${FPS_TEST_SSH_PORT:-12222}"
boot_wait="${FPS_TEST_BOOT_WAIT:-300}"
token="boot-test-$(openssl rand -hex 16)"

fails=0
checks=0
pass() {
	checks=$((checks + 1))
	echo "PASS $1"
}
fail() {
	checks=$((checks + 1))
	fails=$((fails + 1))
	echo "FAIL $1${2:+ ($2)}"
}

need() {
	command -v "$1" >/dev/null 2>&1 || {
		echo "FAIL missing command: $1 (see image/README.md, Prerequisites)" >&2
		exit 2
	}
}
for cmd in qemu-system-x86_64 qemu-img cloud-localds curl ssh scp ssh-keygen openssl python3; do
	need "$cmd"
done
if [[ ! -f "$image" ]]; then
	echo "FAIL image not found: $image (run image/build.sh first)" >&2
	exit 2
fi
image="$(readlink -f "$image")"

if [[ -w /dev/kvm ]]; then
	accel=(-machine "q35,accel=kvm" -cpu host)
elif [[ "${FPS_ALLOW_TCG:-0}" == 1 ]]; then
	echo "NOTE /dev/kvm is not writable, so QEMU uses software emulation and boot takes minutes"
	accel=(-machine "q35,accel=tcg" -cpu max)
	boot_wait=$((boot_wait * 4))
else
	echo "FAIL /dev/kvm is not writable by this user. Add yourself to the kvm group, or set FPS_ALLOW_TCG=1." >&2
	exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/fps-boot-test.XXXXXX")"
pidfile="$work/qemu.pid"
serial="$work/serial.log"

# shellcheck disable=SC2329 # runs through the EXIT trap
cleanup() {
	local status=$?
	if [[ -f "$pidfile" ]]; then
		local pid
		pid="$(cat "$pidfile" 2>/dev/null || true)"
		if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
			kill "$pid" 2>/dev/null || true
			for _ in $(seq 1 20); do
				kill -0 "$pid" 2>/dev/null || break
				sleep 0.5
			done
			kill -9 "$pid" 2>/dev/null || true
		fi
	fi
	if ((fails > 0 || status != 0)) && [[ -s "$serial" ]]; then
		echo "---- last 80 lines of the VM serial console ----"
		tail -n 80 "$serial" || true
		echo "---- end of serial console ----"
	fi
	rm -rf "$work"
}
trap cleanup EXIT

# --- Seed: an ssh key for the default user fps-admin, and a known API token. ---
ssh-keygen -q -t ed25519 -N '' -f "$work/key" -C fps-boot-test
cat >"$work/user-data" <<EOF
#cloud-config
ssh_authorized_keys:
  - $(cat "$work/key.pub")
write_files:
  - path: /etc/fps/admin.token
    owner: root:fps
    permissions: "0640"
    content: $token
EOF
cat >"$work/meta-data" <<EOF
instance-id: fps-boot-test-$(openssl rand -hex 4)
local-hostname: fps-boot-test
EOF
cloud-localds "$work/seed.img" "$work/user-data" "$work/meta-data"

# --- Boot a throwaway overlay of the image. ---
qemu-img create -q -f qcow2 -F qcow2 -b "$image" "$work/overlay.qcow2"

qemu-system-x86_64 \
	"${accel[@]}" -smp 2 -m 2048 \
	-display none -serial "file:$serial" -monitor none \
	-drive "file=$work/overlay.qcow2,if=virtio,format=qcow2" \
	-drive "file=$work/seed.img,if=virtio,format=raw,readonly=on" \
	-netdev "user,id=mgmt,hostfwd=tcp:127.0.0.1:${api_port}-:8443,hostfwd=tcp:127.0.0.1:${ssh_port}-:22" \
	-device virtio-net-pci,netdev=mgmt,mac=52:54:00:12:34:01 \
	-netdev "user,id=test,net=10.0.3.0/24,restrict=on" \
	-device virtio-net-pci,netdev=test,mac=52:54:00:12:34:02 \
	-pidfile "$pidfile" -daemonize
echo "NOTE VM started, waiting up to ${boot_wait}s for GET /v1/capabilities"

api() { # api PATH [curl options]: GET on the forwarded management port
	local path="$1"
	shift
	curl -s -m 5 "$@" "http://127.0.0.1:${api_port}${path}"
}

# --- Check 1: /v1/capabilities answers on the management interface. ---
body=""
code=000
deadline=$((SECONDS + boot_wait))
while ((SECONDS < deadline)); do
	if ! kill -0 "$(cat "$pidfile")" 2>/dev/null; then
		break
	fi
	code="$(api /v1/capabilities -o "$work/caps.json" -w '%{http_code}' -H "Authorization: Bearer $token" || true)"
	if [[ "$code" == 200 ]]; then
		body="$(cat "$work/caps.json")"
		break
	fi
	sleep 3
done
if [[ "$code" == 200 ]] && python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert isinstance(d, dict)' "$work/caps.json"; then
	pass "GET /v1/capabilities on the management interface answers 200 with a JSON object"
	echo "NOTE capabilities: $(printf '%s' "$body" | head -c 300)"
else
	fail "GET /v1/capabilities on the management interface answers 200 with a JSON object" "last HTTP status $code"
fi

# --- Check 2: the API demands a token. ---
code="$(api /v1/capabilities -o /dev/null -w '%{http_code}' || true)"
if [[ "$code" == 401 ]]; then
	pass "GET /v1/capabilities without a token answers 401"
else
	fail "GET /v1/capabilities without a token answers 401" "HTTP status $code"
fi

# --- Checks 3 and up: from inside the VM, over ssh. ---
ssh_opts=(-i "$work/key" -p "$ssh_port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
	-o LogLevel=ERROR -o BatchMode=yes -o ConnectTimeout=5)
scp_opts=(-i "$work/key" -P "$ssh_port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
	-o LogLevel=ERROR -o BatchMode=yes -o ConnectTimeout=5)
ssh_ok=no
for _ in $(seq 1 40); do
	if ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 true 2>/dev/null; then
		ssh_ok=yes
		break
	fi
	sleep 3
done
if [[ "$ssh_ok" == yes ]]; then
	pass "ssh as fps-admin with the seed key works"
	vm_out="$work/vm-checks.out"
	: >"$vm_out"
	if scp "${scp_opts[@]}" "$root/image/tests/vm-checks.sh" "$root/image/tests/isolation-netns.sh" fps-admin@127.0.0.1:/tmp/ &&
		ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 'chmod +x /tmp/vm-checks.sh /tmp/isolation-netns.sh'; then
		ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 'sudo /tmp/vm-checks.sh' >"$vm_out" 2>&1 || true
	fi
	while IFS= read -r line; do
		echo "$line"
		case "$line" in
		PASS*) checks=$((checks + 1)) ;;
		FAIL*)
			checks=$((checks + 1))
			fails=$((fails + 1))
			;;
		esac
	done <"$vm_out"
	if ! grep -q '^PASS\|^FAIL' "$vm_out"; then
		fail "the checks inside the VM ran" "no PASS or FAIL line came back"
	fi
else
	fail "ssh as fps-admin with the seed key works"
fi

echo
if ((fails == 0)); then
	echo "RESULT: PASS ($checks checks)"
	exit 0
fi
echo "RESULT: FAIL ($fails of $checks checks failed)"
exit 1
