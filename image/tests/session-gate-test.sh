#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Offline test of the agent session gate of image/tests/vm-run.sh (IMG-07). It feeds the
# gate journals in the format of `journalctl -o json` and needs no root and no VM.
#
# The replay cases use the journals of the failed job 129666780207 of image.yml (2026-11-10):
# the controller registered session 3 of the agent at 48.835 s, the run went out at 53.911 s,
# and the agent opened the session at 53.912 s, one millisecond after its watchdog had
# counted five seconds of silence. The gate must refuse that moment and accept an earlier
# one, and it must refuse every moment when the agent never reaches the controller.
set -euo pipefail

img="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
. "$img/tests/vm-run.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/fps-gate-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

# journal FILE LINES...: each line is "SECONDS JSON", the monotonic time of the entry and
# the log line that the unit wrote. The file gets one journald JSON object per line.
journal() {
	local file="$1"
	shift
	python3 -I -c 'import json, sys
for spec in sys.argv[1:]:
    sec, _, line = spec.partition(" ")
    print(json.dumps({"MESSAGE": line, "__MONOTONIC_TIMESTAMP": str(int(float(sec) * 1e6)), "_SYSTEMD_UNIT": "x.service"}))' "$@" >"$file"
}
agent_line() { # agent_line MESSAGE SESSION
	printf '{"timestamp":"2026-11-10T06:22:42Z","level":"INFO","fields":{"message":"%s","session":%s},"target":"fps_agent::session"}' "$1" "$2"
}
ctl_line() { # ctl_line MSG SESSION
	printf '{"time":"2026-11-10T06:22:42Z","level":"INFO","msg":"%s","agent_id":"local","session":%s}' "$1" "$2"
}

# verdict_at NOW: the gate verdict at NOW seconds for the journals in $work.
verdict_at() {
	# shellcheck disable=SC2154 # stable_s and fresh_s come from vm-run.sh
	session_verdict "$work/agent.json" "$work/controller.json" "$(python3 -I -c 'import sys; print(int(float(sys.argv[1]) * 1e6))' "$1")" "$stable_s" "$fresh_s"
}
expect() { # expect NAME KIND NOW
	local got
	got="$(verdict_at "$3")"
	if [[ "${got%% *}" == "$2" ]]; then
		pass "$1 -> ${got%% *}"
	else
		fail "$1: wanted $2, got '$got'"
	fi
}

# The journals of the failed job, on a monotonic scale that starts at 100 s (37.578 s -> 137.578 s).
# Session 1 and 2 ended with "no heartbeat" and opened:false. Session 3 is the one that met the run.
replay_agent=(
	"137.558 $(agent_line enrolling 0)"
	"142.625 $(agent_line 'session lost' 1)"
	"148.625 $(agent_line 'session lost' 2)"
)
replay_ctl=(
	"137.579 $(ctl_line 'agent online' 1)"
	"142.624 $(ctl_line 'agent offline' 1)"
	"142.768 $(ctl_line 'agent online' 2)"
	"148.624 $(ctl_line 'agent offline' 2)"
	"148.835 $(ctl_line 'agent online' 3)"
)
journal "$work/agent.json" "${replay_agent[@]}"
journal "$work/controller.json" "${replay_ctl[@]}"

# 1. The moment of the failure: the first command at 153.911 s, session 3 is 5.08 s old.
expect "replay: the run of the failed job (session 3, 5.08 s old, never opened)" wait 153.911
# 2. The same session, one second after it began: the first command flushes it in time.
expect "replay: the same session 1.0 s after it began" fresh 149.835
expect "replay: the same session 1.97 s after it began" fresh 150.8
expect "replay: the same session just past the fresh limit (2.2 s)" wait 151.035
# 3. The cycle of the failed job: every session of it is unopened, so a poll finds a young
# session at once, within the first fraction of a second after the controller registers it.
found=no
for t in 143.0 143.2 143.5 144.0 145.0 146.0 147.0 148.0 148.7; do
	if [[ "$(verdict_at "$t")" == fresh* ]]; then
		found=yes
		break
	fi
done
if [[ "$found" == yes ]]; then
	pass "replay: a young session shows up within the first cycle (session 2 at 142.768 s)"
else
	fail "replay: a young session shows up within the first cycle"
fi
# 4. The old session before the controller registered the next one.
expect "replay: session 2 at 4.9 s, just before it is swept" wait 147.7

# 5. A session that the agent opened and held for the stable age is stable.
journal "$work/agent.json" "100.000 $(agent_line 'session open' 1)"
journal "$work/controller.json" "99.990 $(ctl_line 'agent online' 1)"
expect "an opened session at 3.0 s" stable 103.0
expect "an opened session at 1.0 s is not stable yet" wait 101.0
expect "an opened session at 60 s" stable 160.0
# 6. The journals of two units reach journald in either order: the agent entry may be 100 ms older.
journal "$work/agent.json" "99.950 $(agent_line 'session open' 1)"
journal "$work/controller.json" "100.000 $(ctl_line 'agent online' 1)"
expect "the agent entry reaches the journal 50 ms before the controller entry" stable 104.0
# 7. An open session of an earlier controller session does not count for the new one.
journal "$work/agent.json" "100.000 $(agent_line 'session open' 1)" "110.000 $(agent_line 'session lost' 1)"
journal "$work/controller.json" "99.990 $(ctl_line 'agent online' 1)" "110.000 $(ctl_line 'agent offline' 1)" "110.300 $(ctl_line 'agent online' 2)"
expect "a lost session, then a new unopened one 0.5 s old" fresh 110.8
expect "a lost session, then a new unopened one 4 s old" wait 114.3
journal "$work/agent.json" "100.000 $(agent_line 'session open' 1)"
journal "$work/controller.json" "99.990 $(ctl_line 'agent online' 1)" "110.000 $(ctl_line 'agent offline' 1)"
expect "the controller marked the agent offline" wait 110.5

# 8. The agent cannot reach the controller: no event at all, or only offline events.
journal "$work/agent.json" "100.000 $(agent_line enrolling 0)"
journal "$work/controller.json"
for t in 100.5 130.0 1000.0; do
	expect "the controller never saw the agent (at $t s)" wait "$t"
done
got="$(verdict_at 130.0)"
if [[ "$got" == *"has not seen the agent local online"* ]]; then
	pass "the reason names the missing session ($got)"
else
	fail "the reason names the missing session ($got)"
fi
# Another agent id and noise lines never count for the agent local.
journal "$work/controller.json" "100.000 $(ctl_line 'agent online' 1 | sed 's/"local"/"edge-9"/')" "100.100 not json at all"
expect "a session of another agent, and a line that is no JSON" wait 100.5
# Missing journal files do not crash the gate.
rm -f "$work/agent.json" "$work/controller.json"
expect "missing journals" wait 100.0

# 9. wait_agent_session fails after its wait when the journals never show a session. A fake
# journalctl answers with an empty journal, so the loop runs to its deadline.
mkdir -p "$work/bin"
cat >"$work/bin/journalctl" <<'EOF'
#!/bin/bash
exit 0
EOF
chmod +x "$work/bin/journalctl"
start=$SECONDS
if out="$(PATH="$work/bin:$PATH" wait_agent_session 2)"; then
	fail "wait_agent_session fails when the agent never reaches the controller ($out)"
else
	pass "wait_agent_session fails when the agent never reaches the controller ($out)"
fi
if ((SECONDS - start >= 2 && SECONDS - start <= 6)); then
	pass "wait_agent_session gives up at its limit (about $((SECONDS - start)) s)"
else
	fail "wait_agent_session gives up at its limit (took $((SECONDS - start)) s)"
fi
# And it succeeds at once on a journal with an opened session, which the fake journalctl prints
# with the current monotonic time minus 10 s.
cat >"$work/bin/journalctl" <<'EOF'
#!/bin/bash
now=$(python3 -I -c 'import time; print(int(time.clock_gettime(time.CLOCK_MONOTONIC) * 1e6))')
old=$((now - 10000000))
case "$*" in
*fps-agent*)
	python3 -I -c 'import json, sys; print(json.dumps({"MESSAGE": "{\"fields\":{\"message\":\"session open\",\"session\":1}}", "__MONOTONIC_TIMESTAMP": sys.argv[1]}))' "$old"
	;;
*fps-controller*)
	python3 -I -c 'import json, sys; print(json.dumps({"MESSAGE": "{\"msg\":\"agent online\",\"agent_id\":\"local\",\"session\":1}", "__MONOTONIC_TIMESTAMP": sys.argv[1]}))' "$((old - 5000))"
	;;
esac
EOF
if out="$(PATH="$work/bin:$PATH" wait_agent_session 5)" && [[ "$out" == stable* ]]; then
	pass "wait_agent_session returns stable for an agent that holds its session ($out)"
else
	fail "wait_agent_session returns stable for an agent that holds its session ($out)"
fi

((fails == 0))
