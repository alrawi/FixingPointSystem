# Hosts

This page describes the host pool allocator of the HST workstream. The crate [`fps-hosts`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-hosts) maps host indexes to addresses, slices pools by agent and by worker without overlap, and picks the host of every session from the run seed. HST-01 created the crate, HST-02 adds the kernel port setup, and HST-03 adds the MAC allocator and the ARP and neighbor discovery responders for user space backends.

## Host indexes and addresses

A host pool is a numbered set of emulated hosts. The pool comes from the `host_pool` definition of the test configuration schema, and an agent receives its part as a `HostPoolSlice` message of the agent contract. Host index `i` maps to three addresses, which follow the comment on `HostPoolSlice`.

| Family | Address of host `i` |
|---|---|
| IPv4 | The `i`-th address after the network address, skipping the gateway and never reaching the broadcast address |
| IPv6 | The `i`-th address after the network address, skipping the gateway; the last address of the prefix is usable |
| MAC | `mac_base + i`, which must stay within 48 bits |

`HostPool::parse` takes a `PoolSpec` with the text fields of the contract and refuses a pool whose prefix holds fewer usable addresses than `count`, so every later mapping succeeds. A gateway costs an address only when it lies strictly inside the usable part of the prefix. A prefix whose host bits are not zero is an error, because the contract names the network address. A dual stack pool checks both prefixes. The mapping is injective, so disjoint host indexes give disjoint IPv4, IPv6, and MAC addresses.

## Slicing by agent and by worker

The controller splits each pool into one slice per agent, and each agent splits its slice into one range per worker. Both steps call `HostRange::split` with one weight per part, such as the core count. The function follows four rules.

1. The parts are contiguous, disjoint, cover the input range exactly, and appear in the order of the weights.
2. Every part with a positive weight gets one host first while hosts remain, so no such part stays empty when the range holds enough hosts.
3. The remaining hosts go out by the largest remainder method, and ties go to the lower part, so each part is within one host of its proportional share.
4. A part with weight zero gets no host, and a list without a positive weight is an error.

`check_partition` checks that a list of ranges covers a slice without gaps or overlap. An agent runs it on the worker ranges of each pool in its plan before it starts.

## Host selection

`HostSelector` picks the host of a session within one worker's range. It keeps no state and allocates nothing per call, so it fits the hot path. We support three modes.

| Mode | Host of session `s` |
|---|---|
| `RoundRobin` | `offset + s mod count`, so consecutive session ids visit every host in turn |
| `Uniform` | A uniform draw from the session generator |
| `Zipf { exponent }` | Rank `k`, counting from 1 at the start of the range, with probability proportional to `1 / k^exponent`, sampled by rejection inversion in constant memory |

The session generator is a `Pcg64Mcg` whose state derives from the run seed, the pool name, the worker id, and the session id through SplitMix64 and FNV-1a, with a domain constant that keeps it apart from other streams of the same session. We put the mixed session id in the high half of the state and a mixed value in the low half, because `Pcg64Mcg::new` sets the two lowest state bits and a constant low half biases the first outputs. A pinned test fixes the first uniform picks for seed 7, so a change to the derivation fails on purpose.

Round robin and uniform picks use integer arithmetic and repeat on every platform. Zipf picks use `ln` and `exp` from the platform math library, so a replay on another platform may differ in rare picks.

## Tests

We run the crate tests with `cargo test -p fps-hosts`, and `make test-eng` includes them through the engine crate glob. The tests cover three levels.

| Level | Tests |
|---|---|
| Unit | Address mapping with and without gateway for IPv4, IPv6, and dual stack pools, capacity errors, MAC arithmetic, split rules, and the distribution of each selection mode |
| Property | Slices by agent and then by worker are disjoint and cover the pool, every split stays within one host of its share, and every pick lies in its range and repeats for the same inputs |
| Contract | Every pool of `contracts/examples/tests/` parses and maps unique addresses, and the worker ranges of every example plan partition their agent slice |

## Kernel port setup

HST-02 adds kernel port setup to the crate in the modules `port`, `sysctl`, and `ipcmd`. A kernel test port is one test interface that carries the hosts of one pool slice. `port::setup` moves the interface into a network namespace of its own, named `fps-port-<name>`, and `KernelPort::teardown` returns the host to its earlier state. The kernel backend opens its sockets inside that namespace.

### Setup steps

`PortSpec::new` takes a host pool and the slice of the port and derives the local prefixes and the gateway. The caller then sets the optional VLAN, MTU, interface address, sysctl profile, and extra management names. Setup runs eight steps in order.

1. Refuse the interface when it is the loopback, a named management interface, enslaved to a master, the device of a default route, or the owner of an address with a scope wider than the link. Nothing changes before this check passes.
2. Create the namespace and bring its loopback up.
3. Apply the sysctl profile inside the namespace and read every value back.
4. Move the interface into the namespace, set its MTU, and bring it up.
5. Create the optional 802.1Q subinterface `vlan<id>` on top of the interface.
6. Assign the optional interface address. A device under test that routes the pool prefix to this address needs one neighbor entry for the whole pool.
7. Add `local` routes for the addresses of the slice, so a socket binds any pool address without one interface address per host. This is the AnyIP technique.
8. Add the default route through the gateway with the `onlink` flag.

The local routes cover exactly the addresses of the slice, through the smallest list of aligned prefixes, so two agents that share a link never answer for each other's hosts. The routes leave out the gateway when it lies inside the slice. We first tried one `local` route for the whole prefix and a `throw` route for the gateway in the local table, but the kernel merges the local and main tables while a namespace has no policy rules, and the throw route then makes the gateway unreachable.

Every command after step 2 runs inside the port namespace. The only commands that change the origin namespace move the test interface out and back and restore its MTU and state, and a component test asserts that every recorded command fits one of these forms. A failed setup runs teardown before it returns the first error. Teardown deletes the VLAN subinterface, moves the interface back, restores its MTU and state, and deletes the namespace, which removes every route that setup added. When the interface cannot move back, teardown keeps the namespace, because deleting it would destroy a virtual interface. `port::cleanup` removes the remains of a crashed run from the specification alone.

### Sysctl profile

`SysctlProfile` holds the keys below, and every key is per namespace, so the profile never changes the host. Setup skips an optional key when the namespace lacks it or the kernel keeps the old value, and `KernelPort::skipped_sysctls` lists the skipped keys.

| Key | Default | Need |
|---|---|---|
| `net.ipv4.ip_local_port_range` | `1024 65535` | Required |
| `net.ipv4.tcp_tw_reuse` | `1` | Required |
| `net.ipv4.tcp_rmem` | `4096 131072 16777216` | Required |
| `net.ipv4.tcp_wmem` | `4096 16384 16777216` | Required |
| `net.core.rmem_max`, `net.core.wmem_max` | `16777216` | Optional, because older kernels keep them global and newer kernels refuse to raise them above the host value in a child namespace |
| `net.ipv6.conf.{all,default}.disable_ipv6` | `1` without an IPv6 pool | Optional, for kernels without IPv6 |
| `net.ipv6.conf.{all,default}.accept_ra` | `0` with an IPv6 pool | Required |

### Why we drive ip(8)

We drive iproute2 through the narrow trait `IpRunner` instead of the `rtnetlink` crate. The `ip` program speaks rtnetlink to the kernel, so every step is still a netlink request. We chose it for three reasons.

1. Creating a named namespace and writing sysctls inside it needs `unshare`, `setns`, and a bind mount. Those calls need unsafe code or the `nix` crate and a dedicated thread, while the crate forbids unsafe code, and `rtnetlink` creates namespaces by forking, which is unsound in a multithreaded runtime.
2. `rtnetlink` would bring tokio, futures, and the netlink packet crates into a crate whose other code runs on the hot path without them.
3. iproute2 is present on every target host, its JSON output is stable, and an operator can repeat every logged command by hand.

The wrapper runs `ip` with an argument vector and no shell, and `PortSpec::validate` rejects names that `ip` could read as an option. `FakeIp` records commands and replays scripted answers, and other packages may use it for component tests of code that sets up ports.

### Kernel port tests

The component tests in `tests/port_component.rs` run against `FakeIp` without root. They pin the command sequence of a 20,000 host slice, the refusal of each kind of management interface before any change, the rollback after a failed step, a sysctl that does not take its value, and teardown when the interface cannot move back.

The integration tests in `tests/kernel_port.rs` bring up the QA-01 harness, hand the bare client arm interface to setup, and open one TCP session from every pool address of the port namespace to a server in the server namespace. The server checks that each session arrives from the address of the host index that the client sent. After teardown the tests compare the links, addresses, and routes of the client namespace and of the test namespace with snapshots taken before setup. They need root, so `make test` leaves them out, and `make e2e-hst` runs them. The workflow `.github/workflows/hst.yml` runs them as root on every change to the crate or the harness.

| Test | What it proves |
|---|---|
| `routed_port_binds_and_connects_from_20000_pool_addresses` | The router sends the pool prefix to the port address, 20,000 sessions bind and connect from 20,000 distinct pool addresses, setup refuses the server arm because it carries the default route, and teardown restores both namespaces |
| `on_link_port_answers_for_pool_addresses` | The router resolves 500 hosts on the link, the port answers for them, and the gateway inside the prefix stays reachable |
| `vlan_port_answers_for_on_link_pool_addresses` | The same on the subinterface `vlan100`, skipped without the 8021q module unless `FPS_REQUIRE_VLAN` is 1 |

On a development host the routed test opened its 20,000 sessions in about one second, setup took about 130 ms, and teardown took about 50 ms.

### Limits

The integration tests cover IPv4 only, because the QA-01 harness has no IPv6 arm. IPv6 setup issues the same steps and is covered by unit tests. The on-link test stays at 500 hosts, because the neighbor table limit `gc_thresh3` of 1,024 entries is global to the host, so a device under test that resolves 20,000 hosts on the link needs a larger limit on its side.

## MAC allocation

A kernel port needs no host MAC addresses of its own, because the kernel answers for the addresses it holds. A user space backend owns the frames, so every host needs a MAC address that is unique on the link. `MacAllocator` is the ledger of one port. The contract fixes the address as `mac_base + i`, so agents that serve slices of one pool agree without talking, and the allocator keeps that rule and adds the checks below.

| Call | What it does |
|---|---|
| `reserve(label, mac)` | Takes an address that a station of the port owns already, such as the interface or the gateway |
| `claim(pool, range)` | Takes the addresses of one slice and returns a `MacBlock` with `mac_of(index)` and `host_of(mac)` |

A claim fails, and leaves the ledger as it was, when the block overlaps an earlier claim or reservation, contains a multicast address (first octet odd) or the all zero address, or runs past 48 bits. The error names the first address of the overlap and both owners.

A pool without `mac_base` gets a base from the allocator: a locally administered unicast block (`02:xx:xx:xx:xx:xx`) that starts at a point derived from the pool name and moves up past every taken block, wrapping once. The same claims in the same order give the same base. Two agents on one link agree on it only when they make the same claims, so the controller should set `mac_base` for a pool that more than one agent serves.

## ARP and neighbor discovery

`NeighborTable` lists the slices of a port with their MAC blocks. `HostPool::index_of_ipv4` and `index_of_ipv6` invert the address mapping with arithmetic only, so a lookup costs no memory per host. The table refuses two slices that answer for overlapping IPv4 or IPv6 addresses. `Responder` reads one Ethernet frame and writes the reply into a caller buffer of `MAX_REPLY_LEN` bytes, and it keeps no state, so the workers of all queues share one.

| Request | Reply |
|---|---|
| ARP request for a host address | ARP reply from the host MAC to the sender hardware address, padded to 60 bytes |
| ARP probe (sender 0.0.0.0) for a host address | The same reply, which defends the address as RFC 5227 says |
| ARP announcement (sender equals target) | No reply |
| Neighbor solicitation for a host address | Neighbor advertisement with the solicited and override flags and the target link layer option, sent to the source link layer address of the request |
| Solicitation from `::` (duplicate address detection) | Advertisement to `ff02::1` and `33:33:00:00:00:01` without the solicited flag |

VLAN tags (up to two) are copied to the reply. A solicitation must have hop limit 255, a good ICMPv6 checksum, options of nonzero length that fit the packet, and the target or its solicited node address as destination, and a probe may carry no source link layer option, as RFC 4861 section 7.1.1 says. A frame that breaks a rule is counted as malformed and gets no answer. Requests for the gateway, for hosts of another slice, and for any other address get no answer. ARP replies, other ICMPv6 messages, and every other frame are handed back to the caller, so the transport stack sees them.

`NeighborService` runs the responder on one `FrameIo` queue. `poll` receives a batch, collects the replies in slots that it allocated once, sends them in one batch, and calls a callback for each passed frame. A full transmit queue is retried after a flush eight times, and then the remaining replies count as `tx_dropped`. The `wire` module builds the same frames for the active side, such as a host that resolves its gateway, and parses them back.

### Tests and the 20,000 host proof

The unit tests in `mac.rs`, `neighbor.rs`, and `wire.rs` cover the allocator rules, the address inverse, every reply field, each RFC 4861 validation rule, and truncated and random frames, with property tests for the inverse, the allocator, and the responder. `tests/neighbor_port.rs` runs the service over `fps_backend_afxdp::memory::frame_pair`, the in memory queue pair of TRN-11, with no root and no host. A router end asks for every host, the service answers, and the test writes both directions to a pcap file under `target/tmp/hst-03/`. `tests/score_hosts.py` scores that file with `realism.hosts.check`, the host identity check.

| Capture | Hosts | Result |
|---|---|---|
| 20,000 IPv4 hosts, one pool, ARP | 20,001 (with the router) | Passes, 20,001 MAC addresses, no conflict |
| 20,000 IPv6 hosts, one pool, neighbor discovery | 20,001 | Passes, no conflict |
| 20,000 IPv4 hosts in two pools, one with an allocated base | 20,001 | Passes, no conflict |
| Two responders that answer for the same 50 addresses with different MAC bases | 51 | Fails with 50 address conflicts, as it must |

The last row shows that the check can fail on a conflict, and a shared ledger refuses the second claim in a separate test. The tests need `python3`.

### Limits

HST-03 answers for hosts and does not resolve anything for them. A host that needs the MAC address of its gateway uses the `wire` builders and learns the reply from the passed frames, which the TRN backends do. DHCP is HST-04. The tests do not cover an AF_XDP queue, because the memory pair has the same trait, and the AF_XDP backend is another package.
