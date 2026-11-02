// SPDX-License-Identifier: GPL-3.0-only
//! Session I/O seam of the engine.
//!
//! The interpreter runs one side of a session over any type that implements
//! [`SessionIo`]. Transport backends implement the trait for kernel sockets,
//! AF_XDP, or DPDK streams, and tests use the in memory duplex in [`memory`].
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod memory;

use tokio::io::{AsyncRead, AsyncWrite};

/// A reliable, ordered byte stream that carries one session.
///
/// Reads and writes come from [`AsyncRead`] and [`AsyncWrite`], and
/// `poll_shutdown` closes the write side, which a TCP transport sends as FIN.
/// The trait adds [`SessionIo::abort`], which ends the session with a reset.
pub trait SessionIo: AsyncRead + AsyncWrite + Unpin + Send {
    /// Ends the session at once with a reset.
    ///
    /// The transport discards unsent and unread data, and the peer observes
    /// [`std::io::ErrorKind::ConnectionReset`] on its next read or write. Every
    /// later operation on this side fails with
    /// [`std::io::ErrorKind::NotConnected`]. A kernel TCP transport implements
    /// it with `SO_LINGER` set to zero followed by close. Calling it twice has
    /// no further effect.
    fn abort(&mut self);
}

impl<T: SessionIo + ?Sized> SessionIo for Box<T> {
    fn abort(&mut self) {
        (**self).abort();
    }
}
