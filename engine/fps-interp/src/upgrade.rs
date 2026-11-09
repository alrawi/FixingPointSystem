// SPDX-License-Identifier: GPL-3.0-only
//! The `tls_upgrade` seam.
//!
//! A flow binds its transport when it connects or accepts. The `tls_upgrade`
//! action hands that transport to a [`TlsUpgrader`] in the middle of the
//! session and continues over the secured transport that comes back, which is
//! how STARTTLS in SMTP, IMAP, POP3, FTP, and LDAP works. The interpreter
//! owns no TLS code: the engine runtime supplies an upgrader over the TLS
//! module, and tests supply their own.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use fps_flow::program::TlsRole;
use fps_io::SessionIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A session transport behind a pointer, which an upgrade replaces by a
/// wrapper around itself.
pub type BoxedIo<'io> = Box<dyn SessionIo + 'io>;

/// The future of an upgrade, which resolves to the secured transport.
pub type UpgradeFuture<'io> =
    Pin<Box<dyn Future<Output = Result<BoxedIo<'io>, UpgradeError>> + Send + 'io>>;

/// A rendered `tls_upgrade` action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpgradeRequest<'a> {
    /// The role in the handshake. The client role starts the handshake and
    /// the server role waits for it.
    pub role: TlsRole,
    /// The name of the TLS profile, which the upgrader resolves.
    pub profile: &'a str,
    /// The server name that the action names, or `None` to let the upgrader
    /// choose.
    pub server_name: Option<&'a str>,
}

/// Why an upgrade failed.
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    /// The upgrader cannot serve the request, for example because the run
    /// defines no profile of that name. The session fails with reason
    /// `internal`.
    #[error("{0}")]
    Unavailable(String),
    /// The handshake failed or the transport broke during it. The session
    /// fails with reason `tls_failure`.
    #[error("tls handshake failed")]
    Handshake(#[source] io::Error),
}

/// Upgrades the transport of a running session to TLS.
///
/// The interpreter calls it once per `tls_upgrade` action, with the
/// transport that carried the session so far. The returned transport
/// replaces it, and the transport that the upgrader consumed lives on
/// below the TLS layer, so a capture tap on it keeps recording the bytes on
/// the wire, which are now encrypted. The interpreter bounds the future with
/// the open timeout of the session and drops it when the session stops.
pub trait TlsUpgrader: Sync {
    /// Runs the handshake of `request` over `io`.
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io>;
}

/// A transport that an upgrade can replace. It is empty after a failed
/// upgrade, and every operation then fails with
/// [`io::ErrorKind::NotConnected`].
pub(crate) struct Swappable<'s> {
    io: Option<BoxedIo<'s>>,
}

impl<'s> Swappable<'s> {
    pub(crate) fn new(io: BoxedIo<'s>) -> Self {
        Self { io: Some(io) }
    }

    /// Takes the transport out, leaving the swappable empty.
    pub(crate) fn take(&mut self) -> Option<BoxedIo<'s>> {
        self.io.take()
    }

    /// Puts the secured transport in.
    pub(crate) fn put(&mut self, io: BoxedIo<'s>) {
        self.io = Some(io);
    }

    fn live(&mut self) -> io::Result<&mut BoxedIo<'s>> {
        self.io
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "transport upgraded"))
    }
}

impl AsyncRead for Swappable<'_> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut().live() {
            Ok(io) => Pin::new(&mut **io).poll_read(cx, buf),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl AsyncWrite for Swappable<'_> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut().live() {
            Ok(io) => Pin::new(&mut **io).poll_write(cx, data),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut().live() {
            Ok(io) => Pin::new(&mut **io).poll_write_vectored(cx, bufs),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.io.as_ref().is_some_and(|io| io.is_write_vectored())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().live() {
            Ok(io) => Pin::new(&mut **io).poll_flush(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().live() {
            Ok(io) => Pin::new(&mut **io).poll_shutdown(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl SessionIo for Swappable<'_> {
    fn abort(&mut self) {
        if let Some(io) = &mut self.io {
            io.abort();
        }
    }
}

/// A transport that the caller keeps, which the swappable borrows.
pub(crate) struct Borrowed<'a, S>(pub(crate) &'a mut S);

impl<S: SessionIo> AsyncRead for Borrowed<'_, S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_read(cx, buf)
    }
}

impl<S: SessionIo> AsyncWrite for Borrowed<'_, S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.get_mut().0).poll_write(cx, data)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.get_mut().0).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_shutdown(cx)
    }
}

impl<S: SessionIo> SessionIo for Borrowed<'_, S> {
    fn abort(&mut self) {
        self.0.abort();
    }
}
