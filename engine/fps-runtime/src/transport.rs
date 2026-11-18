// SPDX-License-Identifier: GPL-3.0-only
//! The transport seam of the workers (ENG-31).
//!
//! A worker opens every session through a [`Transport`]: a client dials with
//! [`Transport::connect`] or [`Transport::connect_datagram`], and a server
//! listens with [`Transport::listen`] or [`Transport::listen_datagram`] and
//! accepts from the listener that comes back. Each call returns a boxed
//! [`SessionIo`] or [`DatagramIo`] together with the two addresses of the
//! session, which the capture tap and the instances need. The kernel backend
//! implements the trait as [`crate::KernelTransport`]. The own stack of
//! `fps-datapath` implements it with `Datapath::connector` and
//! `Datapath::listen` (AGT-25), and tests implement it over the in memory
//! duplex of `fps-io`.
//!
//! The trait is not named at the root of the crate, where [`crate::Transport`]
//! already is the enum of the extension host that tells TCP from UDP.
//!
//! # What a transport must do
//!
//! - `connect` and `connect_datagram` take a source address that belongs to a
//!   host of the run and a destination, and fail with an error whose kind says
//!   why. The runtime retries a `connect` of a data session that fails with
//!   [`io::ErrorKind::ConnectionRefused`], because the peer may not have bound
//!   its port yet, and a `listen` that fails with [`io::ErrorKind::AddrInUse`],
//!   because a socket of another process may still hold the port.
//! - `listen` binds every address of the call. The runtime makes one call per
//!   port, so a transport whose stack serves a port for all server hosts
//!   listens once for the call, and the kernel backend binds one socket per
//!   address. A port 0 asks for a port of the transport's choice, which
//!   [`StreamListener::local_addrs`] reports.
//! - `accept` is cancel safe: a future that the caller drops loses no session.
//! - A transport is shared by the threads of the workers, so it is `Send` and
//!   `Sync`. Its listeners and futures stay on the thread of one worker.

use std::fmt;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};

use fps_io::{DatagramIo, SessionIo};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A future that stays on the thread of its worker.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Opens and accepts the sessions of the workers.
///
/// See the [module](self) for what an implementation owes the runtime.
pub trait Transport: fmt::Debug + Send + Sync {
    /// Opens a TCP session from the host `source` to `destination`.
    fn connect(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> BoxFuture<'_, io::Result<StreamSession>>;

    /// Starts listening on `addresses`, which share one port or have port 0.
    ///
    /// # Errors
    ///
    /// Returns the error of the first address that cannot bind, after it
    /// released the addresses that it had bound.
    fn listen(&self, addresses: &[SocketAddr]) -> io::Result<Box<dyn StreamListener>>;

    /// Opens a UDP session from the host `source` to `destination`.
    ///
    /// Nothing is sent, because UDP has no handshake.
    ///
    /// # Errors
    ///
    /// Returns an error when the source does not belong to the host range or
    /// the transport has no free port for it.
    fn connect_datagram(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> io::Result<DatagramSession>;

    /// Starts listening for UDP on `addresses`. The listener gives one session
    /// for each peer that sends to an address and has no open session.
    ///
    /// # Errors
    ///
    /// As [`Transport::listen`].
    fn listen_datagram(&self, addresses: &[SocketAddr]) -> io::Result<Box<dyn DatagramListener>>;
}

/// The accept side of [`Transport::listen`]. Dropping it closes the
/// listening sockets.
pub trait StreamListener {
    /// Returns the bound addresses, in the order of the addresses that the
    /// listener was asked for, with the real port in place of port 0.
    fn local_addrs(&self) -> Vec<SocketAddr>;

    /// Completes with the next session of any address of the listener.
    ///
    /// An error concerns one session, for example `EMFILE`, and the listener
    /// stays usable. The future is cancel safe.
    fn accept(&mut self) -> BoxFuture<'_, io::Result<StreamSession>>;
}

/// The accept side of [`Transport::listen_datagram`].
pub trait DatagramListener {
    /// Returns the bound addresses, as [`StreamListener::local_addrs`] does.
    fn local_addrs(&self) -> Vec<SocketAddr>;

    /// Completes with the session of the next peer that has none, which
    /// holds the datagram that opened it as its first receive.
    ///
    /// An error concerns one datagram or one session, and the listener stays
    /// usable. The future is cancel safe. The listener may close its sockets
    /// only when it is dropped, and the sessions that it gave out may send
    /// and receive until they close, whatever the listener does.
    fn accept(&mut self) -> BoxFuture<'_, io::Result<DatagramSession>>;
}

/// One TCP session of a [`Transport`], with its addresses.
///
/// The addresses are those of the session when the transport handed it out.
/// An address is `None` when the transport could not tell it, as a kernel
/// socket cannot after the peer reset the connection at once.
pub struct StreamSession {
    io: Box<dyn SessionIo>,
    local: Option<SocketAddr>,
    peer: Option<SocketAddr>,
}

impl StreamSession {
    /// Returns a session over `io` between `local` and `peer`.
    pub fn new(
        io: impl SessionIo + 'static,
        local: Option<SocketAddr>,
        peer: Option<SocketAddr>,
    ) -> Self {
        Self {
            io: Box::new(io),
            local,
            peer,
        }
    }

    /// Returns the address of this side.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local
    }

    /// Returns the address of the peer.
    #[must_use]
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer
    }
}

impl fmt::Debug for StreamSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamSession")
            .field("local", &self.local)
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl AsyncRead for StreamSession {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for StreamSession {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.io).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.io).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.io).poll_shutdown(cx)
    }
}

impl SessionIo for StreamSession {
    fn abort(&mut self) {
        self.io.abort();
    }
}

/// One UDP session of a [`Transport`], with its addresses.
pub struct DatagramSession {
    io: Box<dyn DatagramIo>,
    local: SocketAddr,
    peer: SocketAddr,
}

impl DatagramSession {
    /// Returns a session over `io` between `local` and `peer`.
    pub fn new(io: impl DatagramIo + 'static, local: SocketAddr, peer: SocketAddr) -> Self {
        Self {
            io: Box::new(io),
            local,
            peer,
        }
    }

    /// Returns the address of this side.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Returns the address of the peer.
    #[must_use]
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    /// Returns the boxed I/O and the two addresses, for a wrapper that adds
    /// to the session and builds a new one.
    #[must_use]
    pub fn into_parts(self) -> (Box<dyn DatagramIo>, SocketAddr, SocketAddr) {
        (self.io, self.local, self.peer)
    }
}

impl fmt::Debug for DatagramSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatagramSession")
            .field("local", &self.local)
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl DatagramIo for DatagramSession {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        self.io.poll_send(cx, datagram)
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        self.io.poll_recv(cx, buf)
    }

    fn close(&mut self) {
        self.io.close();
    }
}

#[cfg(test)]
mod tests {
    use fps_io::memory::duplex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[tokio::test]
    async fn a_stream_session_carries_bytes_and_its_addresses() {
        let (a, b) = duplex(64);
        let mut a = StreamSession::new(a, Some(addr(1)), Some(addr(2)));
        let mut b = StreamSession::new(b, Some(addr(2)), None);
        assert_eq!(
            (a.local_addr(), a.peer_addr()),
            (Some(addr(1)), Some(addr(2)))
        );
        assert_eq!(b.peer_addr(), None);
        a.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        a.abort();
        assert_eq!(
            b.read(&mut buf).await.unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
    }
}
