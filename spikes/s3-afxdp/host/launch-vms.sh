#!/usr/bin/env bash
# Spike S3 host side: boot two Ubuntu LTS guests joined by a private bridge.
# Guest "tx" generates packets with AF_XDP and guest "rx" receives them.
# Requires root, KVM, qemu-system-x86_64, qemu-img, cloud-image-utils.
set -euo pipefail

WORK=${WORK:-/var/tmp/fps-s3}
IMAGE_URL=${IMAGE_URL:-https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img}
VCPUS=${VCPUS:-16}
MEM_MB=${MEM_MB:-8192}
QUEUES=${QUEUES:-8}
BRIDGE=${BRIDGE:-fpsbr0}
SSH_KEY=${SSH_KEY:-$HOME/.ssh/id_ed25519.pub}
HERE=$(cd "$(dirname "$0")" && pwd)

for cmd in qemu-system-x86_64 qemu-img cloud-localds ip curl; do
  command -v "$cmd" >/dev/null || { echo "missing $cmd" >&2; exit 1; }
done
[[ -c /dev/kvm ]] || { echo "KVM is not available on this host" >&2; exit 1; }
[[ -r $SSH_KEY ]] || { echo "set SSH_KEY to a readable public key" >&2; exit 1; }

mkdir -p "$WORK"
base="$WORK/base.img"
[[ -f $base ]] || curl -fL -o "$base" "$IMAGE_URL"

if ! ip link show "$BRIDGE" >/dev/null 2>&1; then
  ip link add "$BRIDGE" type bridge
  ip link set "$BRIDGE" up
fi

key=$(cat "$SSH_KEY")
i=0
for role in tx rx; do
  i=$((i + 1))
  disk="$WORK/$role.qcow2"
  qemu-img create -f qcow2 -F qcow2 -b "$base" "$disk" 20G >/dev/null
  sed -e "s|@SSH_KEY@|$key|" -e "s|@ROLE@|$role|" -e "s|@ADDR@|10.77.0.$i|" -e "s|@MAC@|52:54:00:77:00:0$i|" \
    "$HERE/../guest/user-data.yaml" > "$WORK/$role-user-data.yaml"
  cloud-localds "$WORK/$role-seed.img" "$WORK/$role-user-data.yaml"

  tap="fps-$role"
  ip tuntap add dev "$tap" mode tap multi_queue 2>/dev/null || true
  ip link set "$tap" master "$BRIDGE" up

  # Management through user mode networking: ssh -p 222$i ubuntu@127.0.0.1
  qemu-system-x86_64 -name "fps-$role" -enable-kvm -cpu host -smp "$VCPUS" -m "$MEM_MB" \
    -drive file="$disk",if=virtio -drive file="$WORK/$role-seed.img",if=virtio,format=raw \
    -netdev user,id=mgmt,hostfwd=tcp:127.0.0.1:222$i-:22 -device virtio-net-pci,netdev=mgmt \
    -netdev tap,id=test,ifname="$tap",script=no,downscript=no,vhost=on,queues="$QUEUES" \
    -device virtio-net-pci,netdev=test,mq=on,vectors=$((2 * QUEUES + 2)),mac=52:54:00:77:00:0$i \
    -display none -daemonize -pidfile "$WORK/$role.pid"
  echo "started $role: ssh -p 222$i ubuntu@127.0.0.1"
done
echo "When cloud-init finishes, run guest/run-afxdp.sh in both guests (rx first)."
