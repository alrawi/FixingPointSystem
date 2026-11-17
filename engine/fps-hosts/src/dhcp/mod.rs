// SPDX-License-Identifier: GPL-3.0-only
//! DHCPv4 for the hosts of a pool on a user space backend.
//!
//! A host pool with `dhcp4` has no prefix, so its hosts get their addresses
//! from a DHCP server on the link of the port. The module has five parts.
//!
//! 1. [`msg`] reads and writes the DHCP message of RFC 2131 and RFC 2132.
//! 2. [`frame`] wraps a message in Ethernet, IPv4, and UDP headers and reads
//!    them back, with both checksums.
//! 3. [`client`] is the state machine of one host: discover, request, bound,
//!    renew at T1, rebind at T2, and start over. It has no I/O and no clock.
//! 4. [`leases`] is [`LeaseTable`], the addresses that the hosts of a slice
//!    hold. It is what the rest of the engine reads where a static pool
//!    would map an index to an address.
//! 5. [`hosts`] is [`DhcpHosts`], which runs the clients of a slice on one
//!    [`fps_backend_afxdp::FrameIo`] queue and fills the lease table.
//!
//! A pool is made with [`crate::HostPool::dhcp`], which takes no prefix, and
//! a kernel port refuses such a pool, because the kernel backend has no DHCP
//! client and needs the addresses at setup.

pub mod client;
pub mod frame;
pub mod hosts;
pub mod leases;
pub mod msg;

pub use client::{Client, Dest, Event, Lease, Outgoing, State, Step, Timing};
pub use hosts::{DhcpConfig, DhcpError, DhcpHosts, DhcpStats};
pub use leases::{HostLease, LeaseTable};
pub use msg::{Message, MessageType, Options, ParseError, CLIENT_PORT, SERVER_PORT};
