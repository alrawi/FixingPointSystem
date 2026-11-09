#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# A short HTTP and DNS run inside the booted all-in-one appliance (IMG-04).
# image/boot-test.sh --run copies this script to the VM and runs it there as root.
#
#   vm-run.sh [OUT_DIR]      OUT_DIR defaults to /tmp/fps-run
#
# It is a reduced variant of the Gate 1 suite (tests/gates/gate1), which needs the
# namespace harness, two agents, and the realism tools that the image does not
# have. This run uses what the image has: the controller REST API on the management
# address with the admin token, and the agent `local`, which enrolled over mutual
# TLS. It installs the http1 and dns module trees that the agent loads, stores a
# test configuration with the Gate 1 mix entries (http1/single_fetch and
# dns/lookup_a) on loopback pools, runs it to its end, and checks the verdict and
# the succeeded sessions of both applications. Every check prints one line that
# starts with PASS or FAIL, and the script exits with 1 when any fails.
#
# Settings through the environment:
#   FPS_RUN_RATE       sessions per second            (default 50)
#   FPS_RUN_STEADY_S   seconds of the steady phase    (default 10)
#   FPS_RUN_WAIT_S     seconds to wait for the run to end after the load phases (default 120)
set -uo pipefail

out="${1:-/tmp/fps-run}"
rate="${FPS_RUN_RATE:-50}"
steady="${FPS_RUN_STEADY_S:-10}"
wait_s="${FPS_RUN_WAIT_S:-120}"
modules=/usr/share/fps/modules

fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1${2:+ ($2)}"
	fails=$((fails + 1))
}
finish() {
	if ((fails == 0)); then exit 0; fi
	echo "NOTE the last 20 lines of the agent journal"
	journalctl -u fps-agent --no-pager -n 20 2>/dev/null || true
	echo "NOTE the last 20 lines of the controller journal"
	journalctl -u fps-controller --no-pager -n 20 2>/dev/null || true
	exit 1
}

mkdir -p "$out"
# shellcheck source=/dev/null
. /run/fps/mgmt.env
base="http://${FPS_MGMT_ADDR}:8443"
token="$(cat /etc/fps/admin.token)"

# call METHOD PATH [CURL ARGS...] prints the body. It returns 1 on an HTTP status of 400 or more.
call() {
	local method="$1" path="$2"
	shift 2
	curl -sS -m 30 --fail-with-body -X "$method" -H "Authorization: Bearer $token" "$@" "$base$path"
}
json_field() { # json_field PATH: prints a field of the JSON on standard input
	python3 -I -c 'import json, sys
v = json.load(sys.stdin)
for k in sys.argv[1].split("."):
    v = v[int(k)] if isinstance(v, list) else v.get(k)
print("" if v is None else v)' "$1"
}

# The local agent is online (it enrolled over mutual TLS before the checks ran).
state=""
for _ in $(seq 60); do
	state="$(call GET /v1/agents 2>/dev/null | python3 -I -c 'import json, sys
for a in json.load(sys.stdin):
    if a.get("id") == "local":
        print(a.get("state", ""))')"
	[[ "$state" == online ]] && break
	sleep 1
done
if [[ "$state" == online ]]; then
	pass "run: the agent local is online"
else
	fail "run: the agent local is online" "state '${state:-absent}'"
	finish
fi

# Install the module trees that the agent loads, so the controller and the agent
# see the same digests.
modules_ok=yes
for name in http1 dns; do
	if [[ -f "$modules/$name/manifest.yaml" ]] &&
		(cd "$modules/$name" && tar -czf "$out/$name.tar.gz" -- *) &&
		call POST /v1/modules -H 'Content-Type: application/gzip' --data-binary "@$out/$name.tar.gz" >"$out/module-$name.json" 2>&1; then
		:
	else
		modules_ok=no
		echo "NOTE installing module $name failed: $(head -c 300 "$out/module-$name.json" 2>/dev/null)"
	fi
done
if [[ "$modules_ok" == yes ]]; then
	pass "run: the modules http1 and dns are installed through POST /v1/modules"
else
	fail "run: the modules http1 and dns are installed through POST /v1/modules"
	finish
fi

# The controller checks port locations against the agent capabilities, which leave
# out the loopback, so the configuration names the first detected interface. The
# loopback pools carry the traffic, and the agent binds no interface.
iface="$(/usr/bin/fps-agent capabilities | awk '$1 == "port" { print $2; exit }')"
if [[ -z "$iface" ]]; then
	fail "run: the agent detects a traffic interface"
	finish
fi

cat >"$out/test-config.yaml" <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: image_boot_run
  description: A short HTTP and DNS run inside the appliance (IMG-04).
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.91.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.92.0.0/24, count: 4 }
profile:
  basis: sessions
  mix:
    - { scenario: http1/single_fetch, weight: 0.6, params: { server_port: 18080, body_size: 4096 } }
    - { scenario: dns/lookup_a, weight: 0.4, params: { server_port: 15353 } }
load:
  objective: { type: sessions_per_second, value: $rate }
  phases:
    - { name: ramp_up, duration_s: 2, shape: linear }
    - { name: steady, duration_s: $steady, shape: hold }
timeline: { seed: 7 }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.01 }
EOF

config="$(call POST /v1/configs -H 'Content-Type: application/yaml' --data-binary "@$out/test-config.yaml" | json_field id)"
report="$(call POST "/v1/configs/$config:validate" 2>&1)"
if [[ -n "$config" && "$(json_field valid <<<"$report")" == True ]]; then
	pass "run: the test configuration is stored and valid"
else
	fail "run: the test configuration is stored and valid" "$(head -c 400 <<<"$report")"
	finish
fi

run="$(call POST /v1/runs -H 'Content-Type: application/json' -d "{\"config_id\":\"$config\",\"agents\":[\"local\"]}" | json_field id)"
if [[ -z "$run" ]]; then
	fail "run: the run starts on the agent local"
	finish
fi
echo "NOTE run $run, $rate sessions per second, $steady seconds steady"

# Wait for the run to end. The load phases take steady + 2 seconds.
got=""
deadline=$((SECONDS + steady + 2 + wait_s))
while ((SECONDS < deadline)); do
	got="$(call GET "/v1/runs/$run" 2>/dev/null | json_field state)"
	case "$got" in completed | failed | aborted) break ;; esac
	sleep 1
done
call GET "/v1/runs/$run" >"$out/run-summary.json" 2>&1 || true
call GET "/v1/runs/$run/report" >"$out/run-report.json" 2>&1 || true
call GET "/v1/runs/$run/report?format=csv" >"$out/run-report.csv" 2>&1 || true
call GET "/v1/runs/$run/metrics?view=application" >"$out/run-metrics.json" 2>&1 || true
if [[ "$got" == completed ]]; then
	pass "run: run $run is completed"
else
	fail "run: run $run is completed" "state '${got:-unknown}'"
fi
if [[ "$(json_field verdict.passed <"$out/run-summary.json" 2>/dev/null)" == True ]]; then
	pass "run: the verdict of run $run is passed (sessions.failed_ratio <= 0.01)"
else
	fail "run: the verdict of run $run is passed" "$(head -c 400 "$out/run-summary.json")"
fi

succeeded_of() { # succeeded_of APPLICATION: succeeded sessions in the application view
	python3 -I -c 'import json, sys
s = json.load(open(sys.argv[2]))
print(int(sum(x["values"].get("sessions.succeeded", 0) for x in s.get("samples", []) if x.get("key") == sys.argv[1])))' \
		"$1" "$out/run-metrics.json" 2>/dev/null || echo 0
}
for app in http1 dns; do
	n="$(succeeded_of "$app")"
	if [[ "$n" -gt 0 ]]; then
		pass "run: $n succeeded $app sessions"
	else
		fail "run: the run counted succeeded $app sessions" "$n"
	fi
done
echo "NOTE the agent local is still $(call GET /v1/agents | json_field 0.state)"
finish
