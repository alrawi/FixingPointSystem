#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Offline test of /usr/libexec/fps/fps-tune: the sizing table, and the apply logic
# with fake update-grub, sysctl, nft, and systemctl commands. It needs no root and
# no VM. The boot test checks the real effect (image/boot-test.sh --role agent).
set -euo pipefail

img="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$img/files/usr/libexec/fps/fps-tune"
work="$(mktemp -d "${TMPDIR:-/tmp}/fps-tune-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

# --- The sizing table. ---
plan_field() { # plan_field ROLE CPUS MEM_MIB FIELD
	"$script" plan "$1" "$2" "$3" | sed -n "s/^$4=//p"
}
expect_plan() { # expect_plan ROLE CPUS MEM_MIB HOUSEKEEPING ISOLATED HUGEPAGES
	local got
	got="$(plan_field "$1" "$2" "$3" housekeeping_cpus)|$(plan_field "$1" "$2" "$3" isolated_cpus)|$(plan_field "$1" "$2" "$3" hugepages)"
	if [[ "$got" == "$4|$5|$6" ]]; then
		pass "plan $1, $2 CPUs, $3 MiB: housekeeping $4, isolated '$5', $6 hugepages"
	else
		fail "plan $1, $2 CPUs, $3 MiB: wanted $4|$5|$6, got $got"
	fi
}
expect_plan agent 1 512 0 "" 0
expect_plan agent 2 2048 0-1 "" 256
expect_plan agent 3 4096 0-2 "" 512
expect_plan agent 4 4096 0 1-3 512
expect_plan agent 4 3900 0 1-3 480
expect_plan agent 8 16384 0-1 2-7 2048
expect_plan agent 16 65536 0-3 4-15 8192
expect_plan agent 64 262144 0-15 16-63 32768
expect_plan agent 4 800 0 1-3 0
expect_plan controller 16 65536 0-15 "" 0
expect_plan all-in-one 16 65536 0-15 "" 0

cmdline_of() { plan_field "$1" "$2" "$3" cmdline; }
if [[ "$(cmdline_of agent 4 4096)" == "isolcpus=1-3 nohz_full=1-3 rcu_nocbs=1-3 hugepagesz=2M hugepages=512" ]]; then
	pass "the agent command line for 4 CPUs and 4096 MiB holds isolcpus, nohz_full, rcu_nocbs, and hugepages"
else
	fail "the agent command line for 4 CPUs and 4096 MiB (got '$(cmdline_of agent 4 4096)')"
fi
if [[ -z "$(cmdline_of controller 8 8192)" && -z "$(cmdline_of all-in-one 8 8192)" ]]; then
	pass "the controller and all-in-one profiles change no kernel option"
else
	fail "the controller and all-in-one profiles change no kernel option"
fi
if "$script" plan router 4 4096 >/dev/null 2>&1; then fail "an unknown role is refused"; else pass "an unknown role is refused"; fi
if "$script" plan agent 0 4096 >/dev/null 2>&1; then fail "zero CPUs are refused"; else pass "zero CPUs are refused"; fi

# --- The apply logic. ---
cat >"$work/fake" <<'EOF'
#!/bin/bash
# Records its name and arguments. FPS_TEST_FAIL names a command that fails.
name="${0##*/}"
echo "$name $*" >>"$FPS_TEST_CALLS"
[[ "${FPS_TEST_FAIL:-}" == "$name" ]] && exit 1
exit 0
EOF
chmod +x "$work/fake"
for n in update-grub sysctl nft systemctl; do ln -s fake "$work/$n"; done

dir="$work/apply"
mkdir -p "$dir/run"
: >"$dir/calls"
tune() { # tune ROLE CPUS MEM_KB CMDLINE  runs apply; the exit code lands in $rc
	printf '%s\n' "$1" >"$dir/run/role"
	printf 'BOOT_IMAGE=/vmlinuz root=UUID=x ro console=ttyS0 %s\n' "$4" >"$dir/cmdline"
	rc=0
	FPS_RUN_DIR="$dir/run" FPS_TUNE_STATE="$dir/tune.applied" FPS_TUNE_GRUB_FILE="$dir/grub.d/60-fps-tune.cfg" \
		FPS_TUNE_SYSCTL_FILE="$dir/90-fps-agent.conf" FPS_TUNE_NOTRACK_NFT="$dir/notrack.nft" \
		FPS_TUNE_CMDLINE_FILE="$dir/cmdline" FPS_TUNE_CONNTRACK_PROBE="$dir/conntrack_max" \
		FPS_TUNE_HUGEPAGES_FILE="$dir/nr_hugepages" FPS_TUNE_UPDATE_GRUB="$work/update-grub" \
		FPS_TUNE_SYSCTL="$work/sysctl" FPS_TUNE_NFT="$work/nft" FPS_SYSTEMCTL="$work/systemctl" \
		FPS_TUNE_CPUS="$2" FPS_TUNE_MEM_KB="$3" FPS_TEST_CALLS="$dir/calls" \
		"$script" apply >"$dir/out" 2>&1 || rc=$?
}
state() { sed -n "s/^$1=//p" "$dir/tune.applied"; }
calls() { grep -c "^$1" "$dir/calls" || true; }
reset_calls() { : >"$dir/calls"; }

want="isolcpus=1-3 nohz_full=1-3 rcu_nocbs=1-3 hugepagesz=2M hugepages=512"
mem=$((4096 * 1024))

# 1. First boot of an agent: the options are missing, so one reboot is asked.
tune agent 4 "$mem" ""
if [[ $rc == 0 && "$(state status)" == reboot-pending && -f "$dir/run/tune-reboot" && "$(calls 'update-grub')" == 1 &&
	"$(calls 'systemctl --no-block reboot')" == 1 ]] && grep -q "hugepages=512" "$dir/grub.d/60-fps-tune.cfg"; then
	pass "agent, first boot: writes the grub file, runs update-grub once, asks for one reboot, marks it"
else
	fail "agent, first boot: writes the grub file, runs update-grub once, asks for one reboot, marks it"
fi
if [[ "$(state cmdline)" == "$want" && "$(state sysctl)" == yes && -f "$dir/90-fps-agent.conf" ]] &&
	grep -q '^net.core.somaxconn' "$dir/90-fps-agent.conf" && grep -q '^net.ipv4.ip_local_port_range' "$dir/90-fps-agent.conf" &&
	grep -q '^fs.file-max' "$dir/90-fps-agent.conf" && [[ "$(calls sysctl)" == 1 ]]; then
	pass "agent, first boot: records the options and applies the sysctls (port range, somaxconn, file limits)"
else
	fail "agent, first boot: records the options and applies the sysctls"
fi
if [[ -z "$(state agent_cpus)" ]]; then pass "agent, first boot: no CPU pinning before the options run"; else fail "agent, first boot: no CPU pinning before the options run"; fi

# 2. The reboot happened but the options are still missing: no second reboot.
reset_calls
tune agent 4 "$mem" ""
if [[ $rc == 1 && "$(state status)" == failed && "$(calls 'systemctl')" == 0 && "$(calls 'update-grub')" == 0 &&
	! -f "$dir/run/tune-reboot" ]]; then
	pass "agent, options still missing after the reboot: status failed, no second reboot, no second update-grub"
else
	fail "agent, options still missing after the reboot: status failed, no second reboot, no second update-grub"
fi

# 3. The options run: active, pinned, and nothing changes on later boots.
reset_calls
tune agent 4 "$mem" "$want"
if [[ $rc == 0 && "$(state status)" == active && "$(state agent_cpus)" == 1-3 && "$(calls 'systemctl')" == 0 &&
	"$(calls 'update-grub')" == 0 && -z "$(state reboot_requested_for)" ]]; then
	pass "agent, options run: status active, agent pinned to 1-3, no reboot, no update-grub"
else
	fail "agent, options run: status active, agent pinned to 1-3, no reboot, no update-grub"
fi
echo 512 >"$dir/nr_hugepages"
tune agent 4 "$mem" "$want"
if [[ $rc == 0 && "$(state hugepages_allocated)" == 512 && "$(calls 'systemctl')" == 0 && "$(calls 'update-grub')" == 0 ]]; then
	pass "agent, second boot: idempotent, and the allocated hugepages are recorded"
else
	fail "agent, second boot: idempotent, and the allocated hugepages are recorded"
fi

# 4. A little less memory on one boot keeps the profile.
tune agent 4 "$((3900 * 1024 - 100 * 1024))" "$want"
if [[ $rc == 0 && "$(state status)" == active && "$(state mem_mib)" == 4096 && "$(calls 'systemctl')" == 0 ]]; then
	pass "agent, memory differs by less than one step: the profile stays and no reboot follows"
else
	fail "agent, memory differs by less than one step: the profile stays and no reboot follows (mem_mib $(state mem_mib), status $(state status))"
fi

# 5. The role changes to controller: the options go away, with one reboot.
reset_calls
tune controller 4 "$mem" "$want"
if [[ $rc == 0 && "$(state status)" == reboot-pending && ! -e "$dir/grub.d/60-fps-tune.cfg" && "$(calls 'update-grub')" == 1 &&
	"$(calls 'systemctl --no-block reboot')" == 1 && ! -e "$dir/90-fps-agent.conf" ]]; then
	pass "controller after agent: removes the grub file, the sysctl file, and asks for one reboot"
else
	fail "controller after agent: removes the grub file, the sysctl file, and asks for one reboot"
fi

# 6. A clean controller does nothing.
rm -f "$dir/tune.applied"
reset_calls
tune controller 8 "$((8192 * 1024))" ""
if [[ $rc == 0 && "$(state status)" == none && "$(state cmdline)" == "" && "$(calls 'systemctl')" == 0 && "$(calls 'update-grub')" == 0 &&
	"$(calls 'sysctl')" == 0 && "$(calls 'nft')" == 0 && "$(state grub_file)" == none && "$(state notrack)" == "no (role controller)" ]]; then
	pass "controller: nothing applied, no reboot, no sysctl, no nft"
else
	fail "controller: nothing applied, no reboot, no sysctl, no nft"
fi

# 7. All-in-one: sysctls only, no kernel option, no reboot.
reset_calls
tune all-in-one 4 "$mem" ""
if [[ $rc == 0 && "$(state status)" == none && "$(state sysctl)" == yes && "$(calls 'systemctl')" == 0 &&
	"$(calls 'update-grub')" == 0 && "$(state grub_file)" == none ]]; then
	pass "all-in-one: sysctls only, no kernel option, no reboot"
else
	fail "all-in-one: sysctls only, no kernel option, no reboot"
fi

# 8. Loopback notrack, only when nf_conntrack is loaded.
reset_calls
tune all-in-one 4 "$mem" ""
if [[ "$(calls 'nft')" == 0 && "$(state notrack)" == "no (nf_conntrack is not loaded)" ]]; then
	pass "no notrack table while nf_conntrack is not loaded"
else
	fail "no notrack table while nf_conntrack is not loaded"
fi
: >"$dir/conntrack_max"
reset_calls
tune all-in-one 4 "$mem" ""
if [[ "$(calls "nft -f $dir/notrack.nft")" == 1 && "$(state notrack)" == yes ]]; then
	pass "the notrack table loads when nf_conntrack is loaded"
else
	fail "the notrack table loads when nf_conntrack is loaded"
fi

# 9. update-grub fails: no reboot, status failed.
rm -f "$dir/tune.applied" "$dir/grub.d/60-fps-tune.cfg"
reset_calls
FPS_TEST_FAIL=update-grub tune agent 4 "$mem" ""
if [[ $rc == 1 && "$(state status)" == failed && "$(calls 'systemctl')" == 0 ]]; then
	pass "update-grub fails: status failed and no reboot"
else
	fail "update-grub fails: status failed and no reboot"
fi

# The shipped notrack file has the safe-reload header and the loopback rules.
nft_file="$img/files/etc/fps/notrack.nft"
if grep -q '^table inet fps_notrack$' "$nft_file" && grep -q '^delete table inet fps_notrack$' "$nft_file" &&
	grep -q 'iif "lo" notrack' "$nft_file" && grep -q 'oif "lo" notrack' "$nft_file"; then
	pass "notrack.nft exempts the loopback in a table of its own and is safe to load again"
else
	fail "notrack.nft exempts the loopback in a table of its own and is safe to load again"
fi

((fails == 0))
