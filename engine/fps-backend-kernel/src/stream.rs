// SPDX-License-Identifier: GPL-3.0-only
//! The session stream of the kernel backend.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use fps_io::SessionIo;
use socket2::SockRef;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// One kernel TCP session, on the client or the server side.
///
/// Reads and writes go to the socket, and [`AsyncWrite::poll_shutdown`] sends
/// FIN. [`SessionIo::abort`] sets `SO_LINGER` to zero and closes the socket,
/// so the kernel discards unsent data and sends a reset. Every later
/// operation fails with [`io::ErrorKind::NotConnected`]. Dropping the stream
/// closes it with the linger behavior of its [`crate::SocketOptions`].
#[derive(Debug)]
pub struct KernelStream {
    /// The socket, or `None` after an abort.
    inner: Option<TcpStream>,
}

impl KernelStream {
    pub(crate) fn new(stream: TcpStream) -> Self {
        Self {
            inner: Some(stream),
        }
    }

    /// Returns the local address and port of the session.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.tcp()?.local_addr()
    }

    /// Returns the address and port of the peer.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.tcp()?.peer_addr()
    }

    /// Returns the socket for inspection, or `None` after an abort.
    ///
    /// Backends that set further options, such as the operating system
    /// profiles of TRN-03, use it with `socket2::SockRef`.
    pub fn tcp_stream(&self) -> Option<&TcpStream> {
        self.inner.as_ref()
    }

    fn tcp(&self) -> io::Result<&TcpStream> {
        self.inner.as_ref().ok_or_else(not_connected)
    }

    fn pinned(&mut self) -> io::Result<Pin<&mut TcpStream>> {
        self.inner.as_mut().map(Pin::new).ok_or_else(not_connected)
    }
}

fn not_connected() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        "the kernel session was aborted",
    )
}

impl AsyncRead for KernelStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut().pinned() {
            Ok(s) => s.poll_read(cx, buf),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl AsyncWrite for KernelStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut().pinned() {
            Ok(s) => s.poll_write(cx, data),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut().pinned() {
            Ok(s) => s.poll_write_vectored(cx, bufs),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().pinned() {
            Ok(s) => s.poll_flush(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().pinned() {
            Ok(s) => s.poll_shutdown(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl SessionIo for KernelStream {
    fn abort(&mut self) {
        if let Some(stream) = self.inner.take() {
            // A zero linger makes close send a reset and return at once. If
            // the kernel refuses the option, the drop below still closes the
            // socket, with FIN in place of the reset, and nothing remains open.
            let _ = SockRef::from(&stream).set_linger(Some(Duration::ZERO));
            drop(stream);
        }
    }
}
