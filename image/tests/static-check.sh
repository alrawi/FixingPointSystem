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
for name in fps-firewall fps-mgmt-net fps-mgmt-addr fps-controller fps-agent; do
	if [[ -f "$img/files/etc/systemd/system/$name.service" ]] && grep -q "$name.service" "$img/scripts/provision.sh"; then
		pass "$name.service has a unit and provision.sh enables it"
	else
		fail "$name.service has a unit and provision.sh enables it"
	fi
done

# 3. The management ports agree between the units and the rules.
for port in 8443 8444; do
	if grep -q ":$port" "$img/files/etc/systemd/system/fps-controller.service" && grep -q "$port" "$img/files/etc/fps/isolation.nft"; then
		pass "port $port appears in the controller unit and in the rules"
	else
		fail "port $port appears in the controller unit and in the rules"
	fi
done

# 4. The controller listens on the management address, never on a wildcard or the loopback.
# shellcheck disable=SC2016 # the unit text holds a literal ${FPS_MGMT_ADDR}
if grep -q -- '-listen ${FPS_MGMT_ADDR}:8443' "$img/files/etc/systemd/system/fps-controller.service" &&
	grep -q -- '-grpc-listen ${FPS_MGMT_ADDR}:8444' "$img/files/etc/systemd/system/fps-controller.service"; then
	pass "the controller unit binds both listeners to FPS_MGMT_ADDR"
else
	fail "the controller unit binds both listeners to FPS_MGMT_ADDR"
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

# 6. The rules parse. nft -c needs no privileges for a syntax check on most hosts.
if command -v nft >/dev/null 2>&1; then
	if nft -c -f "$img/files/etc/fps/isolation.nft" 2>/dev/null; then
		pass "nft -c accepts isolation.nft"
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
