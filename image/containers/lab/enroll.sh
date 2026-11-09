#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
#
# Step "enroll" of the container lab (IMG-05). It runs as user 10001 (the user of the
# controller, which owns the admin socket) with the group 10002 (the group of the
# agents), after the controller started.
#
#   1. It waits for the admin socket and the CA certificate of the controller.
#   2. It copies the public CA certificate into the volume "ca", which the agents
#      mount read only. The CA key stays in the volume "ctl-data".
#   3. It mints one enrollment token per agent through the admin socket with
#      `fps-controller token` and writes it into the directory of that agent
#      (the volume a1-enroll or a2-enroll, mode 0640, group of the agents).
#
# The token is a secret that is valid once. It is never an argument, never in the
# environment, and never in a log line: the controller prints it on standard output
# and the script redirects that into the file. Standard error holds only the expiry.
set -eu

umask 0027
socket=/var/lib/fps/pki/admin.sock
ca=/var/lib/fps/pki/ca.crt
config=/etc/fps/controller.yaml
wait_seconds=${FPS_LAB_WAIT:-60}

i=0
while [ ! -S "$socket" ] || [ ! -s "$ca" ]; do
	i=$((i + 1))
	if [ "$i" -gt "$wait_seconds" ]; then
		echo "enroll: the controller wrote no admin socket or CA certificate within ${wait_seconds}s" >&2
		exit 1
	fi
	sleep 1
done

cp "$ca" /ca/ca.crt.tmp
chmod 0644 /ca/ca.crt.tmp
mv /ca/ca.crt.tmp /ca/ca.crt
echo "enroll: published the CA certificate /ca/ca.crt"

for id in agent1 agent2; do
	out=/enroll/$id/enroll.token
	rm -f "$out" "$out.tmp"
	# -config finds the admin socket and the API token file. -ttl 10m limits the token.
	if ! fps-controller token -agent-id "$id" -ttl 10m -config "$config" >"$out.tmp"; then
		rm -f "$out.tmp"
		echo "enroll: fps-controller token failed for $id" >&2
		echo "enroll: $(id)" >&2
		ls -ld /ca /enroll/agent1 /enroll/agent2 /var/lib/fps /var/lib/fps/pki >&2 || true
		exit 1
	fi
	mv "$out.tmp" "$out"
	echo "enroll: wrote the enrollment token of $id to $out"
done
