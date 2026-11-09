#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Offline test of image/convert.sh. It makes a tiny qcow2 whose size is not a multiple of
# 1 MiB, converts it, and checks the VHD rule, the OVA member order, the manifest, and the
# checksums. It needs qemu-img. Without it the test prints SKIP and succeeds, unless
# FPS_REQUIRE_IMAGE_TOOLS=1 makes the missing tool a failure.
set -euo pipefail

img="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fails=0
pass() { echo "PASS $1"; }
fail() {
	echo "FAIL $1"
	fails=$((fails + 1))
}

if ! command -v qemu-img >/dev/null 2>&1; then
	if [[ "${FPS_REQUIRE_IMAGE_TOOLS:-0}" == 1 ]]; then
		echo "FAIL qemu-img is missing and FPS_REQUIRE_IMAGE_TOOLS=1"
		exit 1
	fi
	echo "SKIP convert-test: qemu-img is not installed (set FPS_REQUIRE_IMAGE_TOOLS=1 to make that a failure)"
	exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mib=1048576

# 5000 KiB is not a multiple of 1 MiB, so the script has to round up.
qemu-img create -q -f qcow2 "$tmp/tiny.qcow2" 5000K
if "$img/convert.sh" -o "$tmp/out" "$tmp/tiny.qcow2" >"$tmp/convert.log" 2>&1; then
	pass "convert.sh converts a tiny qcow2 to every format"
else
	fail "convert.sh converts a tiny qcow2 to every format"
	cat "$tmp/convert.log"
	exit 1
fi
out="$tmp/out"

for f in tiny.raw tiny.vhdx tiny.vhd tiny.ova SHA256SUMS; do
	if [[ -s "$out/$f" ]]; then pass "$f exists"; else fail "$f exists"; fi
done

# Azure rule: a fixed VHD is a multiple of 1 MiB plus the 512 byte footer.
vhd_size="$(stat -c %s "$out/tiny.vhd")"
if ((vhd_size > 512 && (vhd_size - 512) % mib == 0)); then
	pass "the VHD size minus the footer is a multiple of 1 MiB ($vhd_size bytes)"
else
	fail "the VHD size minus the footer is a multiple of 1 MiB ($vhd_size bytes)"
fi
raw_size="$(stat -c %s "$out/tiny.raw")"
if ((raw_size > 0 && raw_size % mib == 0)); then
	pass "the raw disk is a multiple of 1 MiB ($raw_size bytes)"
else
	fail "the raw disk is a multiple of 1 MiB ($raw_size bytes)"
fi
if qemu-img info -f vpc "$out/tiny.vhd" | grep -q 'virtual size.*5 MiB'; then
	pass "qemu-img reads the VHD with the exact 5 MiB size"
else
	fail "qemu-img reads the VHD with the exact 5 MiB size"
fi
if qemu-img check -f vhdx "$out/tiny.vhdx" >/dev/null; then pass "qemu-img check accepts the VHDX"; else fail "qemu-img check accepts the VHDX"; fi

# The OVA: .ovf first, then the .vmdk, then the .mf.
order="$(tar -tf "$out/tiny.ova" | tr '\n' ' ')"
if [[ "$order" == "tiny.ovf tiny.vmdk tiny.mf " ]]; then
	pass "the OVA holds .ovf, .vmdk, .mf in this order"
else
	fail "the OVA holds .ovf, .vmdk, .mf in this order (got: $order)"
fi
mkdir "$tmp/ova"
tar -C "$tmp/ova" -xf "$out/tiny.ova"
if (cd "$tmp/ova" && sed -E 's/^SHA256\((.*)\)= ([0-9a-f]{64})$/\2  \1/' tiny.mf | sha256sum -c --quiet -); then
	pass "the manifest lists SHA256 lines that match the .ovf and the .vmdk"
else
	fail "the manifest lists SHA256 lines that match the .ovf and the .vmdk"
fi
if [[ "$(grep -c '^SHA256(' "$tmp/ova/tiny.mf")" == 2 ]]; then pass "the manifest has one line per member"; else fail "the manifest has one line per member"; fi
if qemu-img info -f vmdk "$tmp/ova/tiny.vmdk" | grep -q 'streamOptimized'; then
	pass "the VMDK is streamOptimized"
else
	fail "the VMDK is streamOptimized"
fi
ovf="$tmp/ova/tiny.ovf"
if grep -q 'ovf/envelope/1' "$ovf" && grep -q '<ovf:File\|<File ' "$ovf" && grep -q 'ovf:href="tiny.vmdk"' "$ovf" &&
	grep -q '<rasd:ResourceType>3<' "$ovf" && grep -q '<rasd:ResourceType>4<' "$ovf" &&
	grep -q '<rasd:ResourceType>17<' "$ovf" && [[ "$(grep -c '<rasd:ResourceType>10<' "$ovf")" == 2 ]]; then
	pass "the OVF names the CPU, the memory, one disk, and two NICs"
else
	fail "the OVF names the CPU, the memory, one disk, and two NICs"
fi
if command -v xmllint >/dev/null 2>&1; then
	if xmllint --noout "$ovf"; then pass "the OVF is well formed XML"; else fail "the OVF is well formed XML"; fi
fi

# SHA256SUMS covers every output and verifies.
if (cd "$out" && sha256sum -c --quiet SHA256SUMS) &&
	[[ "$(wc -l <"$out/SHA256SUMS")" == 4 ]] &&
	for f in tiny.raw tiny.vhdx tiny.vhd tiny.ova; do grep -q "  $f\$" "$out/SHA256SUMS" || exit 1; done; then
	pass "SHA256SUMS lists and verifies every output"
else
	fail "SHA256SUMS lists and verifies every output"
fi

# Without raw in the list, the intermediate raw disk is gone and the other outputs remain.
if "$img/convert.sh" -f vhd,ova -o "$tmp/out2" "$tmp/tiny.qcow2" >"$tmp/convert2.log" 2>&1 &&
	[[ ! -e "$tmp/out2/tiny.raw" && -s "$tmp/out2/tiny.vhd" && -s "$tmp/out2/tiny.ova" ]] &&
	[[ "$(wc -l <"$tmp/out2/SHA256SUMS")" == 2 ]]; then
	pass "--formats vhd,ova leaves no raw disk behind"
else
	fail "--formats vhd,ova leaves no raw disk behind"
	cat "$tmp/convert2.log"
fi
if "$img/convert.sh" -f vhd -o "$tmp/out3" --keep-raw "$tmp/tiny.qcow2" >/dev/null 2>&1 && [[ -s "$tmp/out3/tiny.raw" ]]; then
	pass "--keep-raw keeps the raw disk"
else
	fail "--keep-raw keeps the raw disk"
fi

# Bad input fails with a nonzero code.
if ! "$img/convert.sh" -f bogus -o "$tmp/out4" "$tmp/tiny.qcow2" >/dev/null 2>&1 && ! "$img/convert.sh" "$tmp/missing.qcow2" >/dev/null 2>&1; then
	pass "an unknown format and a missing input fail"
else
	fail "an unknown format and a missing input fail"
fi

if ((fails > 0)); then
	exit 1
fi
