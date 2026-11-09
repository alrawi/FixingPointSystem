#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
#
# Step "lab-test" of the container lab (IMG-05): a short HTTP and DNS test between
# two agent containers. agent1 runs the client side and agent2 the server side, as the
# Gate 1 suite runs its two agents (tests/gates/gate1/run.sh). The script calls the
# controller REST API on the lab network with the admin token of the volume "secrets".
#
# Every check prints one line that starts with PASS or FAIL, and the last line is
# `RESULT PASS ...` or `RESULT FAIL ...`. The exit code is 0 only for a pass. The test
# writes its files (configurations, run summaries, metrics) to /results.
#
# Settings through the environment:
#   FPS_LAB_RATE       sessions per second of the client run    (default 40)
#   FPS_LAB_STEADY_S   seconds of the steady phase              (default 10)
#   FPS_LAB_WAIT_S     seconds to wait for a state              (default 120)
set -u

base=http://controller:8443
rate=${FPS_LAB_RATE:-40}
steady=${FPS_LAB_STEADY_S:-10}
wait_s=${FPS_LAB_WAIT_S:-120}
out=/results
modules=/usr/share/fps/modules
fails=0

pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1${2:+ ($2)}"
	fails=$((fails + 1))
}
finish() {
	if [ "$fails" -eq 0 ]; then
		echo "RESULT PASS $1"
		exit 0
	fi
	echo "RESULT FAIL $fails check(s) failed"
	exit 1
}
# die MESSAGE ends the test at once, after a failed check that later checks depend on.
die() {
	fail "$1" "${2:-}"
	finish
}

# The token reaches curl through a header file that only this process reads, so it is
# never an argument. printf is a shell builtin, so it does not appear in a process list.
umask 077
mkdir -p "$out/work"
{ printf 'Authorization: Bearer '; cat /secrets/admin.token; printf '\n'; } >/tmp/auth.hdr

# call METHOD PATH [CURL ARGS...] prints the body. It returns 1 on a status of 400 or more.
call() {
	method=$1 path=$2
	shift 2
	curl -sS -m 30 --fail-with-body -X "$method" -H @/tmp/auth.hdr "$@" "$base$path"
}

# wait_for DESCRIPTION COMMAND... retries the command once a second until it succeeds.
wait_for() {
	desc=$1
	shift
	i=0
	while ! "$@" >/dev/null 2>&1; do
		i=$((i + 1))
		if [ "$i" -gt "$wait_s" ]; then
			die "$desc" "no success within ${wait_s}s"
		fi
		sleep 1
	done
}

# shellcheck disable=SC2329 # called through wait_for
rest_up() { curl -sS -m 5 -o /dev/null --fail "$base/healthz"; }
# shellcheck disable=SC2329 # called through wait_for
agent_online() { [ "$(call GET /v1/agents | jq -r --arg id "$1" '.[] | select(.id == $id) | .state')" = online ]; }

wait_for "the controller answers on $base" rest_up
pass "the controller answers on $base"

# The controller configuration has no plain gRPC. The agents must use mutual TLS.
if grep -Eq '^[[:space:]]*insecure_plain_grpc:[[:space:]]*true' /etc/fps/controller.yaml; then
	die "the controller runs with mutual TLS" "insecure_plain_grpc is set"
fi
pass "the controller configuration has no insecure_plain_grpc"

wait_for "agent1 is online" agent_online agent1
wait_for "agent2 is online" agent_online agent2
pass "agent1 and agent2 enrolled over HTTPS, registered over mutual TLS, and are online"

# The agents delete the one time token after they enrolled with it.
for id in agent1 agent2; do
	if [ -e "/enroll/$id/enroll.token" ]; then
		fail "the token of $id was used and deleted"
	fi
done
if [ "$fails" -eq 0 ]; then
	pass "the enrollment tokens are used and deleted"
fi

# Install the module trees that the agents load, so that the digests agree.
install_module() {
	name=$1
	# shellcheck disable=SC2035
	(cd "$modules/$name" && tar -czf "$out/work/$name.tar.gz" *) || return 1
	call POST /v1/modules -H 'Content-Type: application/gzip' --data-binary "@$out/work/$name.tar.gz" \
		>"$out/work/module-$name.json" 2>&1
}
for name in http1 dns; do
	if ! install_module "$name"; then
		die "the module $name is installed" "$(head -c 300 "$out/work/module-$name.json" 2>/dev/null)"
	fi
done
pass "the modules http1 and dns are installed"

# store_config ROLE AGENT STEADY prints the id of a stored and valid configuration.
store_config() {
	role=$1 agent=$2 hold=$3
	file="$out/test-$role.yaml"
	sed -e "s/@ROLE@/$role/g" -e "s/@AGENT@/$agent/g" -e "s/@RATE@/$rate/g" -e "s/@STEADY_S@/$hold/g" \
		/usr/share/fps-lab/test-config.yaml >"$file"
	id=$(call POST /v1/configs -H 'Content-Type: application/yaml' --data-binary "@$file" | jq -r .id) || return 1
	report=$(call POST "/v1/configs/$id:validate") || return 1
	[ "$(printf '%s' "$report" | jq -r .valid)" = true ] || {
		echo "$report" >&2
		return 1
	}
	echo "$id"
}

# The server run holds longer than the client run. The test stops it after the client run.
client_config=$(store_config client agent1 "$steady") || die "the client configuration is stored and valid"
server_config=$(store_config server agent2 "$((steady + 120))") || die "the server configuration is stored and valid"
pass "the client configuration $client_config and the server configuration $server_config are stored and valid"

start_run() {
	call POST /v1/runs -H 'Content-Type: application/json' -d "{\"config_id\":\"$1\",\"agents\":[\"$2\"]}" | jq -r .id
}
run_state() { call GET "/v1/runs/$1" | jq -r .state; }

# wait_state RUN WANT... waits until the run reaches one of the wanted states and
# prints it. A final state that is not wanted ends the wait with an error.
wait_state() {
	run=$1
	shift
	i=0
	while [ "$i" -le "$wait_s" ]; do
		got=$(run_state "$run") || got=""
		for want in "$@"; do
			if [ "$got" = "$want" ]; then
				echo "$got"
				return 0
			fi
		done
		case $got in completed | failed | aborted)
			echo "$got"
			return 1
			;;
		esac
		i=$((i + 1))
		sleep 1
	done
	echo "${got:-unknown}"
	return 1
}

server_run=$(start_run "$server_config" agent2) || die "the server run starts on agent2"
state=$(wait_state "$server_run" running) || die "the server run $server_run is running on agent2" "state $state"
pass "the server run $server_run is running on agent2"
# The servers bind their sockets when the run starts. Give them a moment.
sleep 2

client_run=$(start_run "$client_config" agent1) || die "the client run starts on agent1"
state=$(wait_state "$client_run" completed failed) || state=${state:-unknown}
echo "NOTE the client run $client_run ended in the state $state"
call POST "/v1/runs/$server_run:stop" -H 'Content-Type: application/json' -d '{}' >/dev/null || fail "the server run is stopped"
server_state=$(wait_state "$server_run" completed failed) || server_state=${server_state:-unknown}
echo "NOTE the server run $server_run ended in the state $server_state"

call GET "/v1/runs/$client_run" >"$out/client-run.json" || true
call GET "/v1/runs/$server_run" >"$out/server-run.json" || true
call GET "/v1/runs/$client_run/report" >"$out/client-report.json" || true
call GET "/v1/runs/$client_run/metrics?view=application" >"$out/client-metrics.json" || true
call GET "/v1/runs/$server_run/metrics?view=application&side=server" >"$out/server-metrics.json" || true

if [ "$state" = completed ]; then
	pass "the client run $client_run on agent1 is completed"
else
	fail "the client run $client_run on agent1 is completed" "state $state, reason $(jq -r '.reason // "none"' "$out/client-run.json" 2>/dev/null)"
fi
passed=$(jq -r '.verdict.passed' "$out/client-run.json" 2>/dev/null)
if [ "$passed" = true ]; then
	pass "the verdict of the client run is passed (sessions.failed_ratio <= 0.01)"
else
	fail "the verdict of the client run is passed" "verdict.passed is ${passed:-absent}"
fi

# sessions_of FILE APPLICATION prints the succeeded sessions of one application.
sessions_of() {
	jq -r --arg app "$2" '[.samples[]? | select(.key == $app) | .values["sessions.succeeded"] // 0] | add // 0 | floor' "$1" 2>/dev/null
}
summary=""
for app in http1 dns; do
	c=$(sessions_of "$out/client-metrics.json" "$app")
	s=$(sessions_of "$out/server-metrics.json" "$app")
	if [ "${c:-0}" -gt 0 ] 2>/dev/null; then
		pass "agent1 counted $c succeeded $app sessions on the client side"
	else
		fail "agent1 counted succeeded $app sessions on the client side" "got ${c:-none}"
	fi
	if [ "${s:-0}" -gt 0 ] 2>/dev/null; then
		pass "agent2 counted $s succeeded $app sessions on the server side"
	else
		fail "agent2 counted succeeded $app sessions on the server side" "got ${s:-none}"
	fi
	summary="$summary $app client=${c:-0} server=${s:-0}"
done

finish "run $client_run verdict passed;$summary"
