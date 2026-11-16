// SPDX-License-Identifier: GPL-3.0-only
//! Host pool allocator.
//!
//! A host pool is a numbered set of emulated hosts on one port. Host index `i`
//! of a pool maps to the `i`-th usable address of its IPv4 or IPv6 prefix, as
//! the `HostPoolSlice` message of `contracts/proto/fps/agent/v1/agent.proto`
//! defines it, and to the MAC address `mac_base + i`.
//!
//! The crate offers three things.
//!
//! 1. [`HostPool`] parses a pool of the test configuration and maps host
//!    indexes to addresses ([`pool`]).
//! 2. [`HostRange::split`] slices a pool, or the slice of one agent, into
//!    contiguous ranges that are disjoint and cover it, one range per agent or
//!    per worker ([`slice`]).
//! 3. [`HostSelector`] maps a session id to a host of a range with round
//!    robin, uniform, or Zipf selection, derived from the run seed only
//!    ([`select`]).
//! 4. [`port::setup`] configures a kernel test port for one slice in a
//!    network namespace of its own, and [`KernelPort::teardown`] removes it
//!    ([`port`], [`sysctl`], [`ipcmd`]).
//! 5. [`MacAllocator`] hands out the MAC addresses of the hosts of a port
//!    without conflicts ([`mac`]), [`Responder`] answers ARP and IPv6 neighbor
//!    discovery for them ([`neighbor`], [`wire`]), and [`NeighborService`]
//!    runs the responder on a queue of a user space backend ([`service`]).
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod ipcmd;
pub mod mac;
pub mod neighbor;
pub mod pool;
pub mod port;
pub mod select;
pub mod service;
pub mod slice;
pub mod sysctl;
pub mod wire;

pub use ipcmd::{CommandError, FakeIp, IpCommand, IpRunner};
pub use mac::{MacAllocator, MacBlock, MacError};
pub use neighbor::{NeighborError, NeighborTable, Protocol, Responder, Verdict};
pub use pool::{HostPool, Ipv4Prefix, Ipv6Prefix, MacAddr, PoolError, PoolSpec};
pub use port::{cleanup, setup, InterfaceAddr, KernelPort, LinkState, PortError, PortSpec};
pub use select::{HostSelector, SelectError, Selection};
pub use service::{NeighborService, NeighborStats};
pub use slice::{check_partition, HostRange, SliceError};
pub use sysctl::{BufferSizes, ProfileError, SysctlProfile};
