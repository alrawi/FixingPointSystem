// SPDX-License-Identifier: GPL-3.0-only
//! Datagram I/O seam for UDP sessions.
//!
//! A UDP session keeps datagram boundaries, so it does not fit the byte
//! stream of [`crate::SessionIo`]. The interpreter runs a UDP side over any
//! type that implements [`DatagramIo`]. A transport backend implements it
//! over a connected UDP socket, and tests use the in memory pair of
//! [`crate::memory::datagram_pair`].

use std::future::{poll_fn, Future};
use std::io;
use std::task::{Context, Poll};

/// Largest UDP payload over IPv4, which is 65,535 minus the 20 byte IPv4
/// header and the 8 byte UDP header.
pub const MAX_DATAGRAM: usize = 65_507;

/// A connected datagram socket that carries one session.
///
/// Every datagram comes from the session peer and goes to it. The transport
/// never joins, splits, reorders, deduplicates, or retransmits datagrams.
pub trait DatagramIo: Unpin + Send {
    /// Sends `datagram` as one datagram.
    ///
    /// A datagram longer than [`MAX_DATAGRAM`] fails with
    /// [`io::ErrorKind::InvalidInput`]. A connected kernel socket may report
    /// [`io::ErrorKind::ConnectionRefused`] after the peer answered an
    /// earlier datagram with ICMP port unreachable.
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>>;

    /// Receives one datagram into `buf` and returns its full length.
    ///
    /// The call consumes the whole datagram. When the datagram is longer
    /// than `buf`, the first `buf.len()` bytes land in `buf`, the rest is
    /// discarded, and the returned length still names the full size, as
    /// `recv` with `MSG_TRUNC` does on Linux. The caller detects an
    /// oversized datagram by comparing the length with `buf.len()`.
    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>>;

    /// Releases the socket.
    ///
    /// The peer observes nothing, as UDP has no close. Every later operation
    /// on this side fails with [`io::ErrorKind::NotConnected`]. Calling it
    /// twice has no further effect.
    fn close(&mut self);
}

impl<T: DatagramIo + ?Sized> DatagramIo for Box<T> {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        (**self).poll_send(cx, datagram)
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        (**self).poll_recv(cx, buf)
    }

    fn close(&mut self) {
        (**self).close();
    }
}

/// Async helpers over [`DatagramIo`].
pub trait DatagramIoExt: DatagramIo {
    /// Sends `datagram` as one datagram.
    fn send_datagram<'a>(
        &'a mut self,
        datagram: &'a [u8],
    ) -> impl Future<Output = io::Result<()>> + Send + 'a
    where
        Self: Sized,
    {
        poll_fn(move |cx| self.poll_send(cx, datagram))
    }

    /// Receives one datagram into `buf` and returns its full length, see
    /// [`DatagramIo::poll_recv`].
    fn recv_datagram<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl Future<Output = io::Result<usize>> + Send + 'a
    where
        Self: Sized,
    {
        poll_fn(move |cx| self.poll_recv(cx, buf))
    }
}

impl<T: DatagramIo + ?Sized> DatagramIoExt for T {}
