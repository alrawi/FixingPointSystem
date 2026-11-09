#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Runs the real fps-controller against the real fps-agent over loopback.
#
# Usage: controller_e2e.sh FPS_CONTROLLER FPS_AGENT
#
# The script starts the controller with a temporary configuration, which
# serves the agent over mutual TLS (CTL-27). It mints an enrollment token
# with `fps-controller token`, starts `fps-agent connect` with the token and
# the CA file of the controller (AGT-08), and waits until the agent has
# enrolled over HTTPS and registered over mutual TLS. It installs the example
# modules, stores a small loopback test configuration, starts a run, waits
# for it to run, stops it, and waits for the completed run. It then checks
# that the run counted succeeded sessions and that the agent went back to
# online.
#
# The tail of the script checks the handshake failures. An agent that trusts
# another CA ends with exit code 2 and a TLS error before it spends its
# token. After `fps-controller revoke`, the running agent ends with exit code
# 2 as well, because the controller refuses its certificate at the next
# dial, and a new token enrolls it again.
#
# It needs curl, tar, python3, and openssl, and no root.
set -euo pipefail

controller=${1:?usage: controller_e2e.sh FPS_CONTROLLER FPS_AGENT}
agent=${2:?usage: controller_e2e.sh FPS_CONTROLLER FPS_AGENT}
here=$(cd "$(dirname "$0")" && pwd)
examples=$(cd "$here/../../../contracts/examples" && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/fps-agent-e2e.XXXXXX")
admin=e2e-admin-0123456789
runner=e2e-run-0123456789
pids=()
agent_pid=""

cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

fail() {
  echo "controller_e2e: $*" >&2
  echo "--- controller log" >&2
  tail -n 40 "$work/controller.log" >&2 || true
  echo "--- agent log" >&2
  tail -n 40 "$work/agent.log" >&2 || true
  exit 1
}

cat >"$work/controller.yaml" <<EOF
listen: 127.0.0.1:0
grpc_listen: 127.0.0.1:0
enroll_listen: 127.0.0.1:0
log_level: debug
data_dir: $work/data
tokens:
  - { name: admin, scope: admin, token: $admin }
  - { name: ci, scope: run, token: $runner }
EOF

"$controller" -config "$work/controller.yaml" 2>"$work/controller.log" &
pids+=($!)

# addr MSG prints the addr field of the first log record with msg MSG.
addr() {
  python3 -I - "$work/controller.log" "$1" <<'EOF'
import json, sys
for line in open(sys.argv[1]):
    try:
        rec = json.loads(line)
    except ValueError:
        continue
    if rec.get("msg") == sys.argv[2]:
        print(rec.get("addr", ""))
        break
EOF
}

rest="" grpc="" enroll=""
for _ in $(seq 150); do
  rest=$(addr "controller listening")
  grpc=$(addr "agent service listening")
  enroll=$(addr "enrollment service listening")
  [[ -n $rest && -n $grpc && -n $enroll ]] && break
  sleep 0.1
done
[[ -n $rest && -n $grpc && -n $enroll ]] || fail "the controller did not listen"
# The controller writes its CA here, in pki_dir, which defaults to data_dir/pki.
ca="$work/data/pki/ca.crt"
[[ -s $ca ]] || fail "the controller wrote no CA certificate at $ca"

# call METHOD PATH TOKEN [CURL ARGS...] prints the body and fails on a status
# of 300 or more.
call() {
  local method=$1 path=$2 token=$3
  shift 3
  curl -sS --fail-with-body -X "$method" -H "Authorization: Bearer $token" "$@" "http://$rest$path" ||
    fail "$method $path failed"
}

json_field() {
  python3 -I -c 'import json, sys; v = json.load(sys.stdin)
for k in sys.argv[1].split("."):
    v = v[int(k)] if isinstance(v, list) else v.get(k)
print("" if v is None else v)' "$1"
}

# mint_token writes a fresh enrollment token for the agent `local` to
# $work/token. Minting a token also adds the identity to the inventory of the
# controller, so the script needs no POST /v1/agents.
mint_token() {
  (umask 077 && "$controller" token -agent-id local -ttl 5m -config "$work/controller.yaml" >"$work/token" 2>>"$work/token.log") ||
    fail "fps-controller token failed: $(cat "$work/token.log")"
  [[ -s $work/token ]] || fail "fps-controller token printed no token"
}

# start_agent starts the agent with the token file $work/token, the CA file
# of the controller, and the enrollment address, and sets agent_pid.
start_agent() {
  "$agent" connect --controller "https://$grpc" --agent-id local --modules "$examples/modules" \
    --cert-dir "$work/agent-pki" --tls-ca "$ca" --enroll-address "$enroll" \
    --enroll-token-file "$work/token" 2>>"$work/agent.log" &
  agent_pid=$!
  pids+=("$agent_pid")
}

mint_token
start_agent

state=""
for _ in $(seq 100); do
  state=$(call GET /v1/agents "$runner" | json_field 0.state)
  [[ $state == online ]] && break
  sleep 0.1
done
[[ $state == online ]] || fail "the agent is $state, want online"
[[ ! -e $work/token ]] || fail "the agent kept its enrollment token file"
[[ $(stat -c %a "$work/agent-pki/agent.key") == 600 ]] || fail "the agent key is not mode 0600"

for name in http1 dns; do
  (cd "$examples/modules/$name" && tar -czf "$work/$name.tar.gz" -- *)
  call POST /v1/modules "$admin" -H 'Content-Type: application/gzip' --data-binary "@$work/$name.tar.gz" >/dev/null
done

# The controller checks port locations against the agent capabilities,
# which leave out loopback, so the test names the first detected interface.
# The loopback pools carry the traffic, and the agent binds no interface.
iface=$("$agent" capabilities | awk '$1 == "port" { print $2; exit }')
[[ -n $iface ]] || fail "the agent detects no traffic interface"

cat >"$work/test.yaml" <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: agent_e2e
  description: A loopback run of the real agent under the real controller.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.91.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.92.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - { scenario: http1/web_get, weight: 1, params: { server_port: 18680 } }
load:
  objective: { type: sessions_per_second, value: 40 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 7 }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.05 }
EOF

config=$(call POST /v1/configs "$runner" -H 'Content-Type: application/yaml' --data-binary "@$work/test.yaml" | json_field id)
report=$(call POST "/v1/configs/$config:validate" "$runner")
[[ $(json_field valid <<<"$report") == True ]] || fail "the configuration is not valid: $report"
run=$(call POST /v1/runs "$runner" -H 'Content-Type: application/json' -d "{\"config_id\":\"$config\",\"agents\":[\"local\"]}" | json_field id)

wait_run() {
  local want=$1 got=""
  for _ in $(seq 300); do
    got=$(call GET "/v1/runs/$run" "$runner" | json_field state)
    [[ $got == "$want" ]] && return 0
    [[ $got == failed || $got == aborted ]] && fail "run $run is $got, want $want"
    sleep 0.1
  done
  fail "run $run is $got, want $want"
}

wait_run running
sleep 3
call POST "/v1/runs/$run:stop" "$runner" -H 'Content-Type: application/json' -d '{}' >/dev/null
wait_run completed

summary=$(call GET "/v1/runs/$run" "$runner")
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "run $run did not pass: $summary"
metrics=$(call GET "/v1/runs/$run/metrics?view=application" "$runner")
succeeded=$(python3 -I -c 'import json, sys
s = json.load(sys.stdin)
print(int(sum(x["values"].get("sessions.succeeded", 0) for x in s.get("samples", []) if x.get("key") == "http1")))' <<<"$metrics")
[[ $succeeded -gt 0 ]] || fail "run $run counted no succeeded sessions: $metrics"

state=""
for _ in $(seq 100); do
  state=$(call GET /v1/agents "$runner" | json_field 0.state)
  [[ $state == online ]] && break
  sleep 0.1
done
[[ $state == online ]] || fail "after the run the agent is $state, want online"
echo "controller_e2e: run $run completed with $succeeded succeeded sessions, verdict passed"

# A CA that is not the controller CA. The agent checks the enrollment endpoint
# against it, so the handshake fails before the token leaves the agent.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 1 \
  -subj "/CN=wrong ca" -keyout "$work/wrong-ca.key" -out "$work/wrong-ca.pem" 2>/dev/null ||
  fail "openssl could not make a CA"
echo "unused.token" >"$work/unused-token"
set +e
"$agent" connect --controller "https://$grpc" --agent-id other --modules "$examples/modules" \
  --cert-dir "$work/other-pki" --tls-ca "$work/wrong-ca.pem" --enroll-address "$enroll" \
  --enroll-token-file "$work/unused-token" 2>"$work/wrong-ca.log"
code=$?
set -e
[[ $code == 2 ]] || fail "an agent with the wrong CA exited with $code, want 2: $(cat "$work/wrong-ca.log")"
grep -q "TLS failure" "$work/wrong-ca.log" || fail "the wrong CA gave no TLS error: $(cat "$work/wrong-ca.log")"
[[ -e $work/unused-token ]] || fail "the agent used its token although the handshake failed"
echo "controller_e2e: an agent that trusts another CA ended with exit code 2: $(tail -n 1 "$work/wrong-ca.log")"

# Revoking the identity ends the session. The agent dials again, the
# controller refuses the revoked certificate in the handshake, and the agent
# ends with exit code 2.
"$controller" revoke -agent-id local -config "$work/controller.yaml" >/dev/null 2>>"$work/token.log" ||
  fail "fps-controller revoke failed: $(cat "$work/token.log")"
code=""
for _ in $(seq 100); do
  if ! kill -0 "$agent_pid" 2>/dev/null; then
    set +e
    wait "$agent_pid"
    code=$?
    set -e
    break
  fi
  sleep 0.1
done
[[ $code == 2 ]] || fail "the agent with a revoked certificate exited with '$code', want 2"
grep -q "the failure is permanent" "$work/agent.log" || fail "the agent did not log a permanent failure"

# A new token enrolls the agent again, over the revoked certificate.
mint_token
start_agent
state=""
for _ in $(seq 100); do
  state=$(call GET /v1/agents "$runner" | json_field 0.state)
  [[ $state == online ]] && break
  sleep 0.1
done
[[ $state == online ]] || fail "after a new enrollment the agent is $state, want online"
echo "controller_e2e: the revoked agent ended with exit code 2 and enrolled again with a new token"
