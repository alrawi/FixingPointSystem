#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Gate 1 acceptance suite. The suite checks the exit criterion of Phase 1, an
# HTTP/1.1 and DNS mix that nDPI classifies at 99% through the controller and
# the agent.
#
# Usage:
#   sudo tests/gates/gate1/run.sh FPS_CONTROLLER FPS_AGENT OUT_DIR
#   sudo tests/gates/gate1/run.sh --rescore OUT_DIR
#
# The first form brings up the QA-01 harness and starts the real controller
# in the client namespace. It enrolls two identities of the real agent binary
# over REST, gate1-client in the client namespace with --side client and
# gate1-server in the server namespace with --side server, so every session
# crosses the routing namespace. It installs the http1 and dns modules,
# stores test-config.yaml once per agent, starts the server run, records the
# router's client arm, and starts the client run. When the client run ends it
# stops the server run, fetches the report and the metrics of the client run,
# scores the capture with the realism runner, and writes OUT_DIR/report.md.
#
# The second form scores OUT_DIR/gate1.pcapng again and rewrites the report,
# which shows that a raised threshold fails the gate without a new run.
#
# Exit codes are 0 when the gate passes, 1 when a gate check fails, and 2 when
# the suite cannot run. The environment tunes the run.
#   FPS_GATE1_THRESHOLD   conformance threshold of every application, 0.99
#   FPS_GATE1_RATE        sessions per second of the client run, 100
#   FPS_GATE1_STEADY_S    seconds of the steady phase, 20
#   FPS_GATE1_MIN_SESSIONS  sessions each application needs in the capture, 100
#   FPS_REALISM_PARSERS   reference parsers of the realism runner, tshark,zeek
#   FPS_NETNS             name prefix of the harness, gate1
#   FPS_GATE1_MGMT_NET    management link between the client and server
#                         namespaces, 10.203.0.0/30
#   FPS_GATE1_REPEAT      runs of the suite in a row, 1. With more than one,
#                         each run writes to OUT_DIR/run-N, the loop stops at
#                         the first run that does not pass, and OUT_DIR holds
#                         the files of that run, or of the last run when all
#                         pass, and repeat.json with the exit code of each run
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
readonly here root
harness="$root/tests/harness/netns.sh"
threshold=${FPS_GATE1_THRESHOLD:-0.99}
rate=${FPS_GATE1_RATE:-100}
steady=${FPS_GATE1_STEADY_S:-20}
min_sessions=${FPS_GATE1_MIN_SESSIONS:-100}
parsers=${FPS_REALISM_PARSERS:-tshark,zeek}
repeat=${FPS_GATE1_REPEAT:-1}

usage() {
  echo "usage: run.sh FPS_CONTROLLER FPS_AGENT OUT_DIR | run.sh --rescore OUT_DIR" >&2
  exit 2
}

die() {
  echo "run.sh: $*" >&2
  exit 2
}

log() {
  echo "run.sh: $*" >&2
}

# score_and_report scores the capture of OUT and writes the gate report. It
# exits with the verdict of the gate.
score_and_report() {
  local dir=$1 realism_threshold rc=0
  [[ -s $dir/gate1.pcapng && -s $dir/run-report.json ]] ||
    die "$dir holds no gate1.pcapng and run-report.json"
  # The realism runner accepts thresholds up to 1, and the gate compares
  # every ratio with the full threshold itself, so a threshold above 1 fails.
  realism_threshold=$(python3 -I -c 'import sys; print(min(float(sys.argv[1]), 1.0))' "$threshold") ||
    die "FPS_GATE1_THRESHOLD $threshold is not a number"
  (cd "$root" && python3 -m realism score --pcap "$dir/gate1.pcapng" --labels "$here/expected.yaml" \
    --threshold "$realism_threshold" --parsers "$parsers" --output "$dir/realism.json") || rc=$?
  [[ $rc -ne 2 && -s $dir/realism.json ]] || die "the realism runner could not score $dir/gate1.pcapng"
  # The parser findings stay in OUT as files, so an artifact upload carries
  # them when the parser error check fails. A fault here never decides the gate.
  python3 -I "$here/parser_findings.py" --out-dir "$dir" || log "the parser findings of $dir could not be written"
  rc=0
  python3 -I "$here/gate1_report.py" --out-dir "$dir" --threshold "$threshold" \
    --min-sessions "$min_sessions" || rc=$?
  cat "$dir/report.md"
  return "$rc"
}

if [[ ${1:-} == --rescore ]]; then
  [[ $# -eq 2 ]] || usage
  [[ -d $2 ]] || die "$2 is not a directory"
  out=$(cd "$2" && pwd)
  score_and_report "$out"
  exit 0
fi

[[ $# -eq 3 ]] || usage
[[ $EUID -eq 0 ]] || die "this suite needs root for the namespace harness"
controller=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
agent=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
[[ -x $controller ]] || die "controller binary $1 is not executable"
[[ -x $agent ]] || die "agent binary $2 is not executable"
for cmd in curl tar ip tcpdump ss; do
  command -v "$cmd" >/dev/null 2>&1 || die "missing command $cmd"
done
mkdir -p "$3"
out=$(cd "$3" && pwd)
readonly controller agent out
[[ $repeat =~ ^[1-9][0-9]*$ ]] || die "FPS_GATE1_REPEAT $repeat is not a positive integer"

# repeat_runs runs the suite FPS_GATE1_REPEAT times in a row. Every run starts
# a new process of this script, so each run brings up its own harness,
# controller, and agents. The loop stops at the first run that does not pass
# and exits with its code. OUT then holds the files of that run, which the
# artifact upload of the workflow picks up, and otherwise the files of the
# last run, which prove-threshold.sh scores again. The captures of earlier
# passing runs are deleted to save disk space.
repeat_runs() {
  local i rc=0 last="" codes=""
  for ((i = 1; i <= repeat; i++)); do
    rc=0
    log "run $i of $repeat"
    FPS_GATE1_REPEAT=1 "${BASH_SOURCE[0]}" "$controller" "$agent" "$out/run-$i" || rc=$?
    codes+="${codes:+, }{\"run\": $i, \"exit\": $rc}"
    [[ -z $last ]] || rm -f "$out/run-$last/gate1.pcapng"
    last=$i
    [[ $rc -eq 0 ]] || break
  done
  find "$out/run-$last" -maxdepth 1 -type f -exec cp -t "$out" {} +
  printf '{"requested": %s, "completed": %s, "passed": %s, "runs": [%s]}\n' \
    "$repeat" "$last" "$([[ $rc -eq 0 ]] && echo true || echo false)" "$codes" >"$out/repeat.json"
  log "$last of $repeat runs completed, the last exited with $rc"
  return "$rc"
}

if [[ $repeat -gt 1 ]]; then
  repeat_runs || exit $?
  exit 0
fi

export FPS_NETNS=${FPS_NETNS:-gate1}
export FPS_STATE_DIR=${FPS_STATE_DIR:-$out/state}
mgmt_net=${FPS_GATE1_MGMT_NET:-10.203.0.0/30}
work=$out/work
rm -rf "$work" "$out/state"
mkdir -p "$work/modules"
rest=127.0.0.1:7080
grpc_port=7443
admin=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
runner=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
client_run="" server_run=""

fail() {
  echo "run.sh: $*" >&2
  local f
  for f in controller client-agent server-agent; do
    echo "--- last lines of $f.log" >&2
    tail -n 30 "$out/$f.log" >&2 || true
  done
  exit 2
}

cleanup() {
  "$harness" down >/dev/null 2>&1 || true
}
trap cleanup EXIT

"$harness" up
info() {
  "$harness" info | sed -n "s/^$1=//p"
}
client_addr=$(info CLIENT_ADDR)
server_addr=$(info SERVER_ADDR)
ns_client="$FPS_NETNS-client"
ns_server="$FPS_NETNS-server"

# The controller and the two agents talk over a veth link between the client
# and server namespaces, so the control traffic never crosses the captured
# arm. Deleting the namespaces removes the link.
read -r mgmt_client mgmt_server mgmt_len < <(python3 -I -c 'import ipaddress, sys
n = ipaddress.ip_network(sys.argv[1])
h = list(n.hosts())
print(h[0], h[1], n.prefixlen)' "$mgmt_net") || die "FPS_GATE1_MGMT_NET $mgmt_net is not a usable prefix"
ip -n "$ns_client" link add mgmt0 type veth peer name mgmt0 netns "$ns_server"
ip -n "$ns_client" addr add "$mgmt_client/$mgmt_len" dev mgmt0
ip -n "$ns_server" addr add "$mgmt_server/$mgmt_len" dev mgmt0
ip -n "$ns_client" link set mgmt0 up
ip -n "$ns_server" link set mgmt0 up

# The pools of the configuration become local routes on the loopback of
# their namespace (AnyIP), and the router routes each pool to its arm.
pool() {
  python3 -I -c 'import re, sys
for line in open(sys.argv[1]):
    m = re.search(r"name: " + sys.argv[2] + r",.*ipv4: ([0-9./]+)", line)
    if m:
        print(m.group(1))
        break' "$here/test-config.yaml" "$1"
}
client_pool=$(pool clients)
server_pool=$(pool servers)
[[ -n $client_pool && -n $server_pool ]] || die "test-config.yaml names no client and server pools"
"$harness" exec router ip route add "$client_pool" via "$client_addr"
"$harness" exec router ip route add "$server_pool" via "$server_addr"
"$harness" exec client ip route add local "$client_pool" dev lo
"$harness" exec server ip route add local "$server_pool" dev lo

cat >"$work/controller.yaml" <<EOF
listen: $rest
grpc_listen: 0.0.0.0:$grpc_port
# The agent has no TLS until AGT-08, so the gate uses the explicit plain mode.
insecure_plain_grpc: true
log_level: debug
data_dir: $work/data
tokens:
  - { name: admin, scope: admin, token: $admin }
  - { name: gate1, scope: run, token: $runner }
EOF
"$harness" exec client "$controller" -config "$work/controller.yaml" 2>"$out/controller.log" &
# harness down kills every process in the namespaces, so the shell need not
# track the background jobs.
disown

# call METHOD PATH TOKEN [CURL ARGS...] prints the body and fails on a status
# of 300 or more. The REST listener lives on the loopback of the client
# namespace.
call() {
  local method=$1 path=$2 token=$3
  shift 3
  "$harness" exec client curl -sS --fail-with-body -X "$method" -H "Authorization: Bearer $token" \
    "$@" "http://$rest$path" || fail "$method $path failed"
}

json_field() {
  python3 -I -c 'import json, sys
v = json.load(sys.stdin)
for k in sys.argv[1].split("."):
    v = v[int(k)] if isinstance(v, list) else v.get(k)
print("" if v is None else v)' "$1"
}

up=""
for _ in $(seq 150); do
  if "$harness" exec client curl -s -o /dev/null "http://$rest/v1/agents" 2>/dev/null; then
    up=1
    break
  fi
  sleep 0.1
done
[[ -n $up ]] || fail "the controller did not listen on $rest"

# The modules are copies of the tracked trees of modules/http1 and
# modules/dns, so build output in the checkout cannot change their digests.
for name in http1 dns; do
  if git -c safe.directory="$root" -C "$root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    (cd "$root" && git -c safe.directory="$root" ls-files -z "modules/$name" | tar --null -T - -cf -) |
      tar -xf - -C "$work"
  else
    cp -R "$root/modules/$name" "$work/modules/"
  fi
  [[ -f $work/modules/$name/manifest.yaml ]] || die "modules/$name has no manifest.yaml"
  (cd "$work/modules/$name" && tar -czf "$work/$name.tar.gz" -- *)
done

agent_state() {
  call GET /v1/agents "$runner" | python3 -I -c 'import json, sys
for a in json.load(sys.stdin):
    if a.get("id") == sys.argv[1]:
        print(a.get("state", ""))' "$1"
}

start_agent() {
  local id=$1 side=$2 ns=$3 dial=$4 state=""
  call POST /v1/agents "$admin" -H 'Content-Type: application/json' \
    -d "{\"id\":\"$id\",\"labels\":{\"gate\":\"1\",\"side\":\"$side\"}}" >/dev/null
  "$harness" exec "$ns" "$agent" connect --controller "http://$dial:$grpc_port" --agent-id "$id" \
    --modules "$work/modules" --side "$side" --label "side=$side" 2>"$out/$side-agent.log" &
  disown
  for _ in $(seq 150); do
    state=$(agent_state "$id")
    [[ $state == online ]] && return 0
    sleep 0.1
  done
  fail "agent $id is ${state:-absent}, want online"
}

start_agent gate1-client client client 127.0.0.1
start_agent gate1-server server server "$mgmt_client"

for name in http1 dns; do
  call POST /v1/modules "$admin" -H 'Content-Type: application/gzip' \
    --data-binary "@$work/$name.tar.gz" >/dev/null
done

# store_config ROLE AGENT STEADY prints the id of a stored and valid configuration.
store_config() {
  local role=$1 id=$2 hold=$3 file="$work/$1.yaml" config report
  sed -e "s/@ROLE@/$role/g" -e "s/@AGENT@/$id/g" -e "s/@RATE@/$rate/g" \
    -e "s/@STEADY_S@/$hold/g" "$here/test-config.yaml" >"$file"
  config=$(call POST /v1/configs "$runner" -H 'Content-Type: application/yaml' \
    --data-binary "@$file" | json_field id)
  report=$(call POST "/v1/configs/$config:validate" "$runner")
  [[ $(json_field valid <<<"$report") == True ]] || fail "the $role configuration is not valid: $report"
  echo "$config"
}

# The server run holds longer than the client run, and the suite stops it once
# the client run ends.
client_config=$(store_config client gate1-client "$steady")
server_config=$(store_config server gate1-server "$((steady + 120))")

start_run() {
  call POST /v1/runs "$runner" -H 'Content-Type: application/json' \
    -d "{\"config_id\":\"$1\",\"agents\":[\"$2\"]}" | json_field id
}

# wait_run RUN WANT... waits until the run reaches one of the wanted states
# and prints it.
wait_run() {
  local run=$1 got=""
  shift
  for _ in $(seq $((10 * (steady + 180)))); do
    got=$(call GET "/v1/runs/$run" "$runner" | json_field state)
    for want in "$@"; do
      if [[ $got == "$want" ]]; then
        echo "$got"
        return 0
      fi
    done
    case $got in completed | failed | aborted) fail "run $run is $got, want $*" ;; esac
    sleep 0.1
  done
  fail "run $run is $got, want $*"
}

server_run=$(start_run "$server_config" gate1-server)
wait_run "$server_run" running >/dev/null
# The server side is ready when it listens on every port of the mix.
first_server=$(python3 -I -c 'import ipaddress, sys; print(next(ipaddress.ip_network(sys.argv[1]).hosts()))' "$server_pool")
ready=""
for _ in $(seq 100); do
  if "$harness" exec server ss -Hltn src "$first_server" | grep -q ":8080 " &&
    "$harness" exec server ss -Hltn src "$first_server" | grep -q ":80 " &&
    "$harness" exec server ss -Hlun src "$first_server" | grep -q ":53 "; then
    ready=1
    break
  fi
  sleep 0.1
done
[[ -n $ready ]] || fail "the server agent does not listen on $first_server ports 80, 8080, and 53"

"$harness" capture start "$out/gate1.pcapng" client
client_run=$(start_run "$client_config" gate1-client)
log "client run $client_run and server run $server_run started"
client_state=$(wait_run "$client_run" completed failed)
call POST "/v1/runs/$server_run:stop" "$runner" -H 'Content-Type: application/json' -d '{}' >/dev/null
server_state=$(wait_run "$server_run" completed failed)
"$harness" capture stop
# tcpdump counts the packets that the kernel dropped when it stops. The log
# stays in OUT, and the count goes to context.json, so a capture that lost
# packets shows in the report and in the artifact.
cp "$FPS_STATE_DIR/$FPS_NETNS.capture.log" "$out/tcpdump.log" 2>/dev/null || : >"$out/tcpdump.log"
dropped=$(sed -n 's/^\([0-9][0-9]*\) packets dropped by kernel.*/\1/p' "$out/tcpdump.log" | tail -n 1)
log "client run $client_run is $client_state and server run $server_run is $server_state"

call GET "/v1/runs/$client_run/report" "$runner" >"$out/run-report.json"
call GET "/v1/runs/$client_run/report?format=csv" "$runner" >"$out/run-report.csv"
call GET "/v1/runs/$client_run/metrics?view=application" "$runner" >"$out/run-metrics.json"
call GET "/v1/runs/$server_run/report" "$runner" >"$out/server-run-report.json"
cp "$work/client.yaml" "$out/test-config.yaml"
python3 -I -c 'import json, sys
keys = ("client_run", "server_run", "server_run_state", "rate", "steady_s", "parsers", "capture_dropped", "commit")
json.dump(dict(zip(keys, sys.argv[1:])), open(sys.argv[-1] + "/context.json", "w"), indent=2)' \
  "$client_run" "$server_run" "$server_state" "$rate" "$steady" "$parsers" "${dropped:-unknown}" \
  "$(git -c safe.directory="$root" -C "$root" rev-parse --short HEAD 2>/dev/null || echo unknown)" "$out"

# Under set -e a failed gate ends the script with the code of the report.
score_and_report "$out"
