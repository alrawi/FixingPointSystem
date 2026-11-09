#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
#
# Step "prep" of the container lab (IMG-05). It writes the admin API token of the
# controller into the volume "secrets" when the file does not exist. The controller
# reads the file at start (token_file in controller.yaml), and the test reads it to
# call the REST API. The token never appears in an argument, in the environment, or
# in a log line: it reaches the file through a redirect only.
set -eu

file=/secrets/admin.token
umask 0077
if [ ! -s "$file" ]; then
	# 32 random bytes, URL safe, above the 16 byte minimum of the controller.
	head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=\n' >"$file.tmp"
	mv "$file.tmp" "$file"
	echo "prep: created $file"
fi
chmod 0600 "$file"
