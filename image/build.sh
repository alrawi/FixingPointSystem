#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the FixingPointSystem appliance image.
#
#   image/build.sh
#
# The script builds the controller (Go, CGO off) and the agent (cargo, release profile,
# x86_64-unknown-linux-musl), both static,
# stages the modules, and runs `packer init` and `packer build` on image/fps.pkr.hcl.
# The result is image/output/fps-base.qcow2. Run it from any directory. It needs
# the tools of image/README.md, Prerequisites, and it downloads a pinned Go toolchain into
# image/.cache when `go` is missing or older than controller/go.mod asks for.
#
# Settings through the environment:
#   FPS_GO_VERSION     Go toolchain to download when needed       (default 1.24.7)
#   FPS_ACCEL          QEMU accelerator for the build: kvm or tcg  (default kvm)
#   FPS_ALLOW_TCG=1    let the build fall back to tcg without /dev/kvm (very slow)
#   FPS_PACKER_LOG=1   print the Packer debug log
#   FPS_BASE_IMAGE_URL, FPS_BASE_IMAGE_CHECKSUM   pin another Ubuntu cloud image
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
img="$root/image"
cache="$img/.cache"
build="$img/build"
out="$img/output"
go_version="${FPS_GO_VERSION:-1.24.7}"

step() { printf '\n== %s\n' "$*"; }
die() {
	echo "FAIL $*" >&2
	exit 1
}
need() {
	command -v "$1" >/dev/null 2>&1 || die "missing command: $1 (see image/README.md, Prerequisites)"
}

step "Check the prerequisites"
for cmd in packer qemu-system-x86_64 qemu-img cargo file curl tar python3 openssl sha256sum; do
	need "$cmd"
done
if ! command -v xorriso >/dev/null && ! command -v mkisofs >/dev/null && ! command -v genisoimage >/dev/null; then
	die "Packer needs xorriso, mkisofs, or genisoimage to make the cloud-init CD (sudo apt-get install xorriso)"
fi
[[ "$(uname -m)" == x86_64 ]] || die "this build supports x86_64 hosts only"
accel="${FPS_ACCEL:-kvm}"
if [[ "$accel" == kvm && ! -w /dev/kvm ]]; then
	if [[ "${FPS_ALLOW_TCG:-0}" == 1 ]]; then
		echo "NOTE /dev/kvm is not writable, so the build uses tcg and takes a long time"
		accel=tcg
	else
		die "/dev/kvm is not writable by this user. Add yourself to the kvm group, or set FPS_ALLOW_TCG=1."
	fi
fi
# Both binaries are static, so the build host's C library does not matter. The agent
# targets musl, which needs the rust-std component for that target and, when a
# dependency compiles C code, musl-gcc.
musl_target=x86_64-unknown-linux-musl
command -v rustup >/dev/null 2>&1 ||
	die "rustup is missing. Distro cargo cannot add the musl target. Install rustup (image/README.md, Prerequisites)."
if ! rustup target list --installed 2>/dev/null | grep -qx "$musl_target"; then
	die "the Rust target $musl_target is not installed. Run: rustup target add $musl_target"
fi
command -v musl-gcc >/dev/null 2>&1 ||
	die "musl-gcc is missing, which a dependency may need to compile C code. Run: sudo apt install musl-tools"

step "Find or install Go"
need_go="$(awk '/^go /{print $2; exit}' "$root/controller/go.mod")"
go_ok() {
	local have
	have="$("$1" env GOVERSION 2>/dev/null | sed 's/^go//')" || return 1
	[[ -n "$have" && "$(printf '%s\n%s\n' "$need_go" "$have" | sort -V | head -n 1)" == "$need_go" ]]
}
go_bin=""
if command -v go >/dev/null && GOTOOLCHAIN=local go_ok go; then
	go_bin="$(command -v go)"
elif [[ -x "$cache/go/bin/go" ]] && GOTOOLCHAIN=local go_ok "$cache/go/bin/go"; then
	go_bin="$cache/go/bin/go"
else
	tarball="go${go_version}.linux-amd64.tar.gz"
	echo "Downloading $tarball into $cache"
	mkdir -p "$cache/dl"
	curl -fsSL -o "$cache/dl/$tarball" "https://go.dev/dl/$tarball"
	# go.dev publishes the checksum of every release in its download index.
	want="$(curl -fsSL 'https://go.dev/dl/?mode=json&include=all' |
		python3 -I -c 'import json,sys
name=sys.argv[1]
for rel in json.load(sys.stdin):
    for f in rel["files"]:
        if f["filename"]==name:
            print(f["sha256"]); sys.exit(0)
sys.exit(1)' "$tarball")" || die "go.dev lists no checksum for $tarball"
	have="$(sha256sum "$cache/dl/$tarball" | cut -d' ' -f1)"
	[[ "$want" == "$have" ]] || die "checksum mismatch for $tarball (wanted $want, got $have)"
	rm -rf "$cache/go"
	tar -C "$cache" -xzf "$cache/dl/$tarball"
	go_bin="$cache/go/bin/go"
fi
echo "Go: $go_bin ($("$go_bin" env GOVERSION))"

step "Build the controller"
rm -rf "$build"
mkdir -p "$build/modules"
(
	cd "$root/controller"
	GOWORK=off GOTOOLCHAIN=local CGO_ENABLED=0 GOOS=linux GOARCH=amd64 \
		"$go_bin" build -trimpath -ldflags '-s -w' -o "$build/fps-controller" ./cmd/fps-controller
)
file "$build/fps-controller" | grep -q 'statically linked' ||
	die "the controller is not statically linked: $(file "$build/fps-controller")"
"$build/fps-controller" -version

step "Build the agent"
agent_target_dir="${CARGO_TARGET_DIR:-$cache/cargo-target}"
(
	cd "$root"
	CARGO_TARGET_DIR="$agent_target_dir" cargo build --release --locked --target "$musl_target" -p fps-agent
	cp "$agent_target_dir/$musl_target/release/fps-agent" "$build/fps-agent"
)
strip "$build/fps-agent" 2>/dev/null || true
file "$build/fps-agent" | grep -Eq 'statically linked|static-pie linked' ||
	die "the agent is not statically linked: $(file "$build/fps-agent")"
"$build/fps-agent" capabilities >/dev/null || echo "NOTE fps-agent capabilities failed on this host. The image check still runs it in the VM."

step "Stage the modules"
for m in http1 dns; do
	cp -R "$root/modules/$m" "$build/modules/$m"
done

step "Run Packer"
export PACKER_CACHE_DIR="$cache/packer"
export PACKER_PLUGIN_PATH="$cache/packer-plugins"
mkdir -p "$PACKER_CACHE_DIR" "$PACKER_PLUGIN_PATH"
if [[ "${FPS_PACKER_LOG:-0}" == 1 ]]; then
	export PACKER_LOG=1
fi
ssh_password="$(openssl rand -hex 16)"
vars=(
	-var "controller_binary=$build/fps-controller"
	-var "agent_binary=$build/fps-agent"
	-var "modules_dir=$build/modules"
	-var "output_directory=$out"
	-var "ssh_password=$ssh_password"
	-var "accelerator=$accel"
)
[[ -n "${FPS_BASE_IMAGE_URL:-}" ]] && vars+=(-var "base_image_url=$FPS_BASE_IMAGE_URL")
[[ -n "${FPS_BASE_IMAGE_CHECKSUM:-}" ]] && vars+=(-var "base_image_checksum=$FPS_BASE_IMAGE_CHECKSUM")

packer init "$img/fps.pkr.hcl"
# Packer creates the output directory itself, and packer validate (which has no
# -force) rejects one that already exists, for example from an earlier build.
rm -rf "$out"
packer validate "${vars[@]}" "$img/fps.pkr.hcl"
packer build -force "${vars[@]}" "$img/fps.pkr.hcl"

step "Check the result"
qcow="$out/fps-base.qcow2"
[[ -f "$qcow" ]] || die "Packer did not write $qcow"
qemu-img check "$qcow"
qemu-img info "$qcow"
(cd "$out" && sha256sum fps-base.qcow2 | tee fps-base.qcow2.sha256)
echo
echo "PASS image built: $qcow"
