#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Image stage of the Gate 2 suite. The stage builds the qcow2 appliance image,
# boots it under QEMU, runs the checks of the boot test, and runs a short HTTP and
# DNS test unattended inside the VM, which is IMG-04 as one stage of the gate.
#
#   tests/gates/gate2/image.sh OUT_DIR [IMAGE]
#
# Without IMAGE the stage runs image/build.sh first, which needs Packer, QEMU, Go,
# and the Rust musl target. With IMAGE it boots that image and builds nothing. The
# stage writes OUT_DIR/image/boot-test.log, the files of the run in the VM under
# OUT_DIR/image/run/, and OUT_DIR/image/context.json with the accelerator and the
# image digest. It exits with the code of the boot test. gate2_report.py reads
# the log and the run files, so the verdict never rests on the exit code alone.
#   FPS_ACCEL         kvm or tcg, the accelerator that the workflow chose. tcg sets
#                     FPS_ALLOW_TCG=1 and the longer waits of the CI workflow
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
readonly here root

die() {
  echo "image.sh: $*" >&2
  exit 2
}

[[ $# -ge 1 && $# -le 2 ]] || die "usage: image.sh OUT_DIR [IMAGE]"
mkdir -p "$1/image"
out=$(cd "$1/image" && pwd)
readonly out
image=${2:-$root/image/output/fps-base.qcow2}

accel=${FPS_ACCEL:-}
if [[ -z $accel ]]; then
  if [[ -w /dev/kvm ]]; then accel=kvm; else accel=tcg; fi
fi
if [[ $accel == tcg ]]; then
  export FPS_ALLOW_TCG=1
  # The boot test multiplies the boot wait by 4 under software emulation.
  export FPS_TEST_BOOT_WAIT=${FPS_TEST_BOOT_WAIT:-600}
  export FPS_RUN_RATE=${FPS_RUN_RATE:-10}
  export FPS_RUN_STEADY_S=${FPS_RUN_STEADY_S:-10}
  export FPS_RUN_WAIT_S=${FPS_RUN_WAIT_S:-600}
  export FPS_SSH_TIMEOUT=${FPS_SSH_TIMEOUT:-60m}
fi

built=no
if [[ $# -eq 1 ]]; then
  echo "image.sh: building the image with $accel" >&2
  (cd "$root" && FPS_ACCEL="$accel" image/build.sh) 2>&1 | tee "$out/build.log"
  built=yes
fi
[[ -s $image ]] || die "no image at $image"

rc=0
FPS_TEST_ARTIFACT_DIR="$out" FPS_TEST_REPEAT=1 \
  "$root/image/boot-test.sh" --role all-in-one --run "$image" 2>&1 | tee "$out/boot-test.log" || rc=${PIPESTATUS[0]}

sha=$(sha256sum "$image" | cut -d' ' -f1)
python3 -I -c 'import json, os, sys
keys = ("accelerator", "built_by_stage", "image_bytes", "image_sha256", "boot_test_exit")
json.dump(dict(zip(keys, sys.argv[1:6])), open(sys.argv[6] + "/context.json", "w"), indent=2)' \
  "$accel" "$built" "$(stat -c %s "$image")" "$sha" "$rc" "$out"
exit "$rc"
