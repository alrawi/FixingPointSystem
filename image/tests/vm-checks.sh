#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Checks of the appliance from the inside. image/boot-test.sh copies this script
# and isolation-netns.sh to the VM and runs it there as root. Each check prints one
# line that starts with PASS or FAIL, and the script exits with 1 when any fails.
#
#   vm-checks.sh [ROLE]      ROLE is controller, agent, or all-in-one (the default)
#
# The role decides which services must run and which tuning must show.
set -uo pipefail

role="${1:-all-in-one}"
case "$role" in
controller | agent | all-in-one) ;;
*)
	echo "FAIL unknown role: $role"
	exit 2
	;;
esac

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

# The controller listens on the management address and nowhere else. The agent
# role has no controller, so nothing listens on those ports.
listeners="$(ss -H -ltn 'sport = :8443 or sport = :8444' | awk '{print $4}' | sort -u)"
if [[ "$role" == agent ]]; then
	if [[ -z "$listeners" ]]; then
		pass "nothing listens on ports 8443 and 8444 in the agent role"
	else
		fail "nothing listens on ports 8443 and 8444 in the agent role" "found: $(echo "$listeners" | tr '\n' ' ')"
	fi
else
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
fi

# Services and the local agent.
unit_runs() { systemctl is-active --quiet "$1" && pgrep -x "${1%.service}" >/dev/null; }
unit_off() { ! systemctl is-active --quiet "$1" && ! pgrep -x "${1%.service}" >/dev/null && ! systemctl is-enabled --quiet "$1"; }
if [[ "$role" != controller ]]; then
	# fps-tune writes status=active before the agent starts, so the agent may be a moment late.
	for _ in $(seq 1 60); do
		unit_runs fps-agent.service && break
		sleep 1
	done
fi
if [[ "$role" == agent ]]; then
	check "fps-controller.service is inactive, disabled, and has no process" unit_off fps-controller.service
else
	check "fps-controller.service is active" unit_runs fps-controller.service
fi
if [[ "$role" == controller ]]; then
	check "fps-agent.service is inactive, disabled, and has no process" unit_off fps-agent.service
else
	check "fps-agent.service is active" unit_runs fps-agent.service
fi
if [[ "$role" == all-in-one ]]; then
	registered=no
	for _ in $(seq 1 30); do
		if curl -s -m 5 -H "Authorization: Bearer $token" "http://$mgmt_addr:8443/v1/agents" | grep -q '"local"'; then
			registered=yes
			break
		fi
		sleep 1
	done
	check "the local agent registered with the controller" test "$registered" = yes
fi

# Forwarding is off, and the rules are loaded.
check "net.ipv4.ip_forward is 0" test "$(sysctl -n net.ipv4.ip_forward)" = 0
check "net.ipv6.conf.all.forwarding is 0" test "$(sysctl -n net.ipv6.conf.all.forwarding)" = 0
check "the nftables table inet fps_isolation is loaded" nft list table inet fps_isolation
check "fps-firewall.service is active" systemctl is-active --quiet fps-firewall.service

# Role and tuning (IMG-02). The state files record what fps-role and fps-tune did.
state() { sed -n "s/^$1=//p" "$2" 2>/dev/null | head -n 1; }
no_notrack_table() { ! nft list table inet fps_notrack; }
role_services_ok() { systemctl is-active --quiet fps-role.service && ! systemctl is-failed --quiet fps-tune.service; }
sysctl_is() { [[ "$(sysctl -n "$1")" == "$2" ]]; }
agent_sysctls() {
	sysctl_is net.core.somaxconn 65535 && sysctl_is net.ipv4.ip_local_port_range $'1024\t65535' && sysctl_is fs.file-max 4194304
}
agent_sysctls_nr_open() { agent_sysctls && sysctl_is fs.nr_open 4194304; }
controller_sysctls_absent() { [[ ! -e /etc/sysctl.d/90-fps-agent.conf ]] && ! sysctl_is net.core.somaxconn 65535; }
tune_recorded() { [[ "$(state status /etc/fps/tune.applied)" == "$1" && "$(state cmdline /etc/fps/tune.applied)" == "$2" ]]; }
agent_nofile() {
	local pid
	pid="$(pgrep -x fps-agent | head -n 1)"
	[[ "$(awk '/^Max open files/ {print $4}' "/proc/$pid/limits")" == 4194304 ]]
}
managed_re='^(isolcpus|nohz_full|rcu_nocbs|hugepagesz|hugepages)='
cmdline_managed="$(tr ' ' '\n' </proc/cmdline | grep -E "$managed_re" | tr '\n' ' ')"
check "fps-role recorded the role $role" test "$(state role /etc/fps/role.applied)" = "$role"
check "fps-tune recorded the role $role" test "$(state role /etc/fps/tune.applied)" = "$role"
check "no tuning reboot is pending" test ! -e /run/fps/tune-reboot
check "fps-role.service is active and fps-tune.service did not fail" role_services_ok
tune_cpus="$(state cpus /etc/fps/tune.applied)"
tune_mem="$(state mem_mib /etc/fps/tune.applied)"
plan="$(/usr/libexec/fps/fps-tune plan "$role" "$tune_cpus" "$tune_mem")"
plan_value() { sed -n "s/^$1=//p" <<<"$plan"; }
case "$role" in
controller)
	check "controller: no kernel option of the tuning profile on /proc/cmdline" test -z "$cmdline_managed"
	check "controller: no hugepages" test "$(cat /proc/sys/vm/nr_hugepages)" = 0
	check "controller: no sysctl file and the default somaxconn" controller_sysctls_absent
	check "controller: no notrack table" no_notrack_table
	check "controller: no grub drop-in of fps-tune" test ! -e /etc/default/grub.d/60-fps-tune.cfg
	check "controller: tuning status is none" test "$(state status /etc/fps/tune.applied)" = none
	;;
all-in-one)
	check "all-in-one: no kernel option of the tuning profile on /proc/cmdline" test -z "$cmdline_managed"
	check "all-in-one: no grub drop-in of fps-tune, so no reboot was needed" test ! -e /etc/default/grub.d/60-fps-tune.cfg
	check "all-in-one: tuning status is none" test "$(state status /etc/fps/tune.applied)" = none
	check "all-in-one: the agent sysctls hold (somaxconn, port range, file-max)" agent_sysctls
	;;
agent)
	want_cmdline="$(plan_value cmdline)"
	missing=""
	for opt in $want_cmdline; do
		grep -qw -- "$opt" /proc/cmdline || missing="$missing $opt"
	done
	if [[ -n "$want_cmdline" && -z "$missing" ]]; then
		pass "agent: /proc/cmdline holds $want_cmdline"
	else
		fail "agent: /proc/cmdline holds the options of the profile" "wanted '$want_cmdline', missing '$missing'"
	fi
	check "agent: tuning status is active, and fps-tune recorded the options" tune_recorded active "$want_cmdline"
	want_pages="$(plan_value hugepages)"
	if [[ "$want_pages" -gt 0 && "$(cat /proc/sys/vm/nr_hugepages)" == "$want_pages" ]] &&
		[[ "$(awk '/^HugePages_Total:/ {print $2}' /proc/meminfo)" == "$want_pages" ]]; then
		pass "agent: $want_pages hugepages of 2 MiB are reserved ($tune_cpus CPUs, $tune_mem MiB)"
	else
		fail "agent: $want_pages hugepages of 2 MiB are reserved" "nr_hugepages $(cat /proc/sys/vm/nr_hugepages)"
	fi
	want_iso="$(plan_value isolated_cpus)"
	if [[ -n "$want_iso" ]]; then
		check "agent: the kernel isolates CPUs $want_iso" test "$(cat /sys/devices/system/cpu/isolated)" = "$want_iso"
		check "agent: nohz_full CPUs are $want_iso" test "$(cat /sys/devices/system/cpu/nohz_full)" = "$want_iso"
		agent_pid="$(pgrep -x fps-agent | head -n 1)"
		check "agent: fps-agent runs on the isolated CPUs only" test "$(taskset -cp "$agent_pid" | sed 's/.*: //')" = "$want_iso"
	else
		pass "agent: $tune_cpus CPUs are too few to isolate, so the profile isolates none"
	fi
	check "agent: the agent sysctls hold (somaxconn, port range, file-max, nr_open)" agent_sysctls_nr_open
	check "agent: fps-agent has a file limit of 4194304" agent_nofile
	if [[ -e /proc/sys/net/netfilter/nf_conntrack_max ]]; then
		check "agent: nf_conntrack is loaded, so the loopback notrack table is loaded" nft list table inet fps_notrack
	else
		echo "NOTE nf_conntrack is not loaded in this VM, so fps-tune correctly leaves the notrack table out"
		check "agent: without nf_conntrack no notrack table exists" no_notrack_table
	fi
	# Idempotence: a second run on a VM that already runs the options changes nothing and asks for no reboot.
	grub_before="$(stat -c %Y /etc/default/grub.d/60-fps-tune.cfg 2>/dev/null)"
	boot_before="$(cat /proc/sys/kernel/random/boot_id)"
	/usr/libexec/fps/fps-tune apply >/dev/null 2>&1
	rerun_rc=$?
	sleep 2
	if [[ "$rerun_rc" == 0 && "$(state status /etc/fps/tune.applied)" == active && ! -e /run/fps/tune-reboot &&
		"$(stat -c %Y /etc/default/grub.d/60-fps-tune.cfg 2>/dev/null)" == "$grub_before" &&
		"$(cat /proc/sys/kernel/random/boot_id)" == "$boot_before" ]]; then
		pass "agent: a second fps-tune apply changes nothing and does not reboot"
	else
		fail "agent: a second fps-tune apply changes nothing and does not reboot"
	fi
	;;
esac

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
