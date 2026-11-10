#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Boots the qcow2 appliance under QEMU and checks it from the host and from inside.
#
#   image/boot-test.sh [--role ROLE] [--run] [IMAGE]
#
#   IMAGE defaults to image/output/fps-base.qcow2.
#   ROLE is all-in-one (the default, and the behavior of IMG-01), controller,
#   agent, or all, which boots the image once per role, one after the other. The
#   script seeds the role through NoCloud user-data (/etc/fps/role.env), boots,
#   and checks which units run and which do not, and the tuning of the role. The
#   agent role boots with 4 CPUs and 4 GiB, so that CPU isolation and hugepages
#   apply, and the VM reboots once by itself to apply the kernel options. The
#   script waits for that reboot.
#   --run adds a short HTTP and DNS run inside the all-in-one VM (IMG-04), through
#   the controller REST API with the agent `local`, which image/tests/vm-run.sh
#   does. The other roles ignore it.
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
#   FPS_TEST_SMP         CPUs of the VM    (default 2, and 4 for the agent role)
#   FPS_TEST_MEM         MiB of the VM     (default 2048, and 4096 for the agent role)
#   FPS_ALLOW_TCG=1      use software emulation when /dev/kvm is missing (slow)
#   FPS_TEST_ARTIFACT_DIR  directory that receives the serial console log, the output of
#                        the checks inside the VM, and the files of --run (CI uploads it)
#   FPS_RUN_RATE / FPS_RUN_STEADY_S / FPS_RUN_WAIT_S  settings of the --run (vm-run.sh)
#   FPS_TEST_CONTROLLER_BIN  path of an fps-controller binary for the agent role (IMG-06).
#                        The script runs it on the host, mints an enrollment token with
#                        `fps-controller token` into a file, seeds the token and the CA
#                        through NoCloud, and checks over the host API that the agent
#                        registered over mutual TLS. Without it, the agent role seeds a
#                        placeholder token and a fingerprint, and checks only that the
#                        agent runs against https:// and redials the absent controller.
#   FPS_TEST_HOST_GRPC_PORT / FPS_TEST_HOST_ENROLL_PORT / FPS_TEST_HOST_API_PORT
#                        host loopback ports of that controller (18444, 18445, 18446)
#   FPS_TEST_REPEAT      boot the image this many times in a row, each boot a full run of
#                        the script, and stop at the first boot that fails (IMG-07,
#                        default 1). The result is PASS only when every boot passed.
#                        With FPS_TEST_ARTIFACT_DIR, boot N writes to boot-N/.
#   FPS_TEST_NEGATIVE=agent-unreachable   after the boots, prove that the test fails when the
#                        agent cannot reach the controller (IMG-07). It boots the all-in-one
#                        role once more with the agent pointed at the loopback, where no
#                        controller listens, and passes only when that boot fails at the
#                        agent checks (and at the HTTP and DNS run with --run) while the
#                        controller API and ssh work. It needs --role all-in-one.
set -euo pipefail

self="${BASH_SOURCE[0]}"
root="$(cd "$(dirname "$self")/.." && pwd)"
orig_args=("$@")
role=all-in-one
run_short=0
image_arg=""
while (($# > 0)); do
	case "$1" in
	--role)
		if (($# < 2)); then
			echo "FAIL --role needs a value" >&2
			exit 2
		fi
		role="$2"
		shift 2
		;;
	--role=*)
		role="${1#--role=}"
		shift
		;;
	--run)
		run_short=1
		shift
		;;
	-h | --help)
		sed -n '4,26p' "$self"
		exit 0
		;;
	-*)
		echo "FAIL unknown option: $1" >&2
		exit 2
		;;
	*)
		image_arg="$1"
		shift
		;;
	esac
done
case "$role" in
all-in-one | controller | agent | all) ;;
*)
	echo "FAIL unknown role: $role (use all-in-one, controller, agent, or all)" >&2
	exit 2
	;;
esac
image="${image_arg:-$root/image/output/fps-base.qcow2}"
api_port="${FPS_TEST_API_PORT:-18443}"
ssh_port="${FPS_TEST_SSH_PORT:-12222}"
boot_wait="${FPS_TEST_BOOT_WAIT:-300}"
artifact_dir="${FPS_TEST_ARTIFACT_DIR:-}"
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

# --- IMG-07: boots in a row and the negative boot. Each boot is a full run of this script. ---
repeat="${FPS_TEST_REPEAT:-10}"                # TEMP IMG-07: the PR run loops, a later commit reverts this
negative="${FPS_TEST_NEGATIVE-agent-unreachable}" # TEMP IMG-07: same
if ! [[ "$repeat" =~ ^[1-9][0-9]*$ ]]; then
	echo "FAIL FPS_TEST_REPEAT must be a positive number, not '$repeat'" >&2
	exit 2
fi
case "$negative" in
"") ;;
agent-unreachable)
	if [[ "$role" != all-in-one ]]; then
		echo "FAIL FPS_TEST_NEGATIVE=agent-unreachable needs --role all-in-one" >&2
		exit 2
	fi
	;;
*)
	echo "FAIL unknown FPS_TEST_NEGATIVE: $negative (use agent-unreachable)" >&2
	exit 2
	;;
esac
if ((repeat > 1)) || [[ -n "$negative" ]]; then
	passed=0
	overall=0
	for ((boot = 1; boot <= repeat; boot++)); do
		echo "=== boot $boot of $repeat"
		boot_dir=""
		if [[ -n "$artifact_dir" ]]; then boot_dir="$artifact_dir/boot-$boot"; fi
		if FPS_TEST_REPEAT=1 FPS_TEST_NEGATIVE="" FPS_TEST_ARTIFACT_DIR="$boot_dir" "$self" "${orig_args[@]}"; then
			passed=$((passed + 1))
		else
			echo "FAIL boot $boot of $repeat did not pass"
			overall=1
			break
		fi
	done
	if ((overall == 0)); then
		echo "PASS $passed of $repeat boots passed in a row"
	else
		echo "FAIL $passed of $repeat boots passed before the first failure"
	fi
	if [[ -n "$negative" && "$overall" == 0 ]]; then
		echo "=== negative boot: the agent cannot reach the controller"
		neg_log="$(mktemp "${TMPDIR:-/tmp}/fps-boot-negative.XXXXXX")"
		neg_dir=""
		if [[ -n "$artifact_dir" ]]; then neg_dir="$artifact_dir/negative"; fi
		neg_status=0
		FPS_TEST_REPEAT=1 FPS_TEST_NEGATIVE="" FPS_TEST_AGENT_UNREACHABLE=1 FPS_TEST_ARTIFACT_DIR="$neg_dir" \
			"$self" "${orig_args[@]}" >"$neg_log" 2>&1 || neg_status=$?
		# The lines of the negative boot are shown with a prefix, so that its expected FAIL lines
		# stay out of the counts of the CI summary.
		sed -E 's/^(PASS|FAIL) /negative boot: \1 /' "$neg_log"
		neg_ok=yes
		neg_why=""
		if [[ "$neg_status" != 1 ]]; then
			neg_ok=no
			neg_why="the boot ended with status $neg_status, and a failed test ends with 1"
		fi
		for must in "FAIL the local agent registered with the controller over mutual TLS" \
			"PASS GET /v1/capabilities on the management interface answers 200" \
			"PASS ssh as fps-admin with the seed key works"; do
			if ! grep -q "^$must" "$neg_log"; then
				neg_ok=no
				neg_why="${neg_why:+$neg_why; }no line '$must'"
			fi
		done
		if [[ "$run_short" == 1 ]] && ! grep -q '^FAIL run: the agent local is online' "$neg_log"; then
			neg_ok=no
			neg_why="${neg_why:+$neg_why; }no line 'FAIL run: the agent local is online'"
		fi
		rm -f "$neg_log"
		if [[ "$neg_ok" == yes ]]; then
			echo "PASS negative boot: the test fails when the agent cannot reach the controller"
		else
			echo "FAIL negative boot: the test must fail at the agent checks when the agent cannot reach the controller ($neg_why)"
			overall=1
		fi
	fi
	echo
	if ((overall == 0)); then
		echo "RESULT: PASS ($passed of $repeat boots in a row${negative:+, and the negative boot failed as it must})"
	else
		echo "RESULT: FAIL (see the boots above)"
	fi
	exit "$overall"
fi

if [[ "$role" == all ]]; then
	overall=0
	summary=()
	for r in all-in-one controller agent; do
		echo "=== role $r"
		run_arg=()
		if [[ "$r" == all-in-one && "$run_short" == 1 ]]; then run_arg=(--run); fi
		if "$self" --role "$r" "${run_arg[@]}" "$image"; then
			summary+=("PASS role $r")
		else
			summary+=("FAIL role $r")
			overall=1
		fi
	done
	echo
	printf '%s\n' "${summary[@]}"
	if ((overall == 0)); then echo "RESULT: PASS (all roles)"; else echo "RESULT: FAIL (see the roles above)"; fi
	exit "$overall"
fi

# The agent profile isolates CPUs and reserves hugepages, so it needs CPUs and memory to spare.
vm_smp="${FPS_TEST_SMP:-2}"
vm_mem="${FPS_TEST_MEM:-2048}"
if [[ "$role" == agent ]]; then
	vm_smp="${FPS_TEST_SMP:-4}"
	vm_mem="${FPS_TEST_MEM:-4096}"
fi

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

host_ctl_pid=""
# shellcheck disable=SC2329 # runs through the EXIT trap
cleanup() {
	local status=$?
	if [[ -n "$host_ctl_pid" ]]; then kill "$host_ctl_pid" 2>/dev/null || true; fi
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
	if [[ -n "$artifact_dir" ]]; then
		mkdir -p "$artifact_dir"
		cp "$serial" "$artifact_dir/serial-$role.log" 2>/dev/null || true
		cp "$work/vm-checks.out" "$artifact_dir/vm-checks-$role.out" 2>/dev/null || true
	fi
	rm -rf "$work"
}
trap cleanup EXIT

# --- Seed: an ssh key for the default user fps-admin, a known API token, and the role. ---
# The all-in-one role gets no role file at all, which proves that it is the default.
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
case "$role" in
all-in-one)
	# The all-in-one role gets no role file at all, which proves that it is the default.
	# The negative boot of IMG-07 points the agent at the loopback, where the controller
	# does not listen (it listens on the management address only), so the agent cannot
	# reach it, and nothing else of the VM changes.
	if [[ "${FPS_TEST_AGENT_UNREACHABLE:-0}" == 1 ]]; then
		cat >>"$work/user-data" <<EOF
  - path: /etc/fps/role.env
    permissions: "0644"
    content: |
      FPS_ROLE=all-in-one
      FPS_CONTROLLER_URI=https://127.0.0.1:8444
      FPS_ENROLL_ADDRESS=127.0.0.1:8445
      FPS_CA_SHA256=$(openssl rand -hex 32)
EOF
		echo "NOTE the agent dials the loopback, where no controller listens (negative boot)"
	fi
	;;
controller)
	cat >>"$work/user-data" <<EOF
  - path: /etc/fps/role.env
    permissions: "0644"
    content: |
      FPS_ROLE=controller
EOF
	;;
agent)
	# 10.0.2.2 is the host in user networking. The agent dials https:// and enrolls with
	# a token that the cloud-init data writes to /etc/fps/enroll.token (mode 0600), and it
	# trusts the CA from /etc/fps/controller-ca.pem or from the fingerprint in role.env.
	# The token is a file in the seed, never an argument or a role.env value.
	grpc_port="${FPS_TEST_HOST_GRPC_PORT:-18444}"
	enroll_port="${FPS_TEST_HOST_ENROLL_PORT:-18445}"
	host_api_port="${FPS_TEST_HOST_API_PORT:-18446}"
	if [[ -n "${FPS_TEST_CONTROLLER_BIN:-}" ]]; then
		ctl="$(readlink -f "$FPS_TEST_CONTROLLER_BIN")"
		mkdir -p "$work/ctl"
		printf '%s' "$token" >"$work/ctl/admin.token"
		cat >"$work/ctl/controller.yaml" <<EOF
log_level: info
data_dir: $work/ctl/data
agent_auto_enroll: true
tokens:
  - { name: admin, scope: admin, token_file: $work/ctl/admin.token }
EOF
		"$ctl" -config "$work/ctl/controller.yaml" -listen "127.0.0.1:$host_api_port" \
			-grpc-listen "127.0.0.1:$grpc_port" -enroll-listen "127.0.0.1:$enroll_port" \
			-tls-names 10.0.2.2,127.0.0.1 >"$work/ctl/controller.log" 2>&1 &
		host_ctl_pid=$!
		for _ in $(seq 1 30); do
			[[ -S "$work/ctl/data/pki/admin.sock" ]] && break
			sleep 1
		done
		# The token goes from standard output into a file. It is on no command line.
		(umask 077 && "$ctl" token -agent-id edge-1 -ttl 30m -config "$work/ctl/controller.yaml" >"$work/ctl/enroll.token")
		token_body="$(cat "$work/ctl/enroll.token")"
		ca_block="  - path: /etc/fps/controller-ca.pem
    permissions: \"0644\"
    content: |
$(sed 's/^/      /' "$work/ctl/data/pki/ca.crt")"
		sha_line=""
	else
		token_body="placeholder-token-not-minted-by-any-controller"
		ca_block=""
		sha_line="FPS_CA_SHA256=$(openssl rand -hex 32)"
	fi
	{
		echo "  - path: /etc/fps/enroll.token"
		echo '    owner: root:root'
		echo '    permissions: "0600"'
		echo "    content: $token_body"
		if [[ -n "$ca_block" ]]; then echo "$ca_block"; fi
		cat <<EOF
  - path: /etc/fps/role.env
    permissions: "0644"
    content: |
      FPS_ROLE=agent
      FPS_CONTROLLER_URI=https://10.0.2.2:$grpc_port
      FPS_ENROLL_ADDRESS=10.0.2.2:$enroll_port
      FPS_AGENT_ID=edge-1
      FPS_AGENT_EXTRA_ARGS=--redial-limit 0
EOF
		if [[ -n "$sha_line" ]]; then echo "      $sha_line"; fi
	} >>"$work/user-data"
	;;
esac
echo "NOTE role $role, $vm_smp CPUs, $vm_mem MiB"
cat >"$work/meta-data" <<EOF
instance-id: fps-boot-test-$(openssl rand -hex 4)
local-hostname: fps-boot-test
EOF
cloud-localds "$work/seed.img" "$work/user-data" "$work/meta-data"

# --- Boot a throwaway overlay of the image. ---
qemu-img create -q -f qcow2 -F qcow2 -b "$image" "$work/overlay.qcow2"

qemu-system-x86_64 \
	"${accel[@]}" -smp "$vm_smp" -m "$vm_mem" \
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

# --- Checks 3 and up: from inside the VM, over ssh. ---
ssh_opts=(-i "$work/key" -p "$ssh_port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
	-o LogLevel=ERROR -o BatchMode=yes -o ConnectTimeout=5)
scp_opts=(-i "$work/key" -P "$ssh_port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
	-o LogLevel=ERROR -o BatchMode=yes -o ConnectTimeout=5)
# --- Checks 1 and 2: /v1/capabilities answers on the management interface, and
# demands a token. The agent role has no controller and checks the opposite later. ---
if [[ "$role" != agent ]]; then
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
fi

# The agent role boots twice, because the kernel options of its tuning profile need
# a reboot. The wait ends when fps-tune records status=active, which happens on
# the second boot. The VM keeps its disk, ports, and seed across that reboot.
ssh_ok=no
ssh_tries=40
ssh_probe=true
if [[ "$role" == agent ]]; then
	ssh_tries=$((boot_wait * 2 / 3))
	ssh_probe='grep -qx status=active /etc/fps/tune.applied'
fi
for _ in $(seq 1 "$ssh_tries"); do
	# shellcheck disable=SC2029 # the probe is ours, and it runs on the VM
	if ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 "$ssh_probe" 2>/dev/null; then
		ssh_ok=yes
		break
	fi
	if ! kill -0 "$(cat "$pidfile")" 2>/dev/null; then break; fi
	sleep 3
done
if [[ "$role" == agent ]]; then
	code="$(api /v1/capabilities -o /dev/null -w '%{http_code}' || true)"
	if [[ "$code" != 200 && "$code" != 401 ]]; then
		pass "agent role: nothing answers on the forwarded management API port (HTTP status $code)"
	else
		fail "agent role: nothing answers on the forwarded management API port" "HTTP status $code"
	fi
fi
if [[ "$role" == agent && -n "$host_ctl_pid" ]]; then
	# The agent role proves mutual TLS: the controller on the host lists edge-1 as online,
	# and only a session with a certificate that the controller issued makes it online.
	registered=no
	deadline=$((SECONDS + boot_wait))
	while ((SECONDS < deadline)); do
		agents="$(curl -s -m 5 -H "Authorization: Bearer $token" "http://127.0.0.1:${FPS_TEST_HOST_API_PORT:-18446}/v1/agents" || true)"
		if grep -q '"edge-1"' <<<"$agents" && grep -Eq '"state": ?"(online|reserved|busy)"' <<<"$agents"; then
			registered=yes
			break
		fi
		sleep 3
	done
	if [[ "$registered" == yes ]]; then
		pass "agent role: edge-1 registered with the controller over mutual TLS (GET /v1/agents shows it online)"
	else
		fail "agent role: edge-1 registered with the controller over mutual TLS" "log: $(tail -n 5 "$work/ctl/controller.log" 2>/dev/null | tr '
' ' ')"
	fi
	if grep -q 'enroll' "$work/ctl/controller.log" && ! grep -q INSECURE "$work/ctl/controller.log"; then
		pass "agent role: the controller log shows an enrollment and no INSECURE line"
	else
		fail "agent role: the controller log shows an enrollment and no INSECURE line"
	fi
fi
if [[ "$ssh_ok" == yes ]]; then
	pass "ssh as fps-admin with the seed key works"
	vm_out="$work/vm-checks.out"
	: >"$vm_out"
	if scp "${scp_opts[@]}" "$root/image/tests/vm-checks.sh" "$root/image/tests/isolation-netns.sh" fps-admin@127.0.0.1:/tmp/ &&
		ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 'chmod +x /tmp/vm-checks.sh /tmp/isolation-netns.sh'; then
		# shellcheck disable=SC2029 # role is one of three fixed words
		ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 "sudo /tmp/vm-checks.sh $role" >"$vm_out" 2>&1 || true
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
	# IMG-04: the short HTTP and DNS run through the controller and the local agent.
	if [[ "$run_short" == 1 && "$role" == all-in-one ]]; then
		echo "NOTE the short HTTP and DNS run inside the VM"
		run_out="$work/vm-run.out"
		: >"$run_out"
		run_env=()
		for v in FPS_RUN_RATE FPS_RUN_STEADY_S FPS_RUN_WAIT_S; do
			if [[ -n "${!v:-}" ]]; then run_env+=("$v=${!v}"); fi
		done
		if scp "${scp_opts[@]}" "$root/image/tests/vm-run.sh" fps-admin@127.0.0.1:/tmp/ &&
			ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 'chmod +x /tmp/vm-run.sh'; then
			# shellcheck disable=SC2029 # run_env holds names and numbers that we set
			ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 "sudo env ${run_env[*]:-} /tmp/vm-run.sh /tmp/fps-run" >"$run_out" 2>&1 || true
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
		done <"$run_out"
		if ! grep -q '^PASS\|^FAIL' "$run_out"; then
			fail "the short HTTP and DNS run inside the VM ran" "no PASS or FAIL line came back"
		fi
		if [[ -n "$artifact_dir" ]]; then
			mkdir -p "$artifact_dir/run"
			cp "$run_out" "$artifact_dir/vm-run.out" 2>/dev/null || true
			# The files are owned by root in the VM, so copy through sudo tar.
			ssh "${ssh_opts[@]}" fps-admin@127.0.0.1 'sudo tar -C /tmp/fps-run -cf - .' 2>/dev/null |
				tar -C "$artifact_dir/run" -xf - 2>/dev/null || true
		fi
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
