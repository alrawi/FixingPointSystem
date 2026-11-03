# Hosts

This page describes the host pool allocator of the HST workstream. The crate [`fps-hosts`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-hosts) maps host indexes to addresses, slices pools by agent and by worker without overlap, and picks the host of every session from the run seed. HST-01 created the crate, and HST-02 adds the kernel port setup to the same directory.

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
