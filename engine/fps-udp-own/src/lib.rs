// SPDX-License-Identifier: GPL-3.0-only
#![deny(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The user space UDP layer of the engine (TRN-29).
//!
//! The layer carries UDP datagrams over Ethernet and IPv4 or IPv6 frames and offers
//! them through [`fps_io::DatagramIo`], so the UDP modules (dns, ntp,
//! sip_rtp) run on it the way they run on the kernel backend. It has no
//! socket of the kernel and no thread: a datapath loop hands it the frames of
//! one queue and takes the frames that it wants to send.
//!
//! # Shape
//!
//! A [`UdpArm`] is the layer of one queue. It serves the hosts of one side of
//! a lane, which `fps-tcp-own` describes as a [`Station`] (the hosts of the
//! side, their MAC addresses, the neighbors, and the VLAN of the port). The
//! datapath loop calls two methods:
//!
//! * [`UdpArm::receive`] takes one Ethernet frame that the classifier
//!   called IPv4 or IPv6 UDP, verifies it, and queues the datagram at the session
//!   that it belongs to, or at a new session of a listener.
//! * [`UdpArm::drain_tx`] hands over the frames that sessions queued.
//!
//! Tasks call the other side. [`UdpArm::connect`] opens a [`UdpSession`] from
//! a host of the arm to a peer with an ephemeral port of that host, and a
//! [`UdpListener`] accepts a session for each peer that sends to a listened
//! port at any host of the arm. A session is connected: it sends to its peer
//! and receives only from it, as the kernel backend's `KernelDatagram` does,
//! and implements [`DatagramIo`].
//!
//! ```text
//!   task --poll_send--> UdpSession --frames--> outbox --drain_tx--> loop --> queue
//!   task <--poll_recv-- UdpSession <-datagram- receive <------------ loop <-- queue
//! ```
//!
//! # What is checked and what is not
//!
//! The layer computes the IPv4 header checksum and the UDP checksum (of either
//! family) on send
//! and verifies both on receive (a received IPv4 UDP checksum of zero means none
//! and is accepted, RFC 768). Frames that fail a check are dropped and
//! counted in [`UdpStats`].
//!
//! * **Fragmentation.** A datagram larger than the MTU leaves as IPv4
//!   fragments, so every size that [`fps_io::MAX_DATAGRAM`] allows can be
//!   sent. The layer does **not reassemble**: a received fragment is dropped
//!   and counted as `rx_fragments`. A peer that fragments its datagrams
//!   (above about 1,472 payload bytes at an MTU of 1,500) is not heard.
//!   Reassembly needs a timer and a memory budget per host, and no module
//!   sends datagrams that large, so it waits for a package that needs it.
//! * **ICMP.** The layer sends no ICMP port unreachable and reads no ICMP, so
//!   a send never fails with `ConnectionRefused`. The datapath classifier
//!   drops ICMP before it reaches the layer.
//! * **Neighbors.** There is no ARP of its own. The MAC address of a peer
//!   comes from the neighbor table of the station, and for a peer that is
//!   not in it the source MAC address of the first frame is used (a server
//!   session) or the connect fails with `HostUnreachable` (a client session).
//! * **IPv6** (TRN-30) works as IPv4 does, with the 40 byte header, a required
//!   UDP checksum, and a fragment header for datagrams above the MTU. The
//!   layer reads no extension header besides the fragment header, which it
//!   counts and drops.
//! * **Ports 67 and 68** are DHCP and never reach the layer.
//!
//! The layer has no unsafe code and takes no dependency besides `fps-io` and
//! `fps-tcp-own` (for the station, the host pools, and the Ethernet framing).

mod arm;
mod session;
pub mod wire;

pub use arm::{UdpArm, UdpConfig, UdpStats};
pub use fps_tcp_own::net::Station;
pub use session::{UdpListener, UdpSession};

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks a mutex and goes on after a panic in another thread: the data that
/// the layer guards stays consistent between statements.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
