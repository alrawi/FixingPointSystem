#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Offline test of /usr/libexec/fps/fps-enroll-token and of the arguments that
# fps-agent-start builds. It runs both with fake commands in a temporary directory,
# so it needs no root and no VM. It also proves that the token reaches no command line.
set -euo pipefail

img="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
helper="$img/files/usr/libexec/fps/fps-enroll-token"
start="$img/files/usr/libexec/fps/fps-agent-start"
work="$(mktemp -d "${TMPDIR:-/tmp}/fps-enroll-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

secret="SECRET-TOKEN-VALUE-0123456789abcdef"
# A fake fps-controller: it records its arguments, answers `token` on standard output
# only, and writes the expiry to standard error, as the real subcommand does.
cat >"$work/fps-controller" <<EOF
#!/bin/bash
echo "\$*" >>"\$FPS_TEST_CALLS"
if [[ "\$1" == token ]]; then
	echo "expires 2030-01-01T00:00:00Z" >&2
	echo "$secret"
else
	exit 3
fi
EOF
chmod +x "$work/fps-controller"

# run NAME ROLE [WITH_SOCKET]: a fresh state, with the role in a fake role.env.
run() {
	name="$1"
	dir="$work/$name"
	mkdir -p "$dir"
	: >"$dir/calls"
	if [[ "$2" != - ]]; then printf 'FPS_ROLE=%s\nFPS_AGENT_ID=edge-7\n' "$2" >"$dir/role.env"; else : >"$dir/role.env"; fi
	if [[ "${3:-}" == socket ]]; then python3 -I -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' "$dir/admin.sock"; fi
	rc=0
	FPS_AGENT_STATE_DIR="$dir/state" FPS_STAGED_TOKEN="$dir/staged" FPS_RUN_ROLE_ENV="$dir/role.env" \
		FPS_AGENT_ENV="$dir/none.env" FPS_ADMIN_SOCKET="$dir/admin.sock" FPS_CONTROLLER_CMD="$work/fps-controller" \
		FPS_CONTROLLER_YAML="$dir/controller.yaml" FPS_SOCKET_WAIT=1 FPS_TEST_CALLS="$dir/calls" \
		"$helper" >"$dir/out" 2>"$dir/err" || rc=$?
}
mode() { stat -c %a "$1"; }

run mint all-in-one socket
if [[ $rc == 0 && "$(cat "$dir/state/enroll.token")" == "$secret" && "$(mode "$dir/state/enroll.token")" == 600 &&
	"$(mode "$dir/state")" == 700 ]]; then
	pass "all-in-one mints a token into a 0600 file in a 0700 directory"
else
	fail "all-in-one mints a token into a 0600 file in a 0700 directory (rc $rc)"
fi
if grep -q -- "token -agent-id edge-7 -ttl 10m -config" "$dir/calls" && ! grep -rqF "$secret" "$dir/calls" "$dir/out" "$dir/err"; then
	pass "the token subcommand gets the agent id and no secret appears in its arguments, the output, or the errors"
else
	fail "the token subcommand gets the agent id and no secret appears in its arguments, the output, or the errors"
fi

mkdir -p "$work/has-cert/state/pki"
: >"$work/has-cert/state/pki/agent.crt"
run has-cert all-in-one socket
if [[ $rc == 0 && ! -s "$dir/calls" && ! -e "$dir/state/enroll.token" ]]; then
	pass "a certificate on disk means no new token"
else
	fail "a certificate on disk means no new token"
fi

mkdir -p "$work/has-token/state"
echo existing >"$work/has-token/state/enroll.token"
run has-token all-in-one socket
if [[ $rc == 0 && ! -s "$dir/calls" && "$(cat "$dir/state/enroll.token")" == existing ]]; then
	pass "a token file on disk means no new token"
else
	fail "a token file on disk means no new token"
fi

run no-socket all-in-one
if [[ $rc != 0 && ! -e "$dir/state/enroll.token" && ! -e "$dir/state/enroll.token.tmp" ]] && grep -q 'admin socket' "$dir/err"; then
	pass "without the admin socket the helper fails and leaves no token file"
else
	fail "without the admin socket the helper fails and leaves no token file"
fi

run agent agent socket
if [[ $rc == 0 && ! -s "$dir/calls" && ! -e "$dir/state/enroll.token" ]] && grep -q 'enroll.token' "$dir/err"; then
	pass "the agent role never mints a token and says where to put one"
else
	fail "the agent role never mints a token and says where to put one"
fi

mkdir -p "$work/staged"
printf '%s' "$secret" >"$work/staged/staged"
run staged agent
if [[ $rc == 0 && "$(cat "$dir/state/enroll.token")" == "$secret" && ! -e "$dir/staged" && "$(mode "$dir/state/enroll.token")" == 600 ]] &&
	! grep -qF "$secret" "$dir/out"; then
	pass "a token from cloud-init moves to the state directory, the staged file is removed, and the log has no secret"
else
	fail "a token from cloud-init moves to the state directory, the staged file is removed, and the log has no secret"
fi

# --- The arguments of fps-agent-start. It reads fixed paths, so the test copies the
# script and swaps /etc/fps, /run/fps, and /var/lib for a temporary tree.
mkdir -p "$work/root/etc/fps" "$work/root/run/fps" "$work/root/var/lib/fps/pki" "$work/bin"
cat >"$work/bin/fps-agent" <<'EOF'
#!/bin/bash
echo "$*" >"$FPS_TEST_ARGS"
EOF
chmod +x "$work/bin/fps-agent"
sed -e "s#/usr/bin/fps-agent#$work/bin/fps-agent#" -e "s#/etc/fps/#$work/root/etc/fps/#g" -e "s#/run/fps/#$work/root/run/fps/#g" \
	-e "s#/var/lib/fps/pki#$work/root/var/lib/fps/pki#g" "$start" >"$work/agent-start"
chmod +x "$work/agent-start"
cp "$img/files/etc/fps/agent.env" "$work/root/etc/fps/agent.env"
echo 'FPS_MGMT_ADDR=192.0.2.9' >"$work/root/run/fps/mgmt.env"

# start CASE ROLE_ENV_CONTENT: runs the script and leaves its arguments in $args.
start_case() {
	printf '%s' "$2" >"$work/root/run/fps/role.env"
	FPS_TEST_ARGS="$work/args" "$work/agent-start" >/dev/null 2>&1 || true
	args="$(cat "$work/args" 2>/dev/null || true)"
}
echo x >"$work/root/var/lib/fps/pki/ca.crt"
start_case aio $'FPS_ROLE=all-in-one\n'
if [[ "$args" == *"--controller https://192.0.2.9:8444"* && "$args" == *"--cert-dir /var/lib/fps-agent/pki"* &&
	"$args" == *"--enroll-token-file /var/lib/fps-agent/enroll.token"* && "$args" == *"--enroll-address 192.0.2.9:8445"* &&
	"$args" == *"--tls-ca $work/root/var/lib/fps/pki/ca.crt"* ]]; then
	pass "all-in-one: https://, the cert dir, the token file, the enrollment address, and the controller CA"
else
	fail "all-in-one: https://, the cert dir, the token file, the enrollment address, and the controller CA ($args)"
fi

sha="$(printf 'a%.0s' $(seq 1 64))"
start_case pin "$(printf 'FPS_ROLE=agent\nFPS_CONTROLLER_URI=https://203.0.113.5:8444\nFPS_CA_SHA256=%s\nFPS_AGENT_ID=edge-1\n' "$sha")"
if [[ "$args" == *"--controller https://203.0.113.5:8444"* && "$args" == *"--tls-ca-sha256 $sha"* && "$args" != *"--tls-ca "* &&
	"$args" != *"--enroll-address"* && "$args" == *"--agent-id edge-1"* ]]; then
	pass "agent role with a fingerprint: --tls-ca-sha256, the default enrollment address, and no CA file"
else
	fail "agent role with a fingerprint: --tls-ca-sha256, the default enrollment address, and no CA file ($args)"
fi

start_case http $'FPS_ROLE=agent\nFPS_CONTROLLER_URI=http://203.0.113.5:8444\n'
if [[ "$args" == *"--controller http://203.0.113.5:8444"* && "$args" != *"--cert-dir"* && "$args" != *"--tls-ca"* ]]; then
	pass "an http:// controller URI (a plain-mode controller) gets no TLS flags"
else
	fail "an http:// controller URI (a plain-mode controller) gets no TLS flags ($args)"
fi
if ! grep -qF -- "$secret" "$work/args"; then pass "no token reaches the agent command line"; else fail "no token reaches the agent command line"; fi

((fails == 0))
