## [2026-11-18] ingest | TRN-32 host pools and Ethernet in the own stack

TRN-32 gave the own stack of `fps-tcp-own` the pieces that a test configuration with host pools needs.

- The connection table is keyed by the 4-tuple (`host::Tuple`). The hasher of the table now mixes every field. A connection stores both addresses.
- `net.rs` holds Ethernet and 802.1Q encode and decode and the `Station`, the link layer of one side. Host `i` of a pool has the MAC address `mac_base + i` (plan rule 5). The station reads neighbors from the `NeighborTable` of `fps-hosts` (read only) and answers ARP with its `Responder`. A frame with another VLAN, a tag on an untagged port, or a stacked tag is counted in `link_dropped`, and a frame with no MAC address for its source or destination in `unresolved`.
- `pools.rs` builds the two stations from `Pools`, claiming the MAC addresses of both sides from one ledger. `OwnStack::with_pools`, `with_links_and_pools`, and `connect_hosts` are new. `OwnConfig::mtu` is now the IP MTU, and the link carries 14 (18 tagged) bytes more.
- `fps-tcp`: the adapter re-exports `net`, `Pools`, and `PoolError`. `tcp_bench` has `--own-pool`, `--own-server-hosts`, and `--own-vlan`.
- Results are in `engine/fps-tcp/results/sandbox-2vcpu-trn32/`. The 1,000,000 socket row holds 1,001,129 sockets at 253 B (228 B before, explained in host.txt) and 46,500 to 48,600 probe sessions per second with a pool of 20,000, equal to the unchanged base binary in the same session (47,200 to 49,100). The acceptance of 50,000 per second is not reached in this slower sandbox session and needs a rerun on a quiet host.

Tests: unit tests of the table, Ethernet and VLAN, and neighbor lookup (`host.rs`, `net_tests.rs`), and component tests of a tagged pool link in `tests/own.rs`.

### Shared page notes

- Testing page: `cargo test -p fps-tcp-own` also runs the unit tests of `net.rs` and the pool tests of `tests/own.rs`.
- `Cargo.lock` changed: `fps-tcp-own` depends on `fps-hosts`. No root `Cargo.toml` edit was needed.
- Rerun `tcp_bench --stack own --profile full --workload scale --shards 64 --own-pool 20000` on fat-nuc or the hosted runner to read the 50,000 per second acceptance.
