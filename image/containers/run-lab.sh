#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the container images of IMG-05, runs the compose lab, and collects the logs.
#
#   image/containers/run-lab.sh [--out DIR] [--no-build] [--keep]
#
#   --out DIR    where to write the logs and the results   (default image/containers/out)
#   --no-build   use the images fps-controller:local, fps-agent:local, and fps-lab-tools:local
#   --keep       leave the lab running and keep its volumes after the test
#
# The script needs docker with the compose plugin. It starts the controller and the
# two agents (the steps prep and enroll run first), runs the test of
# image/containers/lab/run-test.sh, and then checks that the admin token appears in no
# `docker inspect` output and in no log. It prints PASS or FAIL for each check and ends
# with `RESULT PASS ...` or `RESULT FAIL ...`. Exit code: 0 pass, 1 fail.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$here/out"
build=1
keep=0
while [[ $# -gt 0 ]]; do
	case "$1" in
	--out)
		out="${2:?--out needs a directory}"
		shift 2
		;;
	--no-build)
		build=0
		shift
		;;
	--keep)
		keep=1
		shift
		;;
	*)
		echo "usage: run-lab.sh [--out DIR] [--no-build] [--keep]" >&2
		exit 2
		;;
	esac
done

compose=(docker compose -f "$here/compose.yaml")
mkdir -p "$out"
out="$(cd "$out" && pwd)"
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

collect() {
	"${compose[@]}" ps -a >"$out/ps.txt" 2>&1 || true
	"${compose[@]}" logs --no-color --timestamps >"$out/compose.log" 2>&1 || true
	for s in prep controller enroll agent1 agent2 lab-test; do
		"${compose[@]}" logs --no-color --timestamps "$s" >"$out/$s.log" 2>&1 || true
	done
	rm -rf "$out/results"
	lab_test="$("${compose[@]}" ps -a -q lab-test 2>/dev/null | head -n 1)"
	if [[ -n "$lab_test" ]]; then
		docker cp "$lab_test:/results" "$out/results" >/dev/null 2>&1 || echo "NOTE no /results to copy"
	fi
}

# dump_logs prints the tail of every service log, for a run that failed before the test.
dump_logs() {
	for s in prep controller enroll agent1 agent2 lab-test; do
		echo "---- log of $s"
		tail -n 40 "$out/$s.log" 2>/dev/null || true
	done
	cat "$out/ps.txt" 2>/dev/null || true
}

# Single quotes on purpose: the trap expands the variables when it runs.
# shellcheck disable=SC2016
trap '[[ "$keep" == 0 ]] && "${compose[@]}" down -v --remove-orphans >/dev/null 2>&1' EXIT

echo "== Start from a clean lab"
"${compose[@]}" down -v --remove-orphans >/dev/null 2>&1 || true

if [[ "$build" == 1 ]]; then
	echo "== Build the images"
	if ! "${compose[@]}" build 2>&1 | tee "$out/build.log"; then
		fail "the images build"
		echo "RESULT FAIL the images did not build"
		exit 1
	fi
	pass "the images fps-controller, fps-agent, and fps-lab-tools build"
fi
docker image ls --format '{{.Repository}}:{{.Tag}} {{.Size}}' | grep -E '^fps-(controller|agent|lab-tools):local' | tee "$out/images.txt" || true

echo "== Start the controller and the two agents"
if ! "${compose[@]}" up -d agent1 agent2 2>&1 | tee "$out/up.log"; then
	collect
	dump_logs
	fail "the controller and the agents start (prep and enroll complete)"
	echo "RESULT FAIL the lab did not start"
	exit 1
fi
pass "the controller and the agents start (prep and enroll complete)"

echo "== Run the test"
"${compose[@]}" up --no-deps --exit-code-from lab-test lab-test 2>&1 | tee "$out/test.log"
test_code=${PIPESTATUS[0]}
collect
[[ "$test_code" == 0 ]] || dump_logs
if [[ "$test_code" == 0 ]]; then
	pass "the test between agent1 and agent2 ends with exit code 0"
else
	fail "the test between agent1 and agent2 ends with exit code $test_code"
fi
# The compose log prefixes every line with the service name and a timestamp. Strip them.
grep -E 'RESULT (PASS|FAIL)' "$out/lab-test.log" | tail -n 1 |
	sed -E 's/^.*(RESULT (PASS|FAIL))/\1/' >"$out/lab-test.result" || true

echo "== Check that no token leaks"
# The admin token is read from its volume by a throwaway container of the tools image.
token_file="$(mktemp "${TMPDIR:-/tmp}/fps-lab-token.XXXXXX")"
chmod 0600 "$token_file"
if docker run --rm --network none --entrypoint cat -v fps-lab_secrets:/secrets:ro fps-lab-tools:local /secrets/admin.token >"$token_file" 2>/dev/null &&
	[[ -s "$token_file" ]]; then
	printf '\n' >>"$token_file"
	ids="$("${compose[@]}" ps -a -q)"
	# shellcheck disable=SC2086
	docker inspect $ids >"$out/inspect.json" 2>&1 || true
	leaks=0
	for f in "$out/inspect.json" "$out/compose.log"; do
		if grep -q -F -f "$token_file" "$f"; then
			leaks=$((leaks + 1))
			echo "NOTE the admin token appears in $f"
		fi
	done
	if [[ "$leaks" == 0 ]]; then
		pass "the admin token appears in no docker inspect output and in no log"
	else
		fail "the admin token appears in $leaks file(s)"
	fi
else
	fail "the admin token could not be read from the volume secrets to check for leaks"
fi
rm -f "$token_file"
# No container carries a token in its command, environment, or insecure plain gRPC.
if grep -q -i -E '"(FPS_[A-Z_]*TOKEN|[A-Z_]*TOKEN)=' "$out/inspect.json" 2>/dev/null; then
	fail "a container has a token variable in its environment"
else
	pass "no container has a token variable in its environment"
fi
if grep -q -E 'insecure[-_]plain[-_]grpc' "$out/inspect.json" 2>/dev/null; then
	fail "a container runs with plain gRPC"
else
	pass "no container runs with plain gRPC"
fi

if [[ "$fails" == 0 ]]; then
	summary="$(cat "$out/lab-test.result" 2>/dev/null)"
	echo "RESULT PASS the compose lab ran a short test between two agent containers: ${summary#RESULT PASS }"
	exit 0
fi
echo "RESULT FAIL $fails check(s) failed"
exit 1
