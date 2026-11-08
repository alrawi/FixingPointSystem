#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Checks of the appliance from the inside. image/boot-test.sh copies this script
# and isolation-netns.sh to the VM and runs it there as root. Each check prints one
# line that starts with PASS or FAIL, and the script exits with 1 when any fails.
set -uo pipefail

fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1${2:+ ($2)}"
	fails=$((fails + 1))
}
check() { # check DESCRIPTION COMMAND...
	local what="$1"
	shift
	if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi
}

token="$(cat /etc/fps/admin.token)"
mgmt_addr="$(ip -4 -o addr show dev mgmt0 scope global | awk '{print $4}' | cut -d/ -f1 | head -n 1)"

# Kernel requirement of AGT-06.
kernel="$(uname -r)"
if [[ "$(printf '6.3\n%s\n' "${kernel%%-*}" | sort -V | head -n 1)" == "6.3" ]]; then
	pass "kernel $kernel is Linux 6.3 or newer"
else
	fail "kernel $kernel is older than Linux 6.3"
fi

# Interfaces: mgmt0 holds the address, and every other Ethernet interface has none.
check "mgmt0 has an IPv4 address ($mgmt_addr)" test -n "$mgmt_addr"
tests_found=0
for dev in /sys/class/net/*; do
	name="${dev##*/}"
	[[ -e "$dev/device" && "$name" != mgmt0 ]] || continue
	tests_found=$((tests_found + 1))
	if [[ -z "$(ip -4 -o addr show dev "$name")" ]]; then
		pass "test interface $name has no IPv4 address"
	else
		fail "test interface $name has no IPv4 address" "$(ip -4 -o addr show dev "$name")"
	fi
done
check "the VM has at least one test interface" test "$tests_found" -ge 1

# The controller listens on the management address and nowhere else.
listeners="$(ss -H -ltn 'sport = :8443 or sport = :8444' | awk '{print $4}' | sort -u)"
expected="$(printf '%s:8443\n%s:8444\n' "$mgmt_addr" "$mgmt_addr" | sort -u)"
if [[ "$listeners" == "$expected" ]]; then
	pass "ports 8443 and 8444 listen on $mgmt_addr only"
else
	fail "ports 8443 and 8444 listen on $mgmt_addr only" "found: $(echo "$listeners" | tr '\n' ' ')"
fi
check "the loopback does not answer on port 8443" bash -c '! curl -s -m 3 -o /dev/null http://127.0.0.1:8443/healthz'
check "the management address answers GET /healthz" curl -sf -m 5 "http://$mgmt_addr:8443/healthz"
if [[ "$(curl -s -m 5 -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $token" "http://$mgmt_addr:8443/v1/capabilities")" == 200 ]]; then
	pass "GET /v1/capabilities answers 200 on the management address"
else
	fail "GET /v1/capabilities answers 200 on the management address"
fi

# Services and the local agent.
check "fps-controller.service is active" systemctl is-active --quiet fps-controller.service
check "fps-agent.service is active" systemctl is-active --quiet fps-agent.service
registered=no
for _ in $(seq 1 30); do
	if curl -s -m 5 -H "Authorization: Bearer $token" "http://$mgmt_addr:8443/v1/agents" | grep -q '"local"'; then
		registered=yes
		break
	fi
	sleep 1
done
check "the local agent registered with the controller" test "$registered" = yes

# Forwarding is off, and the rules are loaded.
check "net.ipv4.ip_forward is 0" test "$(sysctl -n net.ipv4.ip_forward)" = 0
check "net.ipv6.conf.all.forwarding is 0" test "$(sysctl -n net.ipv6.conf.all.forwarding)" = 0
check "the nftables table inet fps_isolation is loaded" nft list table inet fps_isolation
check "fps-firewall.service is active" systemctl is-active --quiet fps-firewall.service

# The real rules keep a router from forwarding between mgmt0 and a test interface.
if [[ -x /tmp/isolation-netns.sh ]]; then
	while IFS= read -r line; do
		echo "$line"
		[[ "$line" == FAIL* ]] && fails=$((fails + 1))
	done < <(/tmp/isolation-netns.sh /etc/fps/isolation.nft 2>&1)
else
	fail "isolation-netns.sh is present on the VM"
fi

((fails == 0))
