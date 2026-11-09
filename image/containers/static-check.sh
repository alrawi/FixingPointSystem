#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Static checks of the container sources of IMG-05. They need no Docker daemon, no
# network, and no root. Hadolint, ShellCheck, and `docker compose config` run when
# installed. Set FPS_REQUIRE_CONTAINER_TOOLS=1 to fail when one is missing.
#
#   image/containers/static-check.sh
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here" || exit 2
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}
missing() {
	if [[ "${FPS_REQUIRE_CONTAINER_TOOLS:-0}" == 1 ]]; then
		fail "$1 is not installed"
	else
		echo "SKIP $1 is not installed"
	fi
}

dockerfiles=(Dockerfile.controller Dockerfile.agent)

# 1. Every base image is pinned by digest, except scratch and earlier stages.
for f in "${dockerfiles[@]}"; do
	stages=" "
	while read -r from image rest; do
		[[ "$from" == FROM ]] || continue
		name=""
		[[ "$rest" == AS\ * ]] && name="${rest#AS }"
		if [[ "$image" == scratch ]]; then
			:
		elif [[ "$stages" == *" $image "* ]]; then
			:
		elif [[ "$image" =~ ^[a-z0-9./_-]+:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}$ ]] && [[ "$image" != *:latest@* ]]; then
			:
		else
			fail "$f: the base image $image is not pinned by tag and digest"
			continue
		fi
		[[ -n "$name" ]] && stages="$stages$name "
	done <"$f"
	pass "$f: the base images are pinned by digest"
done

# 2. The final stages run as a numeric non root user.
for f in "${dockerfiles[@]}"; do
	for target in controller agent tools; do
		grep -q "^FROM .* AS $target\$" "$f" || continue
		user="$(awk -v t="$target" '/^FROM / { in_t = ($0 ~ (" AS " t "$")) } in_t && /^USER / { u = $2 } END { print u }' "$f")"
		if [[ "$user" =~ ^[1-9][0-9]*(:[1-9][0-9]*)?$ ]]; then
			pass "$f: the target $target runs as the non root user $user"
		else
			fail "$f: the target $target has no numeric non root USER (found '${user:-none}')"
		fi
	done
done

# 3. No secret in the sources: no plain gRPC, no token value in the compose file.
if grep -v -E '^[[:space:]]*#' controller.yaml compose.yaml | grep -q -E 'insecure[-_]plain[-_]grpc'; then
	fail "a container source turns on plain gRPC"
else
	pass "no container source turns on plain gRPC"
fi
if grep -n -i -E '^\s*(environment|env_file):|(_TOKEN|_SECRET|PASSWORD)=' compose.yaml | grep -q .; then
	fail "compose.yaml has an environment section or a token variable"
else
	pass "compose.yaml has no environment section and no token variable"
fi
if grep -n -E 'privileged:|network_mode: *host|pid: *host|cap_add:' compose.yaml | grep -q .; then
	fail "compose.yaml grants privileges (privileged, host network, host pid, or cap_add)"
else
	pass "compose.yaml grants no privilege (no privileged, host network, host pid, or cap_add)"
fi

# 4. Every service drops all capabilities, which the compose file merges in.
if command -v docker >/dev/null && docker compose version >/dev/null 2>&1; then
	rendered="$(docker compose -f compose.yaml config 2>&1)" || fail "compose.yaml does not render: $rendered"
	services="$(docker compose -f compose.yaml config --services)"
	count="$(printf '%s\n' "$services" | grep -c .)"
	dropped="$(printf '%s\n' "$rendered" | grep -c -E '^      - ALL$')"
	ro="$(printf '%s\n' "$rendered" | grep -c -E '^    read_only: true$')"
	if [[ "$count" -gt 0 && "$dropped" == "$count" ]]; then
		pass "all $count services drop every capability (cap_drop: ALL)"
	else
		fail "$dropped of $count services drop every capability"
	fi
	if [[ "$ro" == "$count" ]]; then
		pass "all $count services have a read only root file system"
	else
		fail "$ro of $count services have a read only root file system"
	fi
else
	missing "docker compose"
fi

# 5. Hadolint and ShellCheck.
if command -v hadolint >/dev/null; then
	if hadolint "${dockerfiles[@]}"; then
		pass "hadolint accepts the Dockerfiles"
	else
		fail "hadolint reports findings"
	fi
else
	missing hadolint
fi
if command -v shellcheck >/dev/null; then
	if shellcheck -x run-lab.sh static-check.sh && shellcheck -s sh lab/*.sh; then
		pass "ShellCheck accepts the scripts"
	else
		fail "ShellCheck reports findings"
	fi
else
	missing shellcheck
fi
for s in run-lab.sh static-check.sh; do bash -n "$s" || fail "bash -n $s"; done
for s in lab/*.sh; do sh -n "$s" || fail "sh -n $s"; done

if [[ "$fails" == 0 ]]; then
	echo "RESULT PASS the container sources pass the static checks"
	exit 0
fi
echo "RESULT FAIL $fails check(s) failed"
exit 1
