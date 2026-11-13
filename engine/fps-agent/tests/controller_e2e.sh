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
# A second test (AGT-07) follows the shape of contracts/examples/tests/
# dns_zone.yaml. It sets the module setting zone_file of the first party dns
# module to a zone file that the script adds to the module directory, and the
# lookups ask for a name that only that zone holds. The run passes only when
# the agent applied the setting, and the same test without the setting, which
# the shipped zone cannot answer, fails its criteria.
#
# A third test (AGT-13) runs the smtp/relay scenario of the first party smtp
# module. Its client calls the template functions of smtp and of the content
# module (the message body and the attachment), so the run passes only when
# the agent dispatches the native functions of both crates.
#
# A fourth test (AGT-14) runs the https_fetch scenario of the first party http1
# module, which upgrades both sides to TLS with a browser profile and the server
# profile of the agent, and it carries criteria with a `where` filter on the
# side. The run passes only when the agent builds the TLS profiles of the plan,
# since a session that upgrades without profiles fails, and when it accepts the
# labels, since it refused a plan with a labeled criterion before. A second run
# demands a server side volume that no run reaches and must end failed, which
# shows that the criterion with the label decides the verdict. No SMTP STARTTLS
# scenario exists before MOD-20, so the STARTTLS path is tested by the runtime
# tests of ENG-20, and the agent tests of AGT-14 cover https.
#
# A fifth test (AGT-12) captures through the REST API of the controller. On a dns
# run it starts a capture with the server port of the run, stops it with
# POST /v1/runs/{id}/captures/{capture_id}:stop, downloads it, and checks the
# digest, size, and packet count against Capture.files, and that the pcapng
# holds only UDP packets of that port. A port with no session leaves a valid file
# with no packet, and a port name of the test configuration records the sessions
# of its pools. On an https run a capture with export_tls_keys true must hold a
# decryption secrets block (pcapng type 0x0A) with the TLS key log, which closes
# the loop of AGT-14, and one without must hold none. A small reader in this
# script parses the files, and tshark reads them as well when it is installed.
#
# A sixth test (AGT-16) runs the ftp/mixed_session scenario of the first party ftp
# module, which has a control flow and three spawned data flows. The agent runs
# it as a scenario instance, so every instance gives one succeeded session of
# the application ftp, which holds the control connection and one data connection
# for each of the three transfers (CON-31). The run passes only when the agent
# lifts the check on the number of flows and compiles the scenario with its
# `spawn` actions.
#
# A seventh test (AGT-17) runs ssh/interactive_shell and ssh/scp_upload of the
# first party ssh module, one run each. Every session does a real curve25519
# key exchange and then runs an encrypted SSH transport, so a session succeeds
# only when both sides derived the same keys. The agent holds the state of one
# SSH session in one value of the ssh functions per session side, and the runs
# pass only when no side shares that state with another session. The run
# parameters shorten the shell to one command of a few keys and the upload to a
# small file, so whole sessions finish within the three seconds of a run.
#
# An eighth test (AGT-19) runs smb2/download and smb2/upload of the first party
# smb2 module, one run each, with a synthetic NTLMv2 session setup in every
# session. The run parameters name a server port above 1023 (18687 and 18688)
# because the script runs without root, and a bind to 445 needs a privilege.
# nDPI classifies SMB only on 445, but this test checks the verdict and the
# succeeded sessions, not the classification. Both sizes are small multiples of
# the 64 KiB read size, so a whole session finishes within the run.
#
# A ninth test (AGT-20) runs sip_rtp/call_pcmu of the first party sip_rtp module,
# a SIP dialog over UDP that spawns an RTP media flow, so the agent runs it as
# a scenario instance with two UDP flows. Every call gives one succeeded
# session of the application sip_rtp (CON-31). The SIP port (18689) and the RTP range
# (28000 to 28998) lie above 1023 because the script runs without root. The run
# parameters shorten the ring time and the talk time, so whole calls finish
# within the three seconds of a run. A capture of the port server0 on a second
# run must hold RTP packets: UDP packets with version 2, payload type 0 (PCMU),
# and 160 bytes of payload after the 12 byte header, found by a reader in this
# script that does not use the code of the agent.
#
# A tenth test (AGT-21) runs one scenario of each of the first party modules ntp,
# mqtt, ldap, and hls, one run each: ntp/sntp_query (UDP), mqtt/publish_qos0,
# ldap/bind_search, and hls/vod_segments, which spawns one segment flow per
# segment, so that run needs the multi flow TCP path of the agent. ntp is
# stateless, and mqtt, ldap, and hls hold the state of one session or flow side
# in one value per side. All server ports lie above 1023 because the script runs
# without root. The run parameters shorten the gaps and the segment duration, so
# whole sessions finish within the three seconds of a run.
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
modules=$work/modules
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
  "$agent" connect --controller "https://$grpc" --agent-id local --modules "$modules" \
    --cert-dir "$work/agent-pki" --tls-ca "$ca" --enroll-address "$enroll" \
    --enroll-token-file "$work/token" 2>>"$work/agent.log" &
  agent_pid=$!
  pids+=("$agent_pid")
}

# The agent loads its modules from $modules. http1 is the first party module,
# whose single_fetch scenario the first test runs and whose https_fetch scenario
# the TLS test runs (AGT-14). dns is the first party module, because its native functions read the zone_file
# setting, with the native crate and the tests left out, plus a zone file for
# the name www.e2e.test that the shipped zone lacks. smtp and content are
# first party modules too (AGT-13), installed the same way, because the smtp
# scenarios call the functions of both. The agent and the controller see the
# same trees, so their digests agree.
mkdir -p "$modules"
cp -r "$here/../../../modules/http1" "$modules/http1"
# The manifest names fixtures/http1.pcapng, so the fixtures stay.
rm -r "$modules/http1/native" "$modules/http1/tests"
cp -r "$here/../../../modules/dns" "$modules/dns"
rm -r "$modules/dns/native" "$modules/dns/tests"
cp -r "$here/../../../modules/content" "$modules/content"
rm -r "$modules/content/native" "$modules/content/tests"
cp -r "$here/../../../modules/smtp" "$modules/smtp"
# The manifest names fixtures/smtp.pcapng, and the registry checks that the
# tree holds every file that the manifest names, so the fixtures stay.
rm -r "$modules/smtp/native" "$modules/smtp/tests"
# ftp requires content, which is installed above (AGT-16).
cp -r "$here/../../../modules/ftp" "$modules/ftp"
rm -r "$modules/ftp/native" "$modules/ftp/tests"
# smb2 is the first party module of AGT-19. It keeps its fixtures too.
cp -r "$here/../../../modules/smb2" "$modules/smb2"
rm -r "$modules/smb2/native"
# ssh is the first party module of AGT-17. The manifest names
# fixtures/ssh.pcapng, so the fixtures stay.
cp -r "$here/../../../modules/ssh" "$modules/ssh"
rm -r "$modules/ssh/native" "$modules/ssh/tests"
# sip_rtp is the first party module of AGT-20. It keeps its fixtures too.
cp -r "$here/../../../modules/sip_rtp" "$modules/sip_rtp"
rm -r "$modules/sip_rtp/native"
# ntp, mqtt, ldap, and hls are the first party modules of AGT-21. They keep their
# fixtures, which the manifests name. hls requires http1, which is installed
# above.
for m in ntp mqtt ldap hls; do
  cp -r "$here/../../../modules/$m" "$modules/$m"
  rm -r "$modules/$m/native" "$modules/$m/tests"
done
cat >"$modules/dns/zones/e2e.test.zone" <<'EOF'
$ORIGIN e2e.test.
$TTL 60
@   IN SOA ns1 hostmaster 1 7200 3600 1209600 60
@   IN NS  ns1
ns1 IN A   192.0.2.53
www IN A   192.0.2.77
EOF

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

# content goes before smtp, which requires it.
for name in http1 dns content smtp ftp ssh smb2 sip_rtp ntp mqtt ldap hls; do
  (cd "$modules/$name" && tar -czf "$work/$name.tar.gz" -- *)
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
    - { scenario: http1/single_fetch, weight: 1, params: { server_port: 18680 } }
load:
  objective: { type: sessions_per_second, value: 40 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 7 }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.05 }
EOF

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

# run_test FILE [STATE] stores the test configuration FILE, validates it, runs
# it for three seconds on the agent `local`, and waits for the run to reach
# STATE, which is completed by default. A run whose criteria fail ends in the
# state failed. It sets run, summary, and metrics, which holds the application
# view of the metrics.
run_test() {
  local config report
  config=$(call POST /v1/configs "$runner" -H 'Content-Type: application/yaml' --data-binary "@$1" | json_field id)
  report=$(call POST "/v1/configs/$config:validate" "$runner")
  [[ $(json_field valid <<<"$report") == True ]] || fail "the configuration $1 is not valid: $report"
  run=$(call POST /v1/runs "$runner" -H 'Content-Type: application/json' -d "{\"config_id\":\"$config\",\"agents\":[\"local\"]}" | json_field id)
  wait_run running
  sleep 3
  call POST "/v1/runs/$run:stop" "$runner" -H 'Content-Type: application/json' -d '{}' >/dev/null
  wait_run "${2:-completed}"
  summary=$(call GET "/v1/runs/$run" "$runner")
  metrics=$(call GET "/v1/runs/$run/metrics?view=application" "$runner")
}

# succeeded_of APPLICATION prints the succeeded sessions of the application
# view in $metrics.
succeeded_of() {
  python3 -I -c 'import json, sys
s = json.load(sys.stdin)
print(int(sum(x["values"].get("sessions.succeeded", 0) for x in s.get("samples", []) if x.get("key") == sys.argv[1])))' "$1" <<<"$metrics"
}

run_test "$work/test.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "run $run did not pass: $summary"
succeeded=$(succeeded_of http1)
[[ $succeeded -gt 0 ]] || fail "run $run counted no succeeded sessions: $metrics"

state=""
for _ in $(seq 100); do
  state=$(call GET /v1/agents "$runner" | json_field 0.state)
  [[ $state == online ]] && break
  sleep 0.1
done
[[ $state == online ]] || fail "after the run the agent is $state, want online"
echo "controller_e2e: run $run completed with $succeeded succeeded sessions, verdict passed"

# The dns zone test of AGT-07, in the shape of contracts/examples/tests/
# dns_zone.yaml, with loopback pools and the lookup_a scenario of the first
# party dns module. The lookups ask for www.e2e.test, a name that only the
# zone file of the setting holds, and the server refuses names outside its
# zone.
dns_test() {
  local settings=$1
  cat <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: dns_zone
  description: DNS lookups that the server answers from a configured zone file.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.93.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.94.0.0/24, count: 1 }
$settings
profile:
  basis: sessions
  mix:
    - { scenario: dns/lookup_a, weight: 1, params: { host: www.e2e.test, server_port: 18681 } }
load:
  objective: { type: sessions_per_second, value: 40 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 3 }
criteria:
  - { metric: sessions.failed_ratio, op: "==", value: 0 }
  - { metric: sessions.succeeded, op: ">", value: 0 }
EOF
}

dns_test 'modules:
  dns:
    settings:
      zone_file: zones/e2e.test.zone' >"$work/dns_zone.yaml"
run_test "$work/dns_zone.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the dns zone run $run did not pass: $summary"
dns_succeeded=$(succeeded_of dns)
[[ $dns_succeeded -gt 0 ]] || fail "the dns zone run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run answered from the configured zone with $dns_succeeded succeeded dns sessions, verdict passed"

# Without the setting the server answers from the shipped zone, which has no
# name under e2e.test, so the lookups fail and the run ends in the state
# failed, with the criteria as its reason.
dns_test '' >"$work/dns_default.yaml"
run_test "$work/dns_default.yaml" failed
[[ $(json_field verdict.passed <<<"$summary") != True ]] || fail "the dns run $run without the setting passed, so the zone file made no difference: $summary"
grep -q "\"run_id\":\"$run\".*criteria failed" "$work/controller.log" || fail "the dns run $run without the setting did not fail its criteria"
[[ $(succeeded_of dns) == 0 ]] || fail "the dns run $run without the setting counted succeeded sessions: $metrics"
echo "controller_e2e: run $run without zone_file ended failed on its criteria, as the shipped zone lacks www.e2e.test"

# The smtp relay of AGT-13: an EHLO exchange and one MIME message with a base64
# attachment, which the content module generates.
cat >"$work/smtp_relay.yaml" <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: smtp_relay
  description: SMTP relay sessions of the first party smtp module.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.95.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.96.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - { scenario: smtp/relay, weight: 1, params: { server_port: 18682 } }
load:
  objective: { type: sessions_per_second, value: 20 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 5 }
criteria:
  - { metric: sessions.failed_ratio, op: "==", value: 0 }
  - { metric: sessions.succeeded, op: ">", value: 0 }
EOF
run_test "$work/smtp_relay.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the smtp run $run did not pass: $summary"
smtp_succeeded=$(succeeded_of smtp)
[[ $smtp_succeeded -gt 0 ]] || fail "the smtp run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run relayed mail through the first party smtp and content modules with $smtp_succeeded succeeded sessions, verdict passed"

# The ftp session of AGT-16: one control connection that spawns three data
# connections, a listing, a download, and an upload. The data ports come from a
# range of the scenario variables. The criteria bound the failed ratio, which
# allows for an instance that the stop of the run cuts, and the script checks
# that at least one whole instance, which is one session, succeeded.
cat >"$work/ftp_mixed.yaml" <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: ftp_mixed
  description: FTP sessions with spawned data flows of the first party ftp module.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.99.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.100.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - scenario: ftp/mixed_session
      weight: 1
      params: { server_port: 18684, server_ip: 127.100.0.1, data_port_min: 53000, data_port_max: 53999 }
load:
  objective: { type: sessions_per_second, value: 5 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 13 }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.1 }
  - { metric: sessions.succeeded, op: ">", value: 0 }
EOF
run_test "$work/ftp_mixed.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the ftp run $run did not pass: $summary"
ftp_succeeded=$(succeeded_of ftp)
[[ $ftp_succeeded -ge 1 ]] || fail "the ftp run $run counted $ftp_succeeded succeeded sessions, want at least one whole instance: $metrics"
echo "controller_e2e: run $run ran ftp scenario instances with $ftp_succeeded succeeded sessions (each with its control and data connections), verdict passed"

# The ssh sessions of AGT-17. ssh_test SCENARIO PARAMS PORT SEED prints a test
# configuration for one scenario of the first party ssh module, with the
# parameters PARAMS (the inside of a flow mapping) and the server port PORT.
# The criteria bound the failed ratio, which allows for a session that the stop
# of the run cuts, and the script checks the succeeded sessions of the ssh
# application afterwards.
ssh_test() {
  local scenario=$1 params=$2 port=$3 seed=$4
  cat <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: ssh_$scenario
  description: SSH sessions of the first party ssh module, with a real key exchange and an encrypted transport.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.101.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.102.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - { scenario: ssh/$scenario, weight: 1, params: { server_port: $port, $params } }
load:
  objective: { type: sessions_per_second, value: 10 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: $seed }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.1 }
  - { metric: sessions.succeeded, op: ">", value: 0 }
EOF
}

ssh_test interactive_shell 'commands: 1, typed: 3, think_min_ms: 0, think_max_ms: 20, key_gap_min_ms: 0, key_gap_max_ms: 10, motd_min: 100, motd_max: 300, output_min: 100, output_max: 1000' 18685 17 >"$work/ssh_shell.yaml"
run_test "$work/ssh_shell.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the ssh shell run $run did not pass: $summary"
ssh_shell_succeeded=$(succeeded_of ssh)
[[ $ssh_shell_succeeded -gt 0 ]] || fail "the ssh shell run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run ran ssh/interactive_shell with $ssh_shell_succeeded succeeded sessions, verdict passed"

ssh_test scp_upload 'file_bytes: 65536' 18686 19 >"$work/ssh_scp.yaml"
run_test "$work/ssh_scp.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the ssh scp run $run did not pass: $summary"
ssh_scp_succeeded=$(succeeded_of ssh)
[[ $ssh_scp_succeeded -gt 0 ]] || fail "the ssh scp run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run ran ssh/scp_upload with $ssh_scp_succeeded succeeded sessions, verdict passed"

# The smb2 sessions of AGT-19. smb2_test SCENARIO PARAMS PORT SEED prints a test
# configuration for one scenario of the first party smb2 module, like ssh_test.
smb2_test() {
  local scenario=$1 params=$2 port=$3 seed=$4
  cat <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: smb2_$scenario
  description: SMB2 sessions of the first party smb2 module, with a synthetic NTLMv2 session setup.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.103.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.104.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - { scenario: smb2/$scenario, weight: 1, params: { server_port: $port, $params } }
load:
  objective: { type: sessions_per_second, value: 10 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: $seed }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.1 }
  - { metric: sessions.succeeded, op: ">", value: 0 }
EOF
}

smb2_test download 'file_size: 262144, read_size: 65536, tail_size: 65536' 18687 23 >"$work/smb2_download.yaml"
run_test "$work/smb2_download.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the smb2 download run $run did not pass: $summary"
smb2_download_succeeded=$(succeeded_of smb2)
[[ $smb2_download_succeeded -gt 0 ]] || fail "the smb2 download run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run ran smb2/download with $smb2_download_succeeded succeeded sessions, verdict passed"

smb2_test upload 'upload_size: 262144, write_size: 65536, upload_tail: 65536' 18688 29 >"$work/smb2_upload.yaml"
run_test "$work/smb2_upload.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the smb2 upload run $run did not pass: $summary"
smb2_upload_succeeded=$(succeeded_of smb2)
[[ $smb2_upload_succeeded -gt 0 ]] || fail "the smb2 upload run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run ran smb2/upload with $smb2_upload_succeeded succeeded sessions, verdict passed"

# The sip_rtp calls of AGT-20. sip_test prints a test configuration for
# sip_rtp/call_pcmu with the SIP port 18689. The call is a UDP signaling flow
# and a spawned UDP media flow, so each call gives one session. The criteria
# bound the failed ratio, which allows for the calls that the stop of the run
# cuts, and the script checks the succeeded sessions of the sip_rtp
# application afterwards.
sip_test() {
  cat <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: sip_rtp_call_pcmu
  description: SIP calls with RTP media of the first party sip_rtp module.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.105.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.106.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - scenario: sip_rtp/call_pcmu
      weight: 1
      params: { sip_port: 18689, server_ip: 127.106.0.1, client_ip: 127.105.0.1, rtp_port_min: 28000, rtp_port_max: 28998, ring_ms: 300, talk_ms: 600 }
load:
  objective: { type: sessions_per_second, value: 5 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 31 }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.5 }
  - { metric: sessions.succeeded, op: ">", value: 1 }
EOF
}

sip_test >"$work/sip_rtp.yaml"
run_test "$work/sip_rtp.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the sip_rtp run $run did not pass: $summary"
sip_succeeded=$(succeeded_of sip_rtp)
[[ $sip_succeeded -ge 2 ]] || fail "the sip_rtp run $run counted $sip_succeeded succeeded sessions, want at least two whole calls: $metrics"
echo "controller_e2e: run $run ran sip_rtp/call_pcmu with $sip_succeeded succeeded sessions (each with its SIP dialog and RTP media flow), verdict passed"

# The sessions of AGT-21. module_test MODULE SCENARIO PARAMS SUBNET SEED prints a
# test configuration for one scenario of a first party module, with the
# parameters PARAMS (the inside of a flow mapping, which names the ports), the
# clients in 127.SUBNET.0.0/24, and the servers in the next subnet. The criteria
# bound the failed ratio, which allows for a session that the stop of the run
# cuts, and module_run checks the succeeded sessions of the application.
module_test() {
  local module=$1 scenario=$2 params=$3 subnet=$4 seed=$5
  cat <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: ${module}_$scenario
  description: Sessions of the first party $module module, scenario $scenario.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.$subnet.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.$((subnet + 1)).0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - { scenario: $module/$scenario, weight: 1, params: { $params } }
load:
  objective: { type: sessions_per_second, value: 5 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: $seed }
criteria:
  - { metric: sessions.failed_ratio, op: "<=", value: 0.2 }
  - { metric: sessions.succeeded, op: ">", value: 0 }
EOF
}

# module_run MODULE SCENARIO PARAMS SUBNET SEED MIN runs the test and checks the
# verdict and at least MIN succeeded sessions of the application MODULE.
module_run() {
  local module=$1 scenario=$2 min=$6 count
  module_test "$1" "$2" "$3" "$4" "$5" >"$work/${module}_$scenario.yaml"
  run_test "$work/${module}_$scenario.yaml"
  [[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the $module/$scenario run $run did not pass: $summary"
  count=$(succeeded_of "$module")
  [[ $count -ge $min ]] || fail "the $module/$scenario run $run counted $count succeeded sessions, want at least $min: $metrics"
  echo "controller_e2e: run $run ran $module/$scenario with $count succeeded sessions, verdict passed"
}

# ntp is stateless: a UDP exchange of one 48 byte request and its reply.
module_run ntp sntp_query 'server_port: 18690' 107 37 1
# mqtt and ldap hold the packet or message that recv decoded, one value per side.
module_run mqtt publish_qos0 'server_port: 18691, messages: 2, interval_min_ms: 20, interval_max_ms: 60' 109 41 1
module_run ldap bind_search 'server_port: 18692' 111 43 1
# hls/vod_segments is a playlist flow that spawns one segment flow per segment
# (multi flow TCP). The segments must equal stages times stage_segments, so one
# whole instance gives one session, with the playlists and the six segments.
module_run hls vod_segments 'server_port: 18693, segment_port: 18694, segment_ms: 100, segments: 6, stages: 3, stage_segments: 2' 113 47 1

# The https fetch of AGT-14. Both sides of every session upgrade to TLS at
# once: the client with the browser profile chrome-136, the server with
# server-tls13, whose certificate comes from the authority that the agent
# derives from the seed of the run. The criteria with `where` count the server
# side, see the header.
https_test() {
  local server_bytes=$1
  cat <<EOF
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: https_fetch
  description: HTTPS fetches of the first party http1 module through the TLS profiles of the agent.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/$iface }
    - { name: server0, location: local/$iface }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 127.97.0.0/24, count: 16 }
    - { name: servers, port: server0, role: server, ipv4: 127.98.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - { scenario: http1/https_fetch, weight: 1, params: { server_port: 18683 } }
load:
  objective: { type: sessions_per_second, value: 20 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 11 }
criteria:
  - { metric: sessions.failed_ratio, op: "==", value: 0 }
  - { metric: sessions.succeeded, op: ">", value: 0 }
  - { metric: sessions.succeeded, op: ">", value: 0, where: { side: server } }
  - { metric: bytes.tx, op: ">", value: $server_bytes, where: { side: server } }
EOF
}

https_test 4000 >"$work/https_fetch.yaml"
run_test "$work/https_fetch.yaml"
[[ $(json_field verdict.passed <<<"$summary") == True ]] || fail "the https run $run did not pass: $summary"
https_succeeded=$(succeeded_of http1)
[[ $https_succeeded -gt 0 ]] || fail "the https run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run fetched pages over TLS 1.3 with $https_succeeded succeeded sessions, and its server side criteria held"

# A server side volume of a terabyte is out of reach, so the criterion with the
# label must fail the run while the sessions themselves succeed.
https_test 1000000000000 >"$work/https_fetch_fail.yaml"
run_test "$work/https_fetch_fail.yaml" failed
[[ $(json_field verdict.passed <<<"$summary") != True ]] || fail "the https run $run passed with an impossible server side criterion: $summary"
grep -q "\"run_id\":\"$run\".*criteria failed" "$work/controller.log" || fail "the https run $run did not fail its criteria"
[[ $(succeeded_of http1) -gt 0 ]] || fail "the https run $run counted no succeeded sessions: $metrics"
echo "controller_e2e: run $run ended failed on its server side criterion while its TLS sessions succeeded"

# pcap_report FILE PORT prints a JSON object that describes the pcapng FILE: the
# number of section headers, enhanced packet blocks, decryption secrets blocks,
# TCP and UDP packets, the packets with PORT as source or destination port, and
# whether a secrets block holds a TLS key log with a client traffic secret. It
# checks every block length, and fails on a file that is not a valid pcapng
# file of Ethernet frames. It reads the file with no code of the agent.
pcap_report() {
  python3 -I - "$1" "$2" <<'EOF'
import json, struct, sys

data = open(sys.argv[1], "rb").read()
want = int(sys.argv[2])
r = dict(sections=0, packets=0, secrets=0, tcp=0, udp=0, on_port=0, keylog=False)
at = 0
while at < len(data):
    if at + 12 > len(data):
        sys.exit("block at %d is cut" % at)
    kind, length = struct.unpack_from("<II", data, at)
    if length < 12 or length % 4 or at + length > len(data):
        sys.exit("block at %d has the length %d" % (at, length))
    if struct.unpack_from("<I", data, at + length - 4)[0] != length:
        sys.exit("block at %d has two lengths" % at)
    if kind == 0x0A0D0D0A:
        if data[at + 8:at + 12] != b"\x4d\x3c\x2b\x1a":
            sys.exit("section at %d is not little endian" % at)
        r["sections"] += 1
    elif kind == 6:
        r["packets"] += 1
        cap = struct.unpack_from("<I", data, at + 20)[0]
        f = data[at + 28:at + 28 + cap]
        if f[12:14] == b"\x08\x00":
            proto, l4 = f[23], f[14 + (f[14] & 15) * 4:]
        elif f[12:14] == b"\x86\xdd":
            proto, l4 = f[20], f[54:]
        else:
            sys.exit("packet %d is no IP frame" % r["packets"])
        if proto not in (6, 17):
            sys.exit("packet %d has the protocol %d" % (r["packets"], proto))
        r["tcp" if proto == 6 else "udp"] += 1
        sport, dport = struct.unpack_from(">HH", l4, 0)
        if want in (sport, dport):
            r["on_port"] += 1
    elif kind == 0x0A:
        r["secrets"] += 1
        stype, slen = struct.unpack_from("<II", data, at + 8)
        text = data[at + 16:at + 16 + slen]
        if stype == 0x544C534B and b"CLIENT_TRAFFIC_SECRET_0" in text:
            r["keylog"] = True
    at += length
print(json.dumps(r))
EOF
}

# begin_run FILE stores the test configuration FILE, starts a run on the agent
# `local`, and waits until the agent runs it. It sets run.
begin_run() {
  local config report
  config=$(call POST /v1/configs "$runner" -H 'Content-Type: application/yaml' --data-binary "@$1" | json_field id)
  report=$(call POST "/v1/configs/$config:validate" "$runner")
  [[ $(json_field valid <<<"$report") == True ]] || fail "the configuration $1 is not valid: $report"
  run=$(call POST /v1/runs "$runner" -H 'Content-Type: application/json' -d "{\"config_id\":\"$config\",\"agents\":[\"local\"]}" | json_field id)
  wait_run running
  sleep 2
}

# end_run stops the run of begin_run and waits for the completed run.
end_run() {
  call POST "/v1/runs/$run:stop" "$runner" -H 'Content-Type: application/json' -d '{}' >/dev/null
  wait_run completed
}

# take_capture LABEL PORT KEYS NUMBER starts a capture with PORT and
# export_tls_keys KEYS (true or false) on the running run, lets traffic flow,
# stops it, downloads it, and checks the download against Capture.files: the
# state, the content type, the size, the SHA-256, and the packet count. It sets
# report to the pcap_report of the file, which counts the packets of port NUMBER.
take_capture() {
  local label=$1 port=$2 keys=$3 number=$4 started cap stopped out want got
  out="$work/capture-$label.pcapng"
  started=$(call POST "/v1/runs/$run/captures" "$runner" -H 'Content-Type: application/json' \
    -d "{\"port\":\"$port\",\"export_tls_keys\":$keys}")
  cap=$(json_field id <<<"$started")
  [[ -n $cap ]] || fail "$label: no capture id in $started"
  [[ $(json_field port <<<"$started") == "$port" ]] || fail "$label: the capture answered another port: $started"
  [[ $(json_field state <<<"$started") == running ]] || fail "$label: the capture is not running: $started"
  sleep 2
  stopped=$(call POST "/v1/runs/$run/captures/$cap:stop" "$runner")
  [[ $(json_field state <<<"$stopped") == completed ]] || fail "$label: the stopped capture is not completed: $stopped"
  [[ $(json_field files.0.agent <<<"$stopped") == local ]] || fail "$label: the file is not of the agent local: $stopped"
  call GET "/v1/runs/$run/captures/$cap" "$runner" -D "$work/capture.headers" -o "$out"
  grep -qi '^content-type: application/vnd.tcpdump.pcap' "$work/capture.headers" ||
    fail "$label: the download has another content type: $(cat "$work/capture.headers")"
  want=$(json_field files.0.digest <<<"$stopped")
  got="sha256:$(sha256sum "$out" | cut -d' ' -f1)"
  [[ $want == "$got" ]] || fail "$label: the SHA-256 of the download is $got, Capture.files says '$want'"
  [[ $(stat -c %s "$out") == "$(json_field files.0.bytes <<<"$stopped")" ]] ||
    fail "$label: the download has $(stat -c %s "$out") bytes, Capture.files says: $stopped"
  report=$(pcap_report "$out" "$number") || fail "$label: the download is not a valid pcapng file"
  [[ $(json_field sections <<<"$report") -ge 1 ]] || fail "$label: the download has no section header: $report"
  [[ $(json_field packets <<<"$report") == "$(json_field files.0.packets <<<"$stopped")" ]] ||
    fail "$label: the file holds a different number of packets than Capture.files: $report $stopped"
  if command -v tshark >/dev/null; then
    [[ $(tshark -r "$out" -T fields -e frame.number 2>/dev/null | wc -l) == "$(json_field packets <<<"$report")" ]] ||
      fail "$label: tshark reads another number of packets: $report"
  fi
}

# The dns capture of AGT-12. The port of the capture is the server port of the
# run, which the agent matches against the server end of every session.
dns_test 'modules:
  dns:
    settings:
      zone_file: zones/e2e.test.zone' >"$work/dns_capture.yaml"
begin_run "$work/dns_capture.yaml"
take_capture dns 18681 false 18681
[[ $(json_field packets <<<"$report") -gt 0 ]] || fail "the dns capture holds no packet: $report"
[[ $(json_field tcp <<<"$report") == 0 && $(json_field udp <<<"$report") == "$(json_field packets <<<"$report")" ]] ||
  fail "the dns capture holds more than UDP: $report"
[[ $(json_field on_port <<<"$report") == "$(json_field packets <<<"$report")" ]] ||
  fail "the dns capture holds packets of another port: $report"
[[ $(json_field secrets <<<"$report") == 0 ]] || fail "the dns capture holds a secrets block: $report"
dns_packets=$(json_field packets <<<"$report")
# A port that no session uses leaves a valid file with no packet.
take_capture dns_none 9 false 9
[[ $(json_field packets <<<"$report") == 0 ]] || fail "the capture of port 9 holds packets: $report"
# The name of a port of the test configuration records the sessions of its pools.
take_capture dns_name server0 false 18681
[[ $(json_field packets <<<"$report") -gt 0 && $(json_field tcp <<<"$report") == 0 ]] ||
  fail "the capture of the port server0 holds no dns packet: $report"
end_run
echo "controller_e2e: run $run captured $dns_packets dns packets of port 18681 through the REST API, and the digest, size, and packet count of Capture.files held"

# rtp_count FILE prints the number of RTP packets of the pcapng FILE: UDP
# packets whose payload has the version 2, the payload type 0 (PCMU), and 160
# bytes after the 12 byte header, which is what call_pcmu sends every 20 ms.
# take_capture has checked the block structure of the file by then.
rtp_count() {
  python3 -I - "$1" <<'EOF'
import struct, sys

data = open(sys.argv[1], "rb").read()
at, n = 0, 0
while at < len(data):
    kind, length = struct.unpack_from("<II", data, at)
    if kind == 6:
        cap = struct.unpack_from("<I", data, at + 20)[0]
        f = data[at + 28:at + 28 + cap]
        if f[12:14] == b"\x08\x00" and f[23] == 17:
            body = f[14 + (f[14] & 15) * 4 + 8:]
            if len(body) == 172 and body[0] >> 6 == 2 and body[1] & 0x7F == 0:
                n += 1
    at += length
print(n)
EOF
}

# The sip_rtp capture of AGT-20. The name of the port of the servers records
# the sessions of the pool, the SIP dialog on port 18689 and the RTP media flow
# on the ports that the calls draw, so the file holds UDP only, SIP requests,
# and RTP packets.
begin_run "$work/sip_rtp.yaml"
take_capture sip_rtp server0 false 18689
sip_packets=$(json_field packets <<<"$report")
[[ $sip_packets -gt 0 && $(json_field tcp <<<"$report") == 0 && $(json_field udp <<<"$report") == "$sip_packets" ]] ||
  fail "the sip_rtp capture holds no packet or holds more than UDP: $report"
[[ $(json_field on_port <<<"$report") -gt 0 ]] || fail "the sip_rtp capture holds no packet of the SIP port 18689: $report"
[[ $(json_field on_port <<<"$report") -lt $sip_packets ]] || fail "the sip_rtp capture holds only packets of the SIP port, so the media flow is missing: $report"
rtp_packets=$(rtp_count "$work/capture-sip_rtp.pcapng")
[[ $rtp_packets -ge 20 ]] || fail "the sip_rtp capture holds $rtp_packets RTP packets of PCMU with 160 bytes, want at least 20 (one call): $report"
end_run
echo "controller_e2e: run $run captured $sip_packets sip_rtp packets, $rtp_packets of them RTP (PCMU, 160 byte payload)"

# The https capture of AGT-12 and AGT-14: with export_tls_keys the file holds the
# TLS key log as a decryption secrets block, and without it the file holds none.
https_test 4000 >"$work/https_capture.yaml"
begin_run "$work/https_capture.yaml"
take_capture https_keys 18683 true 18683
https_packets=$(json_field packets <<<"$report")
[[ $https_packets -gt 0 && $(json_field udp <<<"$report") == 0 ]] || fail "the https capture holds no TCP packet: $report"
[[ $(json_field on_port <<<"$report") == "$https_packets" ]] || fail "the https capture holds packets of another port: $report"
[[ $(json_field secrets <<<"$report") -ge 1 && $(json_field keylog <<<"$report") == True ]] ||
  fail "the https capture with export_tls_keys holds no TLS key log block: $report"
take_capture https_plain client0 false 18683
[[ $(json_field packets <<<"$report") -gt 0 && $(json_field secrets <<<"$report") == 0 ]] ||
  fail "the https capture without export_tls_keys holds a secrets block or no packet: $report"
end_run
echo "controller_e2e: run $run captured $https_packets https packets, and the file with export_tls_keys held a TLS key log in a decryption secrets block"

# A CA that is not the controller CA. The agent checks the enrollment endpoint
# against it, so the handshake fails before the token leaves the agent.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 1 \
  -subj "/CN=wrong ca" -keyout "$work/wrong-ca.key" -out "$work/wrong-ca.pem" 2>/dev/null ||
  fail "openssl could not make a CA"
echo "unused.token" >"$work/unused-token"
set +e
"$agent" connect --controller "https://$grpc" --agent-id other --modules "$modules" \
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
