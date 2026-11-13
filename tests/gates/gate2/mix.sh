#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Mix stage of the Gate 2 suite. The stage runs a byte weighted enterprise mix of
# the 15 applications of decision D7 through the real controller and the real
# agent, captures it, and checks that the achieved byte shares stay within the
# tolerance of the configured weights, by the run metrics and by nDPI.
#
# Usage:
#   sudo tests/gates/gate2/mix.sh FPS_CONTROLLER FPS_AGENT OUT_DIR
#   sudo tests/gates/gate2/mix.sh --rescore OUT_DIR
#
# The first form brings up the QA-01 harness and starts the controller in the
# client namespace, like the Gate 1 suite. It enrolls two identities of the agent
# binary, gate2-client with --side client and gate2-server with --side server,
# installs the module trees that mix_config.py stages from the tracked files, and
# stores the test configuration once per agent. The server run holds longer than
# the client run, and the stage stops it when the client run ends. It records the
# router's client arm, fetches the report of the client run, scores the capture
# with the realism runner (conformance, parser errors, and the mix accuracy check
# with the run report), and writes OUT_DIR/mix/.
#
# The second form scores OUT_DIR/mix/mix.pcapng again, which shows that a tighter
# tolerance fails the stage without a new run.
#
# Exit codes are 0 when the stage passes, 1 when a check fails, and 2 when the
# stage cannot run. The environment tunes it.
#   FPS_GATE2_RATE_BPS     throughput objective in bits per second, 8000000
#   FPS_GATE2_RAMP_S       seconds of the ramp up, 10
#   FPS_GATE2_STEADY_S     seconds of the steady phase, 120
#   FPS_GATE2_RAMP_DOWN_S  seconds of the ramp down, 10
#   FPS_GATE2_SEED         seed of the test configuration, 7
#   FPS_GATE2_TOLERANCE    largest difference in points between a measured and a
#                          configured share, 2
#   FPS_GATE2_MIX_WAIVE    modules that the mix leaves out because the agent does
#                          not run them yet. A waiver ends by itself once
#                          engine/fps-agent depends on the crate of the module
#   FPS_REALISM_PARSERS    reference parsers of the realism runner, tshark,zeek
#   FPS_NETNS              name prefix of the harness, gate2
#   FPS_GATE2_MGMT_NET     management link between the client and server
#                          namespaces, 10.203.4.0/30
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
readonly here root
harness="$root/tests/harness/netns.sh"
rate_bps=${FPS_GATE2_RATE_BPS:-8000000}
ramp=${FPS_GATE2_RAMP_S:-10}
steady=${FPS_GATE2_STEADY_S:-120}
ramp_down=${FPS_GATE2_RAMP_DOWN_S:-10}
seed=${FPS_GATE2_SEED:-7}
tolerance=${FPS_GATE2_TOLERANCE:-2}
parsers=${FPS_REALISM_PARSERS:-tshark,zeek}
requested_waiver=${FPS_GATE2_MIX_WAIVE:-}
basis=${FPS_GATE2_BASIS:-bytes}
# Either waiver lets the verdict be PASS WITH WAIVERS: a module left out of the
# mix, or a controller basis other than bytes.
allow_waivers=
[[ -z $requested_waiver && $basis == bytes ]] || allow_waivers=--allow-waivers
mix_config="$here/mix_config.py"

# The realism runner starts Zeek through zeek-gate2.sh, which switches off the event queue
# limit of Zeek for the large capture (QA-13). The real Zeek is FPS_ZEEK, the search path,
# or /opt/zeek/bin/zeek. Without any of them FPS_ZEEK stays as it is, so the runner reports
# the missing parser as before.
zeek_real=${FPS_ZEEK:-$(command -v zeek || true)}
if [[ -z $zeek_real && -x /opt/zeek/bin/zeek ]]; then
  zeek_real=/opt/zeek/bin/zeek
fi
if [[ -n $zeek_real ]]; then
  export FPS_GATE2_ZEEK=$zeek_real
  export FPS_ZEEK="$here/zeek-gate2.sh"
fi

usage() {
  echo "usage: mix.sh FPS_CONTROLLER FPS_AGENT OUT_DIR | mix.sh --rescore OUT_DIR" >&2
  exit 2
}

die() {
  echo "mix.sh: $*" >&2
  exit 2
}

log() {
  echo "mix.sh: $*" >&2
}

# The waivers that still hold, which is the modules that the agent has no crate for.
waived=$(python3 -I "$mix_config" waived --requested "$requested_waiver" --cargo "$root/engine/fps-agent/Cargo.toml") ||
  die "cannot decide the waivers"
mix_modules=$(python3 -I "$mix_config" names --waived "$waived") || die "cannot list the modules of the mix"
readonly waived mix_modules

# log_capture FILE prints the packet count, the duration, and the first and last packet
# time of the capture, so that a parser finding (the report gives its time in UTC) can
# be placed at the start, in the middle, or at the end of the capture. capinfos prints
# local time, which is UTC on the runners. It is best effort and never fails the stage.
log_capture() {
  command -v capinfos >/dev/null 2>&1 || return 0
  local line
  capinfos -M -c -u -a -e "$1" 2>&1 | grep -v '^File name' | while IFS= read -r line; do
    log "capture: $line"
  done || true
}

# score_and_report DIR scores the capture of DIR/mix.pcapng and writes the report of the
# stage. The report names every parser finding in its own section, so the job log shows
# what tshark and Zeek reported. It returns the verdict of the stage.
score_and_report() {
  local dir=$1 rc=0
  [[ -s $dir/mix.pcapng && -s $dir/run-report.json && -s $dir/test-config.yaml && -s $dir/score-config.yaml && -s $dir/expected.yaml ]] ||
    die "$dir holds no mix.pcapng, run-report.json, test-config.yaml, score-config.yaml, and expected.yaml"
  log_capture "$dir/mix.pcapng"
  (cd "$root" && python3 -m realism score --pcap "$dir/mix.pcapng" --labels "$dir/expected.yaml" \
    --test-config "$dir/score-config.yaml" --run-report "$dir/run-report.json" \
    --mix-tolerance "$tolerance" --mix-basis bytes \
    --parsers "$parsers" --output "$dir/realism.json") || rc=$?
  [[ $rc -ne 2 && -s $dir/realism.json ]] || die "the realism runner could not score $dir/mix.pcapng"
  # The job log names every parser finding with its place in the capture, and the findings
  # stay in OUT as parser-findings.json. A fault here never decides the stage.
  python3 -I "$here/mix_findings.py" --out-dir "$(dirname "$dir")" ||
    log "the parser findings of $dir could not be written"
  rc=0
  python3 -I "$here/gate2_report.py" --out-dir "$(dirname "$dir")" --stages mix \
    --tolerance "$tolerance" --mix-modules "${mix_modules// /,}" ${allow_waivers:+"$allow_waivers"} || rc=$?
  return "$rc"
}

if [[ ${1:-} == --rescore ]]; then
  [[ $# -eq 2 ]] || usage
  [[ -d $2/mix ]] || die "$2/mix is not a directory"
  out=$(cd "$2/mix" && pwd)
  score_and_report "$out"
  exit $?
fi

[[ $# -eq 3 ]] || usage
[[ $EUID -eq 0 ]] || die "this stage needs root for the namespace harness"
controller=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
agent=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
[[ -x $controller ]] || die "controller binary $1 is not executable"
[[ -x $agent ]] || die "agent binary $2 is not executable"
for cmd in curl tar ip tcpdump ss; do
  command -v "$cmd" >/dev/null 2>&1 || die "missing command $cmd"
done
mkdir -p "$3/mix"
out=$(cd "$3/mix" && pwd)
readonly controller agent out

export FPS_NETNS=${FPS_NETNS:-gate2}
export FPS_STATE_DIR=${FPS_STATE_DIR:-$out/state}
mgmt_net=${FPS_GATE2_MGMT_NET:-10.203.4.0/30}
work=$out/work
rm -rf "$work" "$out/state"
mkdir -p "$work"
rest=127.0.0.1:7080
grpc_port=7443
admin=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
runner=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
client_run="" server_run=""

fail() {
  echo "mix.sh: $*" >&2
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

[[ -z $waived ]] || log "the mix leaves out $waived, which the agent does not run yet"
log "the mix covers: $mix_modules"

"$harness" up
info() {
  "$harness" info | sed -n "s/^$1=//p"
}
client_addr=$(info CLIENT_ADDR)
server_addr=$(info SERVER_ADDR)
ns_client="$FPS_NETNS-client"
ns_server="$FPS_NETNS-server"

# The controller and the two agents talk over a veth link between the client and
# server namespaces, so the control traffic never crosses the captured arm.
read -r mgmt_client mgmt_server mgmt_len < <(python3 -I -c 'import ipaddress, sys
n = ipaddress.ip_network(sys.argv[1])
h = list(n.hosts())
print(h[0], h[1], n.prefixlen)' "$mgmt_net") || die "FPS_GATE2_MGMT_NET $mgmt_net is not a usable prefix"
ip -n "$ns_client" link add mgmt0 type veth peer name mgmt0 netns "$ns_server"
ip -n "$ns_client" addr add "$mgmt_client/$mgmt_len" dev mgmt0
ip -n "$ns_server" addr add "$mgmt_server/$mgmt_len" dev mgmt0
ip -n "$ns_client" link set mgmt0 up
ip -n "$ns_server" link set mgmt0 up

# The pools of the configuration become local routes on the loopback of their
# namespace (AnyIP), and the router routes each pool to its arm.
read -r client_pool server_pool < <(python3 -I "$mix_config" pools) || die "cannot read the pools"
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
  - { name: gate2, scope: run, token: $runner }
EOF
"$harness" exec client "$controller" -config "$work/controller.yaml" 2>"$out/controller.log" &
disown

# call METHOD PATH TOKEN [CURL ARGS...] prints the body and fails on a status of
# 300 or more. The REST listener lives on the loopback of the client namespace.
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

# The module trees come from the tracked files and carry the byte estimates of
# mix.yaml. The controller needs a module that another module requires first.
install_order=$(python3 -I "$mix_config" modules --root "$root" --work "$work" --waived "$waived") ||
  die "cannot stage the module trees"
for name in $install_order; do
  [[ -s $work/$name.tar.gz ]] || die "no module archive for $name"
  call POST /v1/modules "$admin" -H 'Content-Type: application/gzip' \
    --data-binary "@$work/$name.tar.gz" >/dev/null
done
log "installed the modules $install_order"

agent_state() {
  call GET /v1/agents "$runner" | python3 -I -c 'import json, sys
for a in json.load(sys.stdin):
    if a.get("id") == sys.argv[1]:
        print(a.get("state", ""))' "$1"
}

start_agent() {
  local id=$1 side=$2 ns=$3 dial=$4 state=""
  call POST /v1/agents "$admin" -H 'Content-Type: application/json' \
    -d "{\"id\":\"$id\",\"labels\":{\"gate\":\"2\",\"side\":\"$side\"}}" >/dev/null
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

start_agent gate2-client client client 127.0.0.1
start_agent gate2-server server server "$mgmt_client"

# store_config ROLE AGENT STEADY prints the id of a stored and valid configuration.
store_config() {
  local role=$1 id=$2 hold=$3 file="$work/$1.yaml" config report
  python3 -I "$mix_config" config --role "$role" --agent "$id" --rate-bps "$rate_bps" --ramp-s "$ramp" \
    --steady-s "$hold" --ramp-down-s "$ramp_down" --seed "$seed" --waived "$waived" --basis "$basis" --out "$file" ||
    die "cannot write the $role configuration"
  config=$(call POST /v1/configs "$runner" -H 'Content-Type: application/yaml' \
    --data-binary "@$file" | json_field id)
  report=$(call POST "/v1/configs/$config:validate" "$runner")
  [[ $(json_field valid <<<"$report") == True ]] || fail "the $role configuration is not valid: $report"
  echo "$config"
}

# The server run holds longer than the client run, and the stage stops it once the
# client run ends.
client_config=$(store_config client gate2-client "$steady")
server_config=$(store_config server gate2-server "$((steady + ramp + ramp_down + 150))")

start_run() {
  call POST /v1/runs "$runner" -H 'Content-Type: application/json' \
    -d "{\"config_id\":\"$1\",\"agents\":[\"$2\"]}" | json_field id
}

# wait_run RUN WANT... waits until the run reaches one of the wanted states and prints it.
wait_run() {
  local run=$1 got=""
  shift
  for _ in $(seq $((10 * (steady + ramp + ramp_down + 300)))); do
    got=$(call GET "/v1/runs/$run" "$runner" | json_field state)
    for want in "$@"; do
      if [[ $got == "$want" ]]; then
        echo "$got"
        return 0
      fi
    done
    case $got in completed | failed | aborted) fail "run $run is $got, want $*" ;; esac
    sleep 0.2
  done
  fail "run $run is $got, want $*"
}

server_run=$(start_run "$server_config" gate2-server)
wait_run "$server_run" running >/dev/null
# The server side is ready when it listens on every port of the mix.
first_server=$(python3 -I -c 'import ipaddress, sys; print(next(ipaddress.ip_network(sys.argv[1]).hosts()))' "$server_pool")
ready=""
for _ in $(seq 300); do
  missing=""
  while read -r proto port; do
    flag=l${proto:0:1}n
    "$harness" exec server ss -H"$flag" src "$first_server" | grep -q ":$port " || missing+=" $proto/$port"
  done < <(python3 -I "$mix_config" ports --waived "$waived")
  if [[ -z $missing ]]; then
    ready=1
    break
  fi
  sleep 0.2
done
[[ -n $ready ]] || fail "the server agent does not listen on $first_server for$missing"

"$harness" capture start "$out/mix.pcapng" client
client_run=$(start_run "$client_config" gate2-client)
log "client run $client_run and server run $server_run started"
client_state=$(wait_run "$client_run" completed failed)
call POST "/v1/runs/$server_run:stop" "$runner" -H 'Content-Type: application/json' -d '{}' >/dev/null
server_state=$(wait_run "$server_run" completed failed)
"$harness" capture stop
# tcpdump counts the packets that the kernel dropped when it stops.
cp "$FPS_STATE_DIR/$FPS_NETNS.capture.log" "$out/tcpdump.log" 2>/dev/null || : >"$out/tcpdump.log"
dropped=$(sed -n 's/^\([0-9][0-9]*\) packets dropped by kernel.*/\1/p' "$out/tcpdump.log" | tail -n 1)
log "client run $client_run is $client_state and server run $server_run is $server_state"

# The run object carries the calibration steps (Run.calibration), which the report shows.
call GET "/v1/runs/$client_run" "$runner" >"$out/run.json"
call GET "/v1/runs/$client_run/report" "$runner" >"$out/run-report.json"
call GET "/v1/runs/$client_run/report?format=csv" "$runner" >"$out/run-report.csv"
call GET "/v1/runs/$client_run/metrics?view=scenario" "$runner" >"$out/run-metrics-scenario.json"
call GET "/v1/runs/$server_run/report" "$runner" >"$out/server-run-report.json"
cp "$work/client.yaml" "$out/test-config.yaml"
# The check scores the capture against the byte weights, whatever basis the controller ran.
python3 -I "$mix_config" config --role client --agent gate2-client --rate-bps "$rate_bps" --ramp-s "$ramp" \
  --steady-s "$steady" --ramp-down-s "$ramp_down" --seed "$seed" --waived "$waived" --basis bytes \
  --out "$out/score-config.yaml"
python3 -I "$mix_config" labels --waived "$waived" --out "$out/expected.yaml"
python3 -I -c 'import json, sys
keys = ("client_run", "server_run", "server_run_state", "rate_bps", "ramp_s", "steady_s", "ramp_down_s",
        "seed", "basis", "tolerance_points", "waived", "parsers", "capture_dropped", "commit")
json.dump(dict(zip(keys, sys.argv[1:])), open(sys.argv[-1] + "/context.json", "w"), indent=2)' \
  "$client_run" "$server_run" "$server_state" "$rate_bps" "$ramp" "$steady" "$ramp_down" "$seed" \
  "$basis" "$tolerance" "${waived:-none}" "$parsers" "${dropped:-unknown}" \
  "$(git -c safe.directory="$root" -C "$root" rev-parse --short HEAD 2>/dev/null || echo unknown)" "$out"
# The module trees and their archives are inputs, not results.
rm -rf "$work/modules" "$work"/*.tar.gz

# Under set -e a failed stage ends the script with the code of the report.
score_and_report "$out"
