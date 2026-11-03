// SPDX-License-Identifier: GPL-3.0-only
//! Kernel TCP transport backend of the engine.
//!
//! The backend implements [`fps_io::SessionIo`] over Linux TCP sockets.
//! A [`Connector`] opens client sessions from host pool source addresses. It
//! binds each socket to its source address with `IP_BIND_ADDRESS_NO_PORT`, so
//! the kernel picks the source port at connect time and shares a port across
//! destinations, and it limits that choice to a configured local port range
//! with `IP_LOCAL_PORT_RANGE`. A [`KernelListener`] accepts server sessions on
//! pool addresses with a configurable backlog. Both apply the buffer size,
//! Nagle, and linger settings of [`SocketOptions`].
//!
//! Every session is a [`KernelStream`], whose [`fps_io::SessionIo::abort`]
//! closes the socket with `SO_LINGER` set to zero, so the peer receives a
//! reset.
//!
//! The crate denies `unsafe` code except in one private module that sets the
//! two socket options that `socket2` does not expose.
#![deny(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod connect;
mod error;
mod listen;
mod options;
mod stream;
// The module wraps setsockopt for IP_BIND_ADDRESS_NO_PORT and
// IP_LOCAL_PORT_RANGE, which neither socket2 nor tokio exposes.
#[allow(unsafe_code)]
mod sys;

pub use connect::Connector;
pub use error::BackendError;
pub use listen::{bind_pool, KernelListener};
pub use options::{ConnectConfig, Linger, ListenConfig, PortRange, SocketOptions};
pub use stream::KernelStream;
