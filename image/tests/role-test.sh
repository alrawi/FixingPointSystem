#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Offline test of /usr/libexec/fps/fps-role. It runs the script with a fake
# systemctl in a temporary directory, so it needs no root and no VM.
set -euo pipefail

img="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$img/files/usr/libexec/fps/fps-role"
work="$(mktemp -d "${TMPDIR:-/tmp}/fps-role-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

cat >"$work/systemctl" <<'EOF'
#!/bin/bash
echo "$*" >>"$FPS_TEST_CALLS"
EOF
chmod +x "$work/systemctl"

# run CASE ROLE_FILE_CONTENT|-  runs the script in a fresh directory; the exit code lands in $rc.
run() {
	local name="$1" content="$2"
	dir="$work/$name"
	mkdir -p "$dir"
	if [[ "$content" != "-" ]]; then printf '%s' "$content" >"$dir/role.env"; fi
	: >"$dir/calls"
	rc=0
	FPS_ROLE_FILE="$dir/role.env" FPS_RUN_DIR="$dir/run" FPS_ROLE_APPLIED="$dir/applied" \
		FPS_SYSTEMCTL="$work/systemctl" FPS_TEST_CALLS="$dir/calls" "$script" >"$dir/out" 2>&1 || rc=$?
}
markers() { find "$dir/run/role.d" -type f -printf '%f\n' 2>/dev/null | sort | tr '\n' ' '; }

run default -
if [[ $rc == 0 && "$(markers)" == "fps-agent.service fps-controller.service " && "$(cat "$dir/run/role")" == all-in-one ]]; then
	pass "no role file selects all-in-one with both services"
else
	fail "no role file selects all-in-one with both services ($(markers))"
fi
if grep -q 'enable --no-reload fps-controller.service' "$dir/calls" && grep -q 'enable --no-reload fps-agent.service' "$dir/calls" &&
	! grep -q disable "$dir/calls"; then
	pass "all-in-one enables both units and disables none"
else
	fail "all-in-one enables both units and disables none"
fi

run controller $'FPS_ROLE=controller\n'
if [[ $rc == 0 && "$(markers)" == "fps-controller.service " ]] && grep -q 'disable --no-reload fps-agent.service' "$dir/calls" &&
	grep -q 'enable --no-reload fps-controller.service' "$dir/calls" && grep -q '^role=controller' "$dir/applied"; then
	pass "controller role marks and enables only the controller, and disables the agent"
else
	fail "controller role marks and enables only the controller, and disables the agent"
fi

run agent $'# comment\nFPS_ROLE=agent\nFPS_CONTROLLER_URI="http://192.0.2.10:8444"\nFPS_AGENT_LABELS=\'role=agent site=lab\'\nFPS_AGENT_EXTRA_ARGS=--redial-limit 0\nOTHER=1\n'
if [[ $rc == 0 && "$(markers)" == "fps-agent.service " ]] && grep -q 'disable --no-reload fps-controller.service' "$dir/calls"; then
	pass "agent role marks and enables only the agent, and disables the controller"
else
	fail "agent role marks and enables only the agent, and disables the controller"
fi
# The checked values come back through a plain source, as fps-agent-start reads them.
got="$(
	# shellcheck source=/dev/null
	. "$dir/run/role.env"
	printf '%s|%s|%s|%s' "$FPS_ROLE" "$FPS_CONTROLLER_URI" "$FPS_AGENT_LABELS" "$FPS_AGENT_EXTRA_ARGS"
)"
if [[ "$got" == "agent|http://192.0.2.10:8444|role=agent site=lab|--redial-limit 0" ]]; then
	pass "role.env holds the checked values, quoted"
else
	fail "role.env holds the checked values, quoted (got $got)"
fi
if grep -q 'unknown key OTHER' "$dir/out"; then pass "an unknown key is reported and ignored"; else fail "an unknown key is reported and ignored"; fi

run agent-no-uri $'FPS_ROLE=agent\n'
if [[ $rc != 0 && -z "$(markers)" ]] && grep -q 'needs FPS_CONTROLLER_URI' "$dir/out"; then
	pass "the agent role without FPS_CONTROLLER_URI fails and starts nothing"
else
	fail "the agent role without FPS_CONTROLLER_URI fails and starts nothing"
fi

run bad-role $'FPS_ROLE=router\n'
if [[ $rc != 0 && -z "$(markers)" ]] && grep -q 'valid roles are' "$dir/out"; then
	pass "an unknown role fails and starts nothing"
else
	fail "an unknown role fails and starts nothing"
fi

run bad-uri $'FPS_ROLE=agent\nFPS_CONTROLLER_URI=ftp://x\n'
if [[ $rc != 0 && -z "$(markers)" ]]; then pass "a controller URI that is not http fails"; else fail "a controller URI that is not http fails"; fi

run bad-id $'FPS_ROLE=all-in-one\nFPS_AGENT_ID=a b\n'
if [[ $rc != 0 ]]; then pass "an agent id with a space fails"; else fail "an agent id with a space fails"; fi

run injection $'FPS_ROLE=controller\nFPS_AGENT_LABELS=$(touch '"$work"'/pwned)\n'
# shellcheck source=/dev/null
(. "$dir/run/role.env") >/dev/null 2>&1 || true
if [[ $rc == 0 && ! -e "$work/pwned" ]]; then
	pass "a command substitution in a value runs neither in fps-role nor when role.env is sourced"
else
	fail "a command substitution in a value runs neither in fps-role nor when role.env is sourced"
fi

run garbage $'this is not a setting\n'
if [[ $rc != 0 && -z "$(markers)" ]]; then pass "a line that is not KEY=VALUE fails"; else fail "a line that is not KEY=VALUE fails"; fi

# A second run after a failed one clears the markers of the first.
run rerun $'FPS_ROLE=controller\n'
printf 'FPS_ROLE=bogus\n' >"$dir/role.env"
rc=0
FPS_ROLE_FILE="$dir/role.env" FPS_RUN_DIR="$dir/run" FPS_ROLE_APPLIED="$dir/applied" \
	FPS_SYSTEMCTL="$work/systemctl" FPS_TEST_CALLS="$dir/calls" "$script" >/dev/null 2>&1 || rc=$?
if [[ $rc != 0 && -z "$(markers)" ]]; then pass "a failed run removes the markers of the run before"; else fail "a failed run removes the markers of the run before"; fi


run tls-keys $'FPS_ROLE=agent\nFPS_CONTROLLER_URI=https://192.0.2.10:8444\nFPS_CA_SHA256=sha256:AA:BB\n'
if [[ $rc != 0 ]]; then pass "a fingerprint that is not 64 hex digits fails"; else fail "a fingerprint that is not 64 hex digits fails"; fi

sha="$(printf 'ab%.0s' $(seq 1 32))"
run tls-ok "$(printf 'FPS_ROLE=agent\nFPS_CONTROLLER_URI=https://192.0.2.10:8444\nFPS_CA_SHA256=sha256:%s\nFPS_ENROLL_ADDRESS=192.0.2.10:8445\n' "$sha")"
if [[ $rc == 0 ]] && grep -qx "FPS_CA_SHA256=$sha" "$dir/run/role.env" && grep -q 'FPS_ENROLL_ADDRESS=192.0.2.10:8445' "$dir/run/role.env"; then
	pass "FPS_CA_SHA256 and FPS_ENROLL_ADDRESS reach role.env, and the fingerprint prefix is stripped"
else
	fail "FPS_CA_SHA256 and FPS_ENROLL_ADDRESS reach role.env, and the fingerprint prefix is stripped (rc $rc: $(cat "$dir/out") $(cat "$dir/run/role.env" 2>/dev/null))"
fi

run token-in-role $'FPS_ROLE=agent\nFPS_CONTROLLER_URI=https://192.0.2.10:8444\nFPS_ENROLL_TOKEN=hunter2-secret\n'
if [[ $rc != 0 && -z "$(markers)" ]] && ! grep -q 'hunter2' "$dir/out" && [[ ! -e "$dir/run/role.env" ]]; then
	pass "a token inside role.env is refused, and its value is not printed"
else
	fail "a token inside role.env is refused, and its value is not printed"
fi

run bad-enroll-addr $'FPS_ROLE=agent\nFPS_CONTROLLER_URI=https://192.0.2.10:8444\nFPS_ENROLL_ADDRESS=nonsense\n'
if [[ $rc != 0 ]]; then pass "an enrollment address without a port fails"; else fail "an enrollment address without a port fails"; fi

((fails == 0))
