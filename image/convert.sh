#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Converts the qcow2 appliance image into the formats that hypervisors and clouds import.
#
#   image/convert.sh [options] [input.qcow2]
#
# Formats (--formats takes a comma separated list, default: raw,vhdx,vhd,ova):
#   raw    <name>.raw    raw disk, sized to a multiple of 1 MiB
#   vhdx   <name>.vhdx   Hyper-V
#   vhd    <name>.vhd    fixed VHD for Azure: the file size minus the 512 byte footer is a multiple of 1 MiB
#   ova    <name>.ova    OVF 1.0 package: .ovf, then the streamOptimized .vmdk, then the .mf manifest
# and always SHA256SUMS over every output file of the run.
#
# Options:
#   -o, --out-dir DIR    output directory                  (default: the directory of the input)
#   -f, --formats LIST   formats to write                  (default: raw,vhdx,vhd,ova)
#       --keep-raw       keep the raw disk even when raw is not in --formats
#       --cpus N         vCPUs in the OVF                  (default 4)
#       --memory-mb N    memory in the OVF, in MiB         (default 8192)
#       --nics 1|2       NICs in the OVF: the management NIC first, then one test NIC (default 2)
#   -h, --help           print this text
#
# The VHD needs a raw disk of a 1 MiB multiple. The script makes that raw disk next to the
# outputs and deletes it afterwards unless raw was requested or --keep-raw is set. A format
# that does not fit into the free space is skipped with a warning when it was not asked for
# by --formats, and is an error when it was. qemu-img check runs on qcow2, vhdx, and the
# vmdk; raw and vpc have no check, so qemu-img info runs on them instead.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mib=1048576
input=""
out_dir=""
formats=""
keep_raw=0
cpus=4
memory_mb=8192
nics=2

step() { printf '\n== %s\n' "$*"; }
die() {
	echo "FAIL $*" >&2
	exit 1
}
usage() { sed -n '4,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; }

while (($# > 0)); do
	case "$1" in
	-o | --out-dir)
		[[ $# -ge 2 ]] || die "$1 needs a value"
		out_dir="$2"
		shift 2
		;;
	-f | --formats)
		[[ $# -ge 2 ]] || die "$1 needs a value"
		formats="$2"
		shift 2
		;;
	--keep-raw)
		keep_raw=1
		shift
		;;
	--cpus)
		[[ $# -ge 2 ]] || die "$1 needs a value"
		cpus="$2"
		shift 2
		;;
	--memory-mb)
		[[ $# -ge 2 ]] || die "$1 needs a value"
		memory_mb="$2"
		shift 2
		;;
	--nics)
		[[ $# -ge 2 ]] || die "$1 needs a value"
		nics="$2"
		shift 2
		;;
	-h | --help)
		usage
		exit 0
		;;
	-*) die "unknown option: $1 (see --help)" ;;
	*)
		[[ -z "$input" ]] || die "more than one input file"
		input="$1"
		shift
		;;
	esac
done

[[ -n "$input" ]] || input="$root/image/output/fps-base.qcow2"
[[ -f "$input" ]] || die "no such file: $input (build it with image/build.sh)"
[[ "$cpus" =~ ^[1-9][0-9]*$ ]] || die "--cpus must be a positive integer"
[[ "$memory_mb" =~ ^[1-9][0-9]*$ ]] || die "--memory-mb must be a positive integer"
[[ "$nics" == 1 || "$nics" == 2 ]] || die "--nics must be 1 or 2"
input="$(cd "$(dirname "$input")" && pwd)/$(basename "$input")"
[[ -n "$out_dir" ]] || out_dir="$(dirname "$input")"
mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"
name="$(basename "$input")"
name="${name%.qcow2}"
[[ "$name" =~ ^[A-Za-z0-9._-]+$ ]] || die "the input name '$name' may hold only letters, digits, dot, underscore, and dash"

explicit_formats=1
if [[ -z "$formats" ]]; then
	formats="raw,vhdx,vhd,ova"
	explicit_formats=0
fi
want_raw=0 want_vhdx=0 want_vhd=0 want_ova=0
IFS=, read -r -a fmt_list <<<"$formats"
for f in "${fmt_list[@]}"; do
	case "$f" in
	raw) want_raw=1 ;;
	vhdx) want_vhdx=1 ;;
	vhd) want_vhd=1 ;;
	ova) want_ova=1 ;;
	*) die "unknown format '$f' (raw, vhdx, vhd, ova)" ;;
	esac
done

for cmd in qemu-img sha256sum tar stat df awk; do
	command -v "$cmd" >/dev/null 2>&1 || die "missing command: $cmd (qemu-img comes with the qemu-utils package)"
done

work="$out_dir/.convert.$$"
mkdir -p "$work"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

outputs=()

# have_space BYTES: true when the output directory has that much free space.
have_space() {
	local avail
	avail="$(df -Pk "$out_dir" | awk 'NR==2 {print $4}')"
	((avail * 1024 >= $1))
}

# need_space WHAT BYTES: returns 1 with a warning when it does not fit and was not asked for,
# and dies when it was asked for.
need_space() {
	have_space "$2" && return 0
	if ((explicit_formats == 1)); then
		die "not enough free space in $out_dir for $1 ($(($2 / mib)) MiB needed)"
	fi
	echo "WARN not enough free space in $out_dir for $1 ($(($2 / mib)) MiB needed), skipped"
	return 1
}

# check_img FORMAT FILE: qemu-img check, or qemu-img info when the format has no check.
check_img() {
	local fmt="$1" file="$2" rc=0
	if [[ "$fmt" == raw || "$fmt" == vpc ]]; then
		qemu-img info -f "$fmt" "$file" >/dev/null || die "qemu-img info failed for $file"
		echo "PASS qemu-img info $(basename "$file") ($fmt has no check)"
		return
	fi
	qemu-img check -f "$fmt" "$file" >/dev/null 2>"$work/check.err" || rc=$?
	if ((rc == 0)); then
		echo "PASS qemu-img check $(basename "$file") ($fmt)"
	elif ((rc == 63)); then
		# Exit code 63: the format does not support checks in this qemu-img.
		qemu-img info -f "$fmt" "$file" >/dev/null || die "qemu-img info failed for $file"
		echo "PASS qemu-img info $(basename "$file") ($fmt: this qemu-img has no check for it)"
	else
		cat "$work/check.err" >&2
		die "qemu-img check failed for $file ($fmt, exit $rc)"
	fi
}

step "Check the input"
check_img qcow2 "$input"
# The JSON lists the virtual size of the file node first and of the image last.
vsize="$(qemu-img info -f qcow2 --output=json "$input" | grep -o '"virtual-size": *[0-9]*' | tail -n 1 | grep -o '[0-9]*$')"
[[ -n "$vsize" && "$vsize" -gt 0 ]] || die "cannot read the virtual size of $input"
rsize=$(((vsize + mib - 1) / mib * mib))
echo "virtual size $vsize bytes, rounded up to $rsize bytes ($((rsize / mib)) MiB)"

raw="$out_dir/$name.raw"
raw_ready=0
# The VHD and the raw output need the raw disk. The OVA and the VHDX convert from the qcow2.
raw_needed=0
((want_raw == 1 || want_vhd == 1 || keep_raw == 1)) && raw_needed=1

if ((raw_needed == 1)); then
	step "Raw disk"
	if need_space "the raw disk" "$rsize"; then
		qemu-img convert -f qcow2 -O raw "$input" "$raw"
		have="$(stat -c %s "$raw")"
		if ((have != rsize)); then
			qemu-img resize -f raw "$raw" "$rsize" >/dev/null
		fi
		raw_ready=1
		echo "wrote $raw ($(stat -c %s "$raw") bytes)"
	fi
fi
if ((raw_ready == 1 && (want_raw == 1 || keep_raw == 1))); then
	check_img raw "$raw"
	outputs+=("$name.raw")
fi

if ((want_vhdx == 1)); then
	step "VHDX"
	qemu-img convert -f qcow2 -O vhdx "$input" "$out_dir/$name.vhdx"
	check_img vhdx "$out_dir/$name.vhdx"
	outputs+=("$name.vhdx")
fi

if ((want_vhd == 1)); then
	step "Fixed VHD, aligned to 1 MiB"
	if ((raw_ready == 0)); then
		echo "WARN no raw disk, so the VHD is skipped"
		((explicit_formats == 0)) || die "vhd was requested but the raw disk could not be made"
	elif need_space "the fixed VHD" "$((rsize + 512))"; then
		# force_size=on keeps the exact size. Without it qemu-img rounds the size to a disk
		# geometry (CHS) and the result misses the 1 MiB multiple that Azure demands.
		qemu-img convert -f raw -O vpc -o subformat=fixed,force_size=on "$raw" "$out_dir/$name.vhd"
		vhd_size="$(stat -c %s "$out_dir/$name.vhd")"
		body=$((vhd_size - 512))
		if ((body > 0 && body % mib == 0)); then
			echo "PASS vhd alignment: $vhd_size bytes - 512 byte footer = $body bytes = $((body / mib)) MiB, a multiple of 1 MiB"
		else
			die "vhd alignment: $vhd_size bytes - 512 byte footer = $body bytes, not a multiple of 1 MiB (Azure rejects it)"
		fi
		check_img vpc "$out_dir/$name.vhd"
		outputs+=("$name.vhd")
	fi
fi

if ((want_ova == 1)); then
	step "OVA (OVF 1.0, streamOptimized VMDK)"
	vmdk="$work/$name.vmdk"
	ovf="$work/$name.ovf"
	mf="$work/$name.mf"
	qemu-img convert -f qcow2 -O vmdk -o subformat=streamOptimized "$input" "$vmdk"
	check_img vmdk "$vmdk"
	vmdk_size="$(stat -c %s "$vmdk")"
	{
		cat <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1"
          xmlns:ovf="http://schemas.dmtf.org/ovf/envelope/1"
          xmlns:rasd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData"
          xmlns:vssd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_VirtualSystemSettingData"
          xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <References>
    <File ovf:id="file1" ovf:href="$name.vmdk" ovf:size="$vmdk_size"/>
  </References>
  <DiskSection>
    <Info>Virtual disk information</Info>
    <Disk ovf:diskId="vmdisk1" ovf:fileRef="file1" ovf:capacity="$vsize" ovf:capacityAllocationUnits="byte"
          ovf:format="http://www.vmware.com/interfaces/specifications/vmdk.html#streamOptimized"/>
  </DiskSection>
  <NetworkSection>
    <Info>Networks of the appliance</Info>
    <Network ovf:name="management">
      <Description>Management network. The first NIC becomes mgmt0 and takes an address by DHCP.</Description>
    </Network>
EOF
		if ((nics == 2)); then
			cat <<EOF
    <Network ovf:name="test">
      <Description>Test network. The second NIC is a test interface and gets no address from the image.</Description>
    </Network>
EOF
		fi
		cat <<EOF
  </NetworkSection>
  <VirtualSystem ovf:id="$name">
    <Info>FixingPointSystem appliance</Info>
    <Name>$name</Name>
    <OperatingSystemSection ovf:id="94" ovf:version="24.04">
      <Info>Ubuntu 24.04 LTS, 64 bit</Info>
      <Description>Ubuntu64Guest</Description>
    </OperatingSystemSection>
    <VirtualHardwareSection>
      <Info>Virtual hardware</Info>
      <System>
        <vssd:ElementName>Virtual Hardware Family</vssd:ElementName>
        <vssd:InstanceID>0</vssd:InstanceID>
        <vssd:VirtualSystemIdentifier>$name</vssd:VirtualSystemIdentifier>
        <vssd:VirtualSystemType>vmx-13</vssd:VirtualSystemType>
      </System>
      <Item>
        <rasd:AllocationUnits>hertz * 10^6</rasd:AllocationUnits>
        <rasd:Description>Number of virtual CPUs</rasd:Description>
        <rasd:ElementName>$cpus virtual CPU(s)</rasd:ElementName>
        <rasd:InstanceID>1</rasd:InstanceID>
        <rasd:ResourceType>3</rasd:ResourceType>
        <rasd:VirtualQuantity>$cpus</rasd:VirtualQuantity>
      </Item>
      <Item>
        <rasd:AllocationUnits>byte * 2^20</rasd:AllocationUnits>
        <rasd:Description>Memory size</rasd:Description>
        <rasd:ElementName>$memory_mb MB of memory</rasd:ElementName>
        <rasd:InstanceID>2</rasd:InstanceID>
        <rasd:ResourceType>4</rasd:ResourceType>
        <rasd:VirtualQuantity>$memory_mb</rasd:VirtualQuantity>
      </Item>
      <Item>
        <rasd:Address>0</rasd:Address>
        <rasd:Description>SCSI Controller</rasd:Description>
        <rasd:ElementName>SCSI Controller 0</rasd:ElementName>
        <rasd:InstanceID>3</rasd:InstanceID>
        <rasd:ResourceSubType>VirtualSCSI</rasd:ResourceSubType>
        <rasd:ResourceType>6</rasd:ResourceType>
      </Item>
      <Item>
        <rasd:AddressOnParent>0</rasd:AddressOnParent>
        <rasd:ElementName>Hard Disk 1</rasd:ElementName>
        <rasd:HostResource>ovf:/disk/vmdisk1</rasd:HostResource>
        <rasd:InstanceID>4</rasd:InstanceID>
        <rasd:Parent>3</rasd:Parent>
        <rasd:ResourceType>17</rasd:ResourceType>
      </Item>
      <Item>
        <rasd:AddressOnParent>7</rasd:AddressOnParent>
        <rasd:AutomaticAllocation>true</rasd:AutomaticAllocation>
        <rasd:Connection>management</rasd:Connection>
        <rasd:Description>Management NIC (mgmt0)</rasd:Description>
        <rasd:ElementName>Ethernet 1</rasd:ElementName>
        <rasd:InstanceID>5</rasd:InstanceID>
        <rasd:ResourceSubType>E1000</rasd:ResourceSubType>
        <rasd:ResourceType>10</rasd:ResourceType>
      </Item>
EOF
		if ((nics == 2)); then
			cat <<EOF
      <Item>
        <rasd:AddressOnParent>8</rasd:AddressOnParent>
        <rasd:AutomaticAllocation>true</rasd:AutomaticAllocation>
        <rasd:Connection>test</rasd:Connection>
        <rasd:Description>Test NIC</rasd:Description>
        <rasd:ElementName>Ethernet 2</rasd:ElementName>
        <rasd:InstanceID>6</rasd:InstanceID>
        <rasd:ResourceSubType>E1000</rasd:ResourceSubType>
        <rasd:ResourceType>10</rasd:ResourceType>
      </Item>
EOF
		fi
		cat <<EOF
    </VirtualHardwareSection>
  </VirtualSystem>
</Envelope>
EOF
	} >"$ovf"
	# The manifest lists each member in the order of the archive. The manifest itself is not listed.
	(cd "$work" && for m in "$name.ovf" "$name.vmdk"; do
		printf 'SHA256(%s)= %s\n' "$m" "$(sha256sum "$m" | cut -d' ' -f1)"
	done) >"$mf"
	ova="$out_dir/$name.ova"
	rm -f "$ova"
	# ustar keeps the members plain, and the order is the one that OVA readers expect.
	tar --format=ustar -C "$work" -cf "$ova" "$name.ovf" "$name.vmdk" "$name.mf"
	echo "members of $(basename "$ova"):"
	tar -tvf "$ova"
	order="$(tar -tf "$ova" | tr '\n' ' ')"
	[[ "$order" == "$name.ovf $name.vmdk $name.mf " ]] || die "unexpected OVA member order: $order"
	echo "PASS ova members in order: .ovf, .vmdk, .mf"
	outputs+=("$name.ova")
fi

if ((raw_ready == 1 && want_raw == 0 && keep_raw == 0)); then
	rm -f "$raw"
fi

step "Checksums"
((${#outputs[@]} > 0)) || die "nothing was written"
(cd "$out_dir" && sha256sum "${outputs[@]}" >SHA256SUMS)
cat "$out_dir/SHA256SUMS"
(cd "$out_dir" && sha256sum -c --quiet SHA256SUMS) || die "SHA256SUMS does not verify"

step "Summary"
for f in "${outputs[@]}"; do
	printf '%12d  %s\n' "$(stat -c %s "$out_dir/$f")" "$out_dir/$f"
done
echo "PASS converted $(basename "$input") to: ${outputs[*]}"
