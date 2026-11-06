#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Proves that the isolation rules stop a router from forwarding between a
# management interface and a test interface. It needs root, ip, nft, ping, curl, and python3.
#
#   sudo image/tests/isolation-netns.sh [RULES]      RULES defaults to the shipped file
#
# The test builds a router in a network namespace with two interfaces, named
# mgmt0 and test0 as on the appliance, and one host behind each. With forwarding
# on and no rules, the two hosts reach each other, which proves that the router
# works. After the rules load, the same pings fail in both directions, and a ping
# between the router and a host still works. The script prints one PASS or FAIL
# line per check and exits with 1 when any check fails.
set -euo pipefail

rules="${1:-$(dirname "$0")/../files/etc/fps/isolation.nft}"
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

if [[ "$(id -u)" != 0 ]]; then
	echo "FAIL isolation-netns needs root" >&2
	exit 1
fi

readonly R=fpsit-router A=fpsit-mgmt-host B=fpsit-test-host
cleanup() {
	local ns
	for ns in "$R" "$A" "$B"; do
		ip netns del "$ns" 2>/dev/null || true
	done
}
server_pid=""
tmp="$(mktemp -d)"
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$tmp"; cleanup' EXIT
cleanup

ip netns add "$R"
ip netns add "$A"
ip netns add "$B"
ip -n "$R" link set lo up
ip -n "$A" link set lo up
ip -n "$B" link set lo up

# One veth pair per side. The router end carries the appliance interface name.
ip link add vA netns "$A" type veth peer name mgmt0 netns "$R"
ip link add vB netns "$B" type veth peer name test0 netns "$R"
ip -n "$A" addr add 10.77.1.2/24 dev vA
ip -n "$B" addr add 10.77.2.2/24 dev vB
ip -n "$R" addr add 10.77.1.1/24 dev mgmt0
ip -n "$R" addr add 10.77.2.1/24 dev test0
ip -n "$A" link set vA up
ip -n "$B" link set vB up
ip -n "$R" link set mgmt0 up
ip -n "$R" link set test0 up
ip -n "$A" route add default via 10.77.1.1
ip -n "$B" route add default via 10.77.2.1
ip netns exec "$R" sysctl -qw net.ipv4.ip_forward=1

ping_from() { ip netns exec "$1" ping -c 2 -W 2 -q "$2" >/dev/null 2>&1; }
# A listener on a management port of the router, on every address, as a controller
# that ignored the management address would be.
fetch_from() { ip netns exec "$1" curl -s -m 3 -o /dev/null "http://$2:8443/"; }
(cd "$tmp" && exec ip netns exec "$R" python3 -m http.server 8443 --bind 0.0.0.0 >/dev/null 2>&1) &
server_pid=$!
sleep 1

# Control: without rules the router forwards, so a later failure means the rules.
if ping_from "$A" 10.77.2.2 && ping_from "$B" 10.77.1.2; then
	pass "control: the router forwards between the interfaces without rules"
else
	fail "control: the router forwards between the interfaces without rules"
fi

if fetch_from "$B" 10.77.2.1 && fetch_from "$A" 10.77.1.1; then
	pass "control: port 8443 answers on both interfaces without rules"
else
	fail "control: port 8443 answers on both interfaces without rules"
fi

ip netns exec "$R" nft -f "$rules"

if ping_from "$A" 10.77.2.2; then
	fail "management host cannot reach the test side through the router"
else
	pass "management host cannot reach the test side through the router"
fi
if ping_from "$B" 10.77.1.2; then
	fail "test host cannot reach the management side through the router"
else
	pass "test host cannot reach the management side through the router"
fi
if ping_from "$A" 10.77.1.1 && ping_from "$B" 10.77.2.1; then
	pass "the router itself still answers on both interfaces"
else
	fail "the router itself still answers on both interfaces"
fi
if fetch_from "$B" 10.77.2.1; then
	fail "port 8443 is closed on the test interface"
else
	pass "port 8443 is closed on the test interface"
fi
if fetch_from "$A" 10.77.1.1; then
	pass "port 8443 still answers on the management interface"
else
	fail "port 8443 still answers on the management interface"
fi
dropped="$(ip netns exec "$R" nft list chain inet fps_isolation forward | grep -c 'packets [1-9]' || true)"
if [[ "$dropped" -ge 1 ]]; then
	pass "the forward chain counted the dropped packets"
else
	fail "the forward chain counted the dropped packets"
fi

if ((fails > 0)); then
	exit 1
fi
