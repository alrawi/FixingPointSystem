#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Two arm network namespace harness. A client namespace and a server namespace
# connect through a routing namespace that stands in for the device under test.
#
# Usage:
#   netns.sh up                      create the namespaces, links, and routes
#   netns.sh down                    remove everything that up created
#   netns.sh exec client|router|server CMD [ARG...]
#   netns.sh capture start FILE [client|server|all]
#   netns.sh capture stop
#   netns.sh info                    print the configuration as KEY=VALUE lines
#
# The environment configures up. Later commands need only FPS_NETNS and
# FPS_STATE_DIR, and info prints the configuration that up stored.
#   FPS_NETNS        name prefix of the namespaces, [a-z0-9]{1,12}, default fps
#   FPS_CLIENT_NET   client arm prefix, default 10.201.0.0/24
#   FPS_SERVER_NET   server arm prefix, default 10.202.0.0/24
#   FPS_MTU          MTU of every harness interface, default 1500
#   FPS_CLIENT_VLAN  802.1Q id on the client arm, empty for untagged
#   FPS_SERVER_VLAN  802.1Q id on the server arm, empty for untagged
#   FPS_NAT          1 masquerades the client prefix behind the router's server
#                    arm address, default 0
#   FPS_OFFLOAD      1 keeps veth segmentation offloads, default 0 turns them
#                    off so captures show wire sized frames
#   FPS_STATE_DIR    state directory, default /run/fps-netns
#
# The router takes the first host address of each prefix and the client and
# server take the second. Every link and route lives inside the three
# namespaces, so deleting the namespaces removes all of them.
set -euo pipefail

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly HARNESS_DIR

usage() {
  cat >&2 <<'EOF'
usage: netns.sh up | down | info
       netns.sh exec client|router|server CMD [ARG...]
       netns.sh capture start FILE [client|server|all]
       netns.sh capture stop
EOF
  exit 2
}

die() {
  echo "netns.sh: $*" >&2
  exit 1
}

log() {
  echo "netns.sh: $*" >&2
}

require_root() {
  [[ "${EUID}" -eq 0 ]] || die "this command needs root, run it as root or with sudo"
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command $1"
}

# load_config reads the environment and derives every name and address.
load_config() {
  PREFIX="${FPS_NETNS:-fps}"
  [[ "${PREFIX}" =~ ^[a-z0-9]{1,12}$ ]] || die "FPS_NETNS must match [a-z0-9]{1,12}"
  STATE_DIR="${FPS_STATE_DIR:-/run/fps-netns}"
  STATE_FILE="${STATE_DIR}/${PREFIX}.env"
  CAPTURE_PID="${STATE_DIR}/${PREFIX}.capture.pid"
  CAPTURE_META="${STATE_DIR}/${PREFIX}.capture.env"
  CAPTURE_LOG="${STATE_DIR}/${PREFIX}.capture.log"
  NS_CLIENT="${PREFIX}-client"
  NS_ROUTER="${PREFIX}-router"
  NS_SERVER="${PREFIX}-server"
}

# load_state confirms that up ran. Every name that later commands need derives
# from the prefix, so they never source the state file.
load_state() {
  [[ -f "${STATE_FILE}" ]] || die "harness ${PREFIX} is not up, run netns.sh up first"
}

# meta_value FILE KEY prints one value of a KEY=VALUE file.
meta_value() {
  sed -n "s/^$2=//p" "$1"
}

ip_to_int() {
  local a b c d
  IFS=. read -r a b c d <<<"$1"
  echo $(((a << 24) | (b << 16) | (c << 8) | d))
}

int_to_ip() {
  local n="$1"
  echo "$(((n >> 24) & 255)).$(((n >> 16) & 255)).$(((n >> 8) & 255)).$((n & 255))"
}

# split_prefix NET NAME sets P_NET, P_GW, P_HOST, and P_LEN from a CIDR prefix.
split_prefix() {
  local net="$1" var="$2" addr len octet base mask
  [[ "${net}" =~ ^([0-9]{1,3}(\.[0-9]{1,3}){3})/([0-9]{1,2})$ ]] ||
    die "${var} prefix ${net} is not an IPv4 CIDR prefix"
  addr="${BASH_REMATCH[1]}"
  len="${BASH_REMATCH[3]}"
  for octet in ${addr//./ }; do
    ((octet <= 255)) || die "${var} prefix ${net} has an octet above 255"
  done
  ((len >= 8 && len <= 30)) || die "${var} prefix ${net} needs a length from 8 to 30"
  mask=$(((0xffffffff << (32 - len)) & 0xffffffff))
  base=$(($(ip_to_int "${addr}") & mask))
  P_GW="$(int_to_ip $((base + 1)))"
  P_HOST="$(int_to_ip $((base + 2)))"
  P_NET="$(int_to_ip "${base}")/${len}"
  P_LEN="${len}"
}

check_vlan() {
  local name="$1" id="$2"
  [[ -z "${id}" ]] && return 0
  if ! [[ "${id}" =~ ^[0-9]+$ ]] || ((id < 1 || id > 4094)); then
    die "${name} must be an 802.1Q id from 1 to 4094"
  fi
}

# resolve_up_config validates the environment and fills the variables that up
# stores in the state file.
resolve_up_config() {
  split_prefix "${FPS_CLIENT_NET:-10.201.0.0/24}" FPS_CLIENT_NET
  CLIENT_NET="${P_NET}" CLIENT_GW="${P_GW}" CLIENT_HOST="${P_HOST}" CLIENT_LEN="${P_LEN}"
  split_prefix "${FPS_SERVER_NET:-10.202.0.0/24}" FPS_SERVER_NET
  SERVER_NET="${P_NET}" SERVER_GW="${P_GW}" SERVER_HOST="${P_HOST}" SERVER_LEN="${P_LEN}"
  local c s
  c=$(ip_to_int "${CLIENT_NET%/*}")
  s=$(ip_to_int "${SERVER_NET%/*}")
  local shorter=$((CLIENT_LEN < SERVER_LEN ? CLIENT_LEN : SERVER_LEN))
  local mask=$(((0xffffffff << (32 - shorter)) & 0xffffffff))
  (((c & mask) != (s & mask))) || die "the client and server prefixes overlap"
  MTU="${FPS_MTU:-1500}"
  if ! [[ "${MTU}" =~ ^[0-9]+$ ]] || ((MTU < 576 || MTU > 9000)); then
    die "FPS_MTU must be an integer from 576 to 9000"
  fi
  CLIENT_VLAN="${FPS_CLIENT_VLAN:-}"
  SERVER_VLAN="${FPS_SERVER_VLAN:-}"
  check_vlan FPS_CLIENT_VLAN "${CLIENT_VLAN}"
  check_vlan FPS_SERVER_VLAN "${SERVER_VLAN}"
  NAT="${FPS_NAT:-0}"
  [[ "${NAT}" == 0 || "${NAT}" == 1 ]] || die "FPS_NAT must be 0 or 1"
  OFFLOAD="${FPS_OFFLOAD:-0}"
  [[ "${OFFLOAD}" == 0 || "${OFFLOAD}" == 1 ]] || die "FPS_OFFLOAD must be 0 or 1"
}

write_state() {
  mkdir -p "${STATE_DIR}"
  cat >"${STATE_FILE}" <<EOF
FPS_NETNS=${PREFIX}
NS_CLIENT=${NS_CLIENT}
NS_ROUTER=${NS_ROUTER}
NS_SERVER=${NS_SERVER}
CLIENT_NET=${CLIENT_NET}
CLIENT_GW=${CLIENT_GW}
CLIENT_ADDR=${CLIENT_HOST}
SERVER_NET=${SERVER_NET}
SERVER_GW=${SERVER_GW}
SERVER_ADDR=${SERVER_HOST}
MTU=${MTU}
CLIENT_VLAN=${CLIENT_VLAN}
SERVER_VLAN=${SERVER_VLAN}
NAT=${NAT}
OFFLOAD=${OFFLOAD}
ROUTER_CLIENT_IF=cli0
ROUTER_SERVER_IF=srv0
EOF
}

nsx() {
  local ns="$1"
  shift
  ip netns exec "${ns}" "$@"
}

ns_sysctl() {
  local ns="$1" key="$2" value="$3"
  echo "${value}" | nsx "${ns}" tee "/proc/sys/${key//.//}" >/dev/null
}

# setup_ns creates one namespace with loopback up and IPv6 off, so captures
# carry only the traffic that tests send.
setup_ns() {
  local ns="$1"
  ip netns add "${ns}"
  ip -n "${ns}" link set lo up
  # A kernel without IPv6 has no such keys and needs no change.
  if [[ -e /proc/sys/net/ipv6/conf/all/disable_ipv6 ]]; then
    ns_sysctl "${ns}" net.ipv6.conf.all.disable_ipv6 1
    ns_sysctl "${ns}" net.ipv6.conf.default.disable_ipv6 1
  fi
}

disable_offloads() {
  local ns="$1" dev="$2"
  [[ "${OFFLOAD}" == 1 ]] && return 0
  if command -v ethtool >/dev/null 2>&1; then
    nsx "${ns}" ethtool -K "${dev}" tso off gso off gro off >/dev/null 2>&1 ||
      log "could not turn off offloads on ${dev} in ${ns}"
  else
    log "ethtool is missing, so captures may show frames above the MTU"
  fi
}

# setup_arm links END_NS (interface eth0) with the router (interface RIF),
# applies the MTU and the optional VLAN, and assigns the addresses.
setup_arm() {
  local end_ns="$1" rif="$2" vlan="$3" gw="$4" host="$5" len="$6"
  local end_dev=eth0 r_dev="${rif}"
  ip -n "${end_ns}" link add eth0 type veth peer name "${rif}" netns "${NS_ROUTER}"
  ip -n "${end_ns}" link set eth0 mtu "${MTU}"
  ip -n "${NS_ROUTER}" link set "${rif}" mtu "${MTU}"
  disable_offloads "${end_ns}" eth0
  disable_offloads "${NS_ROUTER}" "${rif}"
  ip -n "${end_ns}" link set eth0 up
  ip -n "${NS_ROUTER}" link set "${rif}" up
  if [[ -n "${vlan}" ]]; then
    end_dev="eth0.${vlan}"
    r_dev="${rif}.${vlan}"
    ip -n "${end_ns}" link add link eth0 name "${end_dev}" type vlan id "${vlan}" ||
      die "cannot create VLAN ${vlan}, the kernel may lack the 8021q module"
    ip -n "${NS_ROUTER}" link add link "${rif}" name "${r_dev}" type vlan id "${vlan}"
    ip -n "${end_ns}" link set "${end_dev}" mtu "${MTU}" up
    ip -n "${NS_ROUTER}" link set "${r_dev}" mtu "${MTU}" up
  fi
  ip -n "${NS_ROUTER}" addr add "${gw}/${len}" dev "${r_dev}"
  ip -n "${end_ns}" addr add "${host}/${len}" dev "${end_dev}"
  ip -n "${end_ns}" route add default via "${gw}" dev "${end_dev}"
}

setup_nat() {
  [[ "${NAT}" == 1 ]] || return 0
  local out_dev=srv0
  [[ -n "${SERVER_VLAN}" ]] && out_dev="srv0.${SERVER_VLAN}"
  if command -v nft >/dev/null 2>&1; then
    nsx "${NS_ROUTER}" nft -f - <<EOF
table ip fps_nat {
  chain postrouting {
    type nat hook postrouting priority srcnat; policy accept;
    ip saddr ${CLIENT_NET} oifname "${out_dev}" masquerade
  }
}
EOF
  elif command -v iptables >/dev/null 2>&1; then
    nsx "${NS_ROUTER}" iptables -t nat -A POSTROUTING -s "${CLIENT_NET}" -o "${out_dev}" -j MASQUERADE
  else
    die "FPS_NAT=1 needs nft or iptables"
  fi
}

cmd_up() {
  require_root
  require_cmd ip
  resolve_up_config
  local ns
  for ns in "${NS_CLIENT}" "${NS_ROUTER}" "${NS_SERVER}"; do
    if ip netns list | awk '{print $1}' | grep -qxF "${ns}"; then
      die "namespace ${ns} exists, run netns.sh down first"
    fi
  done
  # A failed up removes what it created, so a retry starts clean. The trap
  # also covers die, which exits without returning.
  UP_DONE=0
  trap 'if [[ "${UP_DONE}" != 1 ]]; then log "up failed, removing the partial harness"; teardown; fi' EXIT
  write_state
  for ns in "${NS_CLIENT}" "${NS_ROUTER}" "${NS_SERVER}"; do
    setup_ns "${ns}"
  done
  ns_sysctl "${NS_ROUTER}" net.ipv4.ip_forward 1
  setup_arm "${NS_CLIENT}" cli0 "${CLIENT_VLAN}" "${CLIENT_GW}" "${CLIENT_HOST}" "${CLIENT_LEN}"
  setup_arm "${NS_SERVER}" srv0 "${SERVER_VLAN}" "${SERVER_GW}" "${SERVER_HOST}" "${SERVER_LEN}"
  setup_nat
  UP_DONE=1
  trap - EXIT
  log "harness ${PREFIX} is up, client ${CLIENT_HOST} and server ${SERVER_HOST}"
}

stop_capture() {
  [[ -f "${CAPTURE_PID}" ]] || return 0
  local pid
  pid="$(cat "${CAPTURE_PID}")"
  if kill -0 "${pid}" 2>/dev/null; then
    kill -INT "${pid}" 2>/dev/null || true
    local i
    for ((i = 0; i < 50; i++)); do
      kill -0 "${pid}" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "${pid}" 2>/dev/null || true
  fi
  rm -f "${CAPTURE_PID}"
}

# teardown removes the namespaces, which removes every link and route inside
# them, and then the state files. It tolerates a partial harness.
teardown() {
  stop_capture
  local ns
  for ns in "${NS_CLIENT}" "${NS_ROUTER}" "${NS_SERVER}"; do
    if ip netns list | awk '{print $1}' | grep -qxF "${ns}"; then
      ip netns pids "${ns}" 2>/dev/null | xargs -r kill -KILL 2>/dev/null || true
      ip netns del "${ns}"
    fi
  done
  rm -f "${STATE_FILE}" "${CAPTURE_META}" "${CAPTURE_LOG}"
  if [[ -d "${STATE_DIR}" ]]; then
    rmdir --ignore-fail-on-non-empty "${STATE_DIR}"
  fi
}

cmd_down() {
  require_root
  require_cmd ip
  teardown
  log "harness ${PREFIX} is down"
}

cmd_exec() {
  [[ $# -ge 2 ]] || usage
  require_root
  load_state
  local ns
  case "$1" in
    client) ns="${NS_CLIENT}" ;;
    router) ns="${NS_ROUTER}" ;;
    server) ns="${NS_SERVER}" ;;
    *) usage ;;
  esac
  shift
  exec ip netns exec "${ns}" "$@"
}

cmd_info() {
  load_state
  cat "${STATE_FILE}"
}

# capture_start records on the router's client arm, server arm, or both with
# tcpdump in immediate mode, so stop loses no packet that already crossed the
# arm. tcpdump writes pcap, and capture_stop converts the file to pcapng. The
# client and server arms record Ethernet frames with their 802.1Q tags, and
# all records the Linux cooked v2 link type of the any interface.
capture_start() {
  [[ $# -ge 1 && $# -le 2 ]] || usage
  require_root
  require_cmd tcpdump
  require_cmd python3
  load_state
  local file="$1" arm="${2:-client}" dev
  case "${arm}" in
    client) dev=cli0 ;;
    server) dev=srv0 ;;
    all) dev=any ;;
    *) usage ;;
  esac
  [[ ! -f "${CAPTURE_PID}" ]] || die "a capture is running, run netns.sh capture stop first"
  mkdir -p "$(dirname "${file}")"
  file="$(cd "$(dirname "${file}")" && pwd)/$(basename "${file}")"
  # The AppArmor profile of tcpdump on Ubuntu permits writes only to files
  # with a capture extension, so the temporary file ends in .pcap.
  local raw="${file}.part.pcap"
  : >"${CAPTURE_LOG}"
  setsid ip netns exec "${NS_ROUTER}" \
    tcpdump -n -s 0 -U --immediate-mode -i "${dev}" -w "${raw}" \
    >"${CAPTURE_LOG}" 2>&1 </dev/null &
  echo "$!" >"${CAPTURE_PID}"
  cat >"${CAPTURE_META}" <<EOF
CAPTURE_RAW=${raw}
CAPTURE_FILE=${file}
EOF
  # tcpdump reports on stderr once the interface is open.
  local i
  for ((i = 0; i < 100; i++)); do
    if grep -q 'listening on' "${CAPTURE_LOG}"; then
      log "capture of the ${arm} arm writes ${file}"
      return 0
    fi
    kill -0 "$(cat "${CAPTURE_PID}")" 2>/dev/null || break
    sleep 0.1
  done
  cat "${CAPTURE_LOG}" >&2
  stop_capture
  die "the capture did not start"
}

capture_stop() {
  require_root
  load_state
  [[ -f "${CAPTURE_PID}" ]] || die "no capture is running"
  local raw file
  raw="$(meta_value "${CAPTURE_META}" CAPTURE_RAW)"
  file="$(meta_value "${CAPTURE_META}" CAPTURE_FILE)"
  stop_capture
  python3 -I "${HARNESS_DIR}/pcapng.py" convert "${raw}" "${file}"
  rm -f "${raw}" "${CAPTURE_META}"
  log "capture written to ${file}"
}

cmd_capture() {
  [[ $# -ge 1 ]] || usage
  local sub="$1"
  shift
  case "${sub}" in
    start) capture_start "$@" ;;
    stop) [[ $# -eq 0 ]] || usage; capture_stop ;;
    *) usage ;;
  esac
}

main() {
  [[ $# -ge 1 ]] || usage
  load_config
  local cmd="$1"
  shift
  case "${cmd}" in
    up) [[ $# -eq 0 ]] || usage; cmd_up ;;
    down) [[ $# -eq 0 ]] || usage; cmd_down ;;
    exec) cmd_exec "$@" ;;
    capture) cmd_capture "$@" ;;
    info) [[ $# -eq 0 ]] || usage; cmd_info ;;
    *) usage ;;
  esac
}

main "$@"
