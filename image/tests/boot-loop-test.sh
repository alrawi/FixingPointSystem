#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Offline test of the boots in a row (FPS_TEST_REPEAT) and of the negative boot
# (FPS_TEST_NEGATIVE=agent-unreachable) of image/boot-test.sh (IMG-07). It needs no root,
# no QEMU, and no image. The test copies the script, stubs the host commands that it
# checks for, and swaps the body of one boot for a fake that prints the lines and the exit
# status that the case wants. The loop, the stop at the first failure, the verdict on
# the negative boot, and the artifact directories are the real code.
set -euo pipefail

img="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/fps-loop-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

mkdir -p "$work/image" "$work/bin"
# The fake replaces everything from the line that sizes the VM, which is the start of one boot.
marker='# The agent profile isolates CPUs and reserves hugepages'
if [[ "$(grep -c "^$marker" "$img/boot-test.sh")" != 1 ]]; then
	echo "FAIL the marker line of one boot is not in boot-test.sh exactly once: $marker"
	exit 1
fi
sed "s|^$marker|if [[ -n \"\${FAKE_BOOT:-}\" ]]; then \"\$FAKE_BOOT\"; exit \$?; fi\n&|" "$img/boot-test.sh" >"$work/image/boot-test.sh"
chmod +x "$work/image/boot-test.sh"
for cmd in qemu-system-x86_64 qemu-img cloud-localds curl ssh scp ssh-keygen; do
	printf '#!/bin/bash\nexit 0\n' >"$work/bin/$cmd"
	chmod +x "$work/bin/$cmd"
done
: >"$work/disk.qcow2"

# The fake boot. FAKE_MODE says what each call does:
#   pass            prints PASS lines and exits 0
#   fail3           the third call fails (exit 1), the others pass
#   neg_ok          a normal boot passes, and the boot with FPS_TEST_AGENT_UNREACHABLE=1 fails
#                   at the agent checks, as the real one does
#   neg_agent_ok    the boot with FPS_TEST_AGENT_UNREACHABLE=1 still passes (the agent reached it)
#   neg_wrong       the negative boot fails, but at ssh, which is the wrong reason
cat >"$work/fake-boot" <<'EOF'
#!/bin/bash
echo "${FPS_TEST_ARTIFACT_DIR}|${FPS_TEST_AGENT_UNREACHABLE:-0}|${FPS_TEST_REPEAT}|${FPS_TEST_NEGATIVE}" >>"$FAKE_LOG"
n="$(wc -l <"$FAKE_LOG")"
unreach="${FPS_TEST_AGENT_UNREACHABLE:-0}"
ok_lines() {
	echo "PASS GET /v1/capabilities on the management interface answers 200 with a JSON object"
	echo "PASS ssh as fps-admin with the seed key works"
}
case "$FAKE_MODE" in
pass)
	ok_lines
	echo "RESULT: PASS (50 checks)"
	;;
fail3)
	ok_lines
	if ((n == 3)); then
		echo "FAIL run: run run_x is completed (state 'failed')"
		echo "RESULT: FAIL (1 of 50 checks failed)"
		exit 1
	fi
	echo "RESULT: PASS (50 checks)"
	;;
neg_ok)
	ok_lines
	if [[ "$unreach" == 1 ]]; then
		echo "FAIL the local agent registered with the controller over mutual TLS (state online)"
		echo "FAIL run: the agent local is online (state 'absent')"
		echo "RESULT: FAIL (2 of 50 checks failed)"
		exit 1
	fi
	echo "RESULT: PASS (50 checks)"
	;;
neg_agent_ok)
	ok_lines
	echo "RESULT: PASS (50 checks)"
	;;
neg_wrong)
	if [[ "$unreach" == 1 ]]; then
		echo "FAIL ssh as fps-admin with the seed key works"
		echo "RESULT: FAIL (1 of 3 checks failed)"
		exit 1
	fi
	ok_lines
	echo "RESULT: PASS (50 checks)"
	;;
esac
EOF
chmod +x "$work/fake-boot"

# run_case NAME MODE [ENV=VALUE...] -- ARGS...: runs the copy and sets $status, $out, and $calls.
run_case() {
	local name="$1" mode="$2"
	shift 2
	local envs=()
	while (($# > 0)) && [[ "$1" != -- ]]; do
		envs+=("$1")
		shift
	done
	shift
	: >"$work/calls.log"
	status=0
	out="$(env PATH="$work/bin:$PATH" FAKE_BOOT="$work/fake-boot" FAKE_MODE="$mode" FAKE_LOG="$work/calls.log" "${envs[@]}" \
		"$work/image/boot-test.sh" "$@" "$work/disk.qcow2" 2>&1)" || status=$?
	calls="$(wc -l <"$work/calls.log")"
	echo "NOTE case: $name (status $status, $calls boots)"
}
has() { grep -qF -- "$1" <<<"$out"; }
lacks() { ! grep -qF -- "$1" <<<"$out"; }
no_line_starts_with() { ! grep -q "^$1" <<<"$out"; }
# The log of the fake has one line per boot: ARTIFACT_DIR|SABOTAGE|REPEAT|NEGATIVE.
distinct_artifact_dirs() { test "$(cut -d'|' -f1 "$work/calls.log" | sort -u | wc -l)" = "$1"; }
boots_without_sabotage() { test "$(grep -c '|0|1|$' "$work/calls.log")" = "$1"; }
boots_with_sabotage() { test "$(grep -c '|1|1|$' "$work/calls.log")" = "$1"; }
check() { # check NAME CONDITION...
	local name="$1"
	shift
	if "$@"; then pass "$name"; else
		fail "$name"
		while IFS= read -r line; do echo "    | $line"; done <<<"$out"
	fi
}

# 1. Ten boots in a row, all pass.
run_case "ten boots pass" pass FPS_TEST_REPEAT=10 FPS_TEST_ARTIFACT_DIR="$work/art" -- --role all-in-one --run
check "ten passing boots end with status 0" test "$status" = 0
check "the loop ran ten boots" test "$calls" = 10
check "the result line counts the boots" has "RESULT: PASS (10 of 10 boots in a row)"
check "each boot gets its own artifact directory" distinct_artifact_dirs 10
check "the last boot writes to boot-10" grep -q "/art/boot-10|" "$work/calls.log"
check "a boot of the loop runs once, with no negative mode and no sabotage" boots_without_sabotage 10

# 2. A failing boot stops the loop and fails the result.
run_case "the third of five boots fails" fail3 FPS_TEST_REPEAT=5 -- --role all-in-one --run
check "a failed boot ends with status 1" test "$status" = 1
check "the loop stops at the first failure" test "$calls" = 3
check "the result line says FAIL" has "RESULT: FAIL"
check "the failure names the boot" has "FAIL boot 3 of 5 did not pass"

# 3. One boot is the old behavior, without a loop or a prefix.
run_case "one boot" pass -- --role all-in-one --run
check "one boot passes through the output of the boot" has "RESULT: PASS (50 checks)"
check "one boot prints no loop headers" lacks "=== boot"

# 4. The negative boot.
run_case "negative boot, the agent is cut off" neg_ok FPS_TEST_REPEAT=2 FPS_TEST_NEGATIVE=agent-unreachable -- --role all-in-one --run
check "the negative boot passes when the boot fails at the agent checks" test "$status" = 0
check "two normal boots and one negative boot ran" test "$calls" = 3
check "only the negative boot has the sabotage" boots_with_sabotage 1
check "the negative verdict is on the result line" has "and the negative boot failed as it must"
check "the expected FAIL lines carry a prefix, so no line starts with FAIL" no_line_starts_with FAIL
check "the prefixed FAIL line is shown" has "negative boot: FAIL the local agent registered"

run_case "negative boot, the agent still reaches the controller" neg_agent_ok FPS_TEST_REPEAT=1 FPS_TEST_NEGATIVE=agent-unreachable -- --role all-in-one --run
check "a negative boot that passes fails the test" test "$status" = 1
check "it says that the boot did not fail at the agent checks" has "FAIL negative boot: the test must fail at the agent checks"

run_case "negative boot, it fails for the wrong reason" neg_wrong FPS_TEST_REPEAT=1 FPS_TEST_NEGATIVE=agent-unreachable -- --role all-in-one --run
check "a negative boot that fails at ssh fails the test" test "$status" = 1

run_case "negative boot after a failed boot" fail3 FPS_TEST_REPEAT=3 FPS_TEST_NEGATIVE=agent-unreachable -- --role all-in-one --run
check "the negative boot does not run after a failed boot" test "$calls" = 3
check "and the result is a failure" test "$status" = 1

# 5. Bad settings end with status 2.
run_case "bad repeat" pass FPS_TEST_REPEAT=0 -- --role all-in-one
check "FPS_TEST_REPEAT=0 is a usage error" test "$status" = 2
run_case "bad negative" pass FPS_TEST_NEGATIVE=nope -- --role all-in-one
check "an unknown negative mode is a usage error" test "$status" = 2
run_case "negative needs all-in-one" pass FPS_TEST_NEGATIVE=agent-unreachable -- --role controller
check "the negative boot needs the all-in-one role" test "$status" = 2

((fails == 0))
