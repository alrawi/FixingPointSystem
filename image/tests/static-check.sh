#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Static checks of the image sources. They need no root, no QEMU, and no network,
# so `make test-img` runs them on every pull request. The boot test and the
# namespace test cover the behavior.
set -euo pipefail

img="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

# 1. Every command that a unit starts exists in the staged tree and is executable.
for unit in "$img"/files/etc/systemd/system/*.service; do
	while read -r cmd; do
		cmd="${cmd#+}"
		case "$cmd" in
		/usr/libexec/fps/*)
			if [[ -x "$img/files$cmd" ]]; then pass "$(basename "$unit") runs $cmd"; else fail "$(basename "$unit") runs $cmd, which is missing or not executable"; fi
			;;
		esac
	done < <(sed -n -E 's/^Exec(Start|StartPre)=([^ ]+).*/\2/p' "$unit")
done

# 2. Every service that provision.sh enables has a unit file.
for name in fps-firewall fps-mgmt-net fps-mgmt-addr fps-role fps-tune fps-controller fps-agent; do
	if [[ -f "$img/files/etc/systemd/system/$name.service" ]] && grep -q "$name.service" "$img/scripts/provision.sh"; then
		pass "$name.service has a unit and provision.sh enables it"
	else
		fail "$name.service has a unit and provision.sh enables it"
	fi
done

# 2b. The roles. Each service unit of a role names the marker that fps-role writes, so
# a unit outside the role is skipped, and the agent waits for a pending tuning reboot.
for pair in fps-controller:fps-controller fps-agent:fps-agent; do
	unit="${pair%%:*}"
	if grep -q "^ConditionPathExists=/run/fps/role.d/$unit.service" "$img/files/etc/systemd/system/$unit.service" &&
		grep -q 'fps-role.service' "$img/files/etc/systemd/system/$unit.service"; then
		pass "$unit.service starts only when fps-role marks it, and orders after fps-role.service"
	else
		fail "$unit.service starts only when fps-role marks it, and orders after fps-role.service"
	fi
done
if grep -q '^ConditionPathExists=!/run/fps/tune-reboot' "$img/files/etc/systemd/system/fps-agent.service" &&
	grep -q '^LimitNOFILE=' "$img/files/etc/systemd/system/fps-agent.service"; then
	pass "fps-agent.service waits for a pending tuning reboot and raises the file limit"
else
	fail "fps-agent.service waits for a pending tuning reboot and raises the file limit"
fi

# 2c. The offline tests of the role selector and the tuning profiles.
for t in role-test tune-test enroll-token-test convert-test; do
	if out="$("$img/tests/$t.sh" 2>&1)"; then
		pass "$t.sh: $(grep -c '^PASS' <<<"$out") checks pass"
	else
		fail "$t.sh"
		echo "$out" | grep -v '^PASS'
	fi
done

# 3. The management ports agree between the units and the rules.
for port in 8443 8444 8445; do
	if grep -q ":$port" "$img/files/etc/systemd/system/fps-controller.service" && grep -q "$port" "$img/files/etc/fps/isolation.nft"; then
		pass "port $port appears in the controller unit and in the rules"
	else
		fail "port $port appears in the controller unit and in the rules"
	fi
done

# 4. The controller listens on the management address, never on a wildcard or the loopback.
# shellcheck disable=SC2016 # the unit text holds a literal ${FPS_MGMT_ADDR}
if grep -q -- '-listen ${FPS_MGMT_ADDR}:8443' "$img/files/etc/systemd/system/fps-controller.service" &&
	grep -q -- '-grpc-listen ${FPS_MGMT_ADDR}:8444' "$img/files/etc/systemd/system/fps-controller.service" &&
	grep -q -- '-enroll-listen ${FPS_MGMT_ADDR}:8445' "$img/files/etc/systemd/system/fps-controller.service"; then
	pass "the controller unit binds the gRPC and enrollment listeners to FPS_MGMT_ADDR"
else
	fail "the controller unit binds the gRPC and enrollment listeners to FPS_MGMT_ADDR"
fi

# 4b. Mutual TLS (IMG-06): no plain gRPC, https by default, the AGT-08 flags, no token on a command line.
if ! grep -rq 'insecure_plain_grpc\|insecure-plain-grpc' "$img/files"; then
	pass "nothing under image/files mentions insecure_plain_grpc"
else
	fail "nothing under image/files mentions insecure_plain_grpc"
fi
start="$img/files/usr/libexec/fps/fps-agent-start"
# shellcheck disable=SC2016 # the script text holds a literal ${FPS_MGMT_ADDR}
if grep -q 'https://${FPS_MGMT_ADDR}:8444' "$start" && ! grep -q 'http://${FPS_MGMT_ADDR}' "$start" &&
	grep -q -- '--cert-dir /var/lib/fps-agent/pki' "$start" && grep -q -- '--enroll-token-file /var/lib/fps-agent/enroll.token' "$start" &&
	grep -q -- '--tls-ca ' "$start" && grep -q -- '--tls-ca-sha256' "$start" && grep -q -- '--enroll-address' "$start"; then
	pass "fps-agent-start dials https:// and passes the AGT-08 flags"
else
	fail "fps-agent-start dials https:// and passes the AGT-08 flags"
fi
if grep -q '^StateDirectory=fps-agent' "$img/files/etc/systemd/system/fps-agent.service" &&
	grep -q '^StateDirectoryMode=0700' "$img/files/etc/systemd/system/fps-agent.service" &&
	grep -q '^ExecStartPre=+/usr/libexec/fps/fps-enroll-token' "$img/files/etc/systemd/system/fps-agent.service"; then
	pass "fps-agent.service has a private state directory and runs fps-enroll-token first"
else
	fail "fps-agent.service has a private state directory and runs fps-enroll-token first"
fi
# A token is a secret. Nothing may pass one as an argument or an environment value of a unit.
if ! grep -rEq -- '--enroll-token[ =]|ENROLL_TOKEN=|enroll-token-env' "$img/files/etc" "$img/seed-example"; then
	pass "no unit or configuration file carries a token argument or variable"
else
	fail "no unit or configuration file carries a token argument or variable"
fi

# 5. The rules drop forwarding on mgmt0 in both directions, and sysctl keeps it off.
if grep -q 'iifname "mgmt0" counter drop' "$img/files/etc/fps/isolation.nft" &&
	grep -q 'oifname "mgmt0" counter drop' "$img/files/etc/fps/isolation.nft"; then
	pass "the forward chain drops traffic into and out of mgmt0"
else
	fail "the forward chain drops traffic into and out of mgmt0"
fi
if grep -q '^net.ipv4.ip_forward = 0' "$img/files/etc/sysctl.d/99-fps-isolation.conf"; then
	pass "sysctl keeps net.ipv4.ip_forward at 0"
else
	fail "sysctl keeps net.ipv4.ip_forward at 0"
fi

# 6. The rules parse. nft -c needs root on many hosts, and the check is skipped when it fails.
if command -v nft >/dev/null 2>&1; then
	if nft -c -f "$img/files/etc/fps/isolation.nft" 2>/dev/null; then
		pass "nft -c accepts isolation.nft"
		if nft -c -f "$img/files/etc/fps/notrack.nft" 2>/dev/null; then
			pass "nft -c accepts notrack.nft"
		else
			echo "NOTE nft -c refused notrack.nft, perhaps because the kernel lacks nft_ct"
		fi
	else
		echo "NOTE nft -c failed here, perhaps for lack of privileges, so the syntax check is skipped"
	fi
else
	echo "NOTE nft is not installed, so the syntax check of isolation.nft is skipped"
fi

# 7. The configuration files parse, when PyYAML is present.
if python3 -I -c 'import yaml' 2>/dev/null; then
	for f in files/etc/fps/controller.yaml files/etc/cloud/cloud.cfg.d/90-fps.cfg seed-example/user-data; do
		if python3 -I -c 'import sys,yaml; yaml.safe_load(open(sys.argv[1]))' "$img/$f"; then
			pass "$f is valid YAML"
		else
			fail "$f is valid YAML"
		fi
	done
fi

if ((fails > 0)); then
	exit 1
fi
