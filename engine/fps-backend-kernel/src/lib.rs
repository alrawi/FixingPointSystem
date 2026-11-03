// SPDX-License-Identifier: GPL-3.0-only
//! Kernel TCP and UDP transport backend of the engine.
//!
//! The backend implements [`fps_io::SessionIo`] over Linux TCP sockets and
//! [`fps_io::DatagramIo`] over Linux UDP sockets.
//! A [`Connector`] opens client sessions from host pool source addresses. It
//! binds each socket to its source address with `IP_BIND_ADDRESS_NO_PORT`, so
//! the kernel picks the source port at connect time and shares a port across
//! destinations, and it limits that choice to a configured local port range
//! with `IP_LOCAL_PORT_RANGE`. A [`KernelListener`] accepts server sessions on
//! pool addresses with a configurable backlog. Both apply the buffer size,
//! Nagle, linger, TTL, MSS, and window clamp settings of [`SocketOptions`].
//! An [`OsProfile`] fills the last three from the profile of an operating
//! system and lists the sysctls that Linux offers only per network namespace.
//!
//! Every session is a [`KernelStream`], whose [`fps_io::SessionIo::abort`]
//! closes the socket with `SO_LINGER` set to zero, so the peer receives a
//! reset.
//!
//! UDP sessions are [`KernelDatagram`] sockets. [`Connector::connect_datagram`]
//! opens a client socket bound to a pool address and connected to the peer,
//! and a [`KernelDatagramListener`] binds a pool address and hands out one
//! connected server socket per peer. [`DatagramConnectOpener`] and
//! [`DatagramAcceptOpener`] connect both to the interpreter as
//! [`fps_interp::DatagramOpener`] implementations.
//!
//! The crate denies `unsafe` code except in one private module that sets the
//! three socket options that `socket2` does not expose and receives
//! datagrams with `MSG_TRUNC`.
#![deny(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod connect;
mod datagram;
mod error;
mod listen;
mod options;
mod profile;
mod stream;
// The module wraps setsockopt for IP_BIND_ADDRESS_NO_PORT,
// IP_LOCAL_PORT_RANGE, and TCP_WINDOW_CLAMP, and recv with MSG_TRUNC, which
// neither socket2 nor tokio exposes.
#[allow(unsafe_code)]
mod sys;

pub use connect::Connector;
pub use datagram::{
    bind_datagram_pool, DatagramAcceptOpener, DatagramConnectOpener, KernelDatagram,
    KernelDatagramListener, SESSION_QUEUE,
};
pub use error::BackendError;
pub use listen::{bind_pool, KernelListener};
pub use options::{ConnectConfig, Linger, ListenConfig, PortRange, SocketOptions};
pub use profile::{
    read_proc_sysctl, OsProfile, ProfileError, ProfileSocket, Reference, SysctlMismatch,
    SystemSettings, BUILTIN_PROFILES,
};
pub use stream::KernelStream;
