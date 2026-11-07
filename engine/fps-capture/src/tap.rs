// SPDX-License-Identifier: GPL-3.0-only
//! Wrappers that record the bytes of a sampled session.
//!
//! [`Tapped`] wraps a [`SessionIo`] and [`TappedDatagram`] wraps a
//! [`DatagramIo`]. A session that the sampler did not pick gets the `Plain`
//! variant, which forwards every call after one `match` on the variant. A
//! picked session gets the `Recording` variant, which copies the bytes that pass
//! through it into its [`SessionHandle`].
//!
//! Wrap the transport below TLS, so the tap sees the encrypted bytes that a
//! packet capture would see, and give the handle the key log lines of the TLS
//! session, see [`SessionHandle::add_key_lines`].

use std::io::{self, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll};

use fps_io::{DatagramIo, SessionIo};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::SessionHandle;

/// A byte stream session with an optional recorder.
#[derive(Debug)]
pub enum Tapped<IO> {
    /// The session is not sampled, and every call goes straight to the stream.
    Plain(IO),
    /// The session is sampled, and its bytes go to the handle as well.
    Recording {
        /// The wrapped stream.
        io: IO,
        /// The recorder of the session.
        handle: SessionHandle,
    },
}

impl<IO> Tapped<IO> {
    /// Wraps `io`, recording into `handle` when there is one.
    pub fn new(io: IO, handle: Option<SessionHandle>) -> Self {
        match handle {
            Some(handle) => Self::Recording { io, handle },
            None => Self::Plain(io),
        }
    }

    /// Returns the recorder of a sampled session.
    pub fn handle(&self) -> Option<&SessionHandle> {
        match self {
            Self::Plain(_) => None,
            Self::Recording { handle, .. } => Some(handle),
        }
    }
}

impl<IO: AsyncRead + Unpin> AsyncRead for Tapped<IO> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_read(cx, buf),
            Self::Recording { io, handle } => {
                let before = buf.filled().len();
                let had_room = buf.remaining() > 0;
                let result = Pin::new(io).poll_read(cx, buf);
                match &result {
                    Poll::Ready(Ok(())) => {
                        let filled = buf.filled();
                        if filled.len() > before {
                            handle.record_in(&filled[before..]);
                        } else if had_room {
                            handle.record_peer_close();
                        }
                    }
                    Poll::Ready(Err(e)) if e.kind() == io::ErrorKind::ConnectionReset => {
                        handle.record_peer_reset();
                    }
                    _ => {}
                }
                result
            }
        }
    }
}

impl<IO: AsyncWrite + Unpin> AsyncWrite for Tapped<IO> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_write(cx, buf),
            Self::Recording { io, handle } => {
                let result = Pin::new(io).poll_write(cx, buf);
                if let Poll::Ready(Ok(n)) = &result {
                    handle.record_out(&buf[..(*n).min(buf.len())]);
                }
                result
            }
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_write_vectored(cx, bufs),
            Self::Recording { io, handle } => {
                let result = Pin::new(io).poll_write_vectored(cx, bufs);
                if let Poll::Ready(Ok(n)) = &result {
                    let mut left = *n;
                    for slice in bufs {
                        if left == 0 {
                            break;
                        }
                        let take = left.min(slice.len());
                        handle.record_out(&slice[..take]);
                        left -= take;
                    }
                }
                result
            }
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Plain(io) | Self::Recording { io, .. } => io.is_write_vectored(),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(io) | Self::Recording { io, .. } => Pin::new(io).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_shutdown(cx),
            Self::Recording { io, handle } => {
                let result = Pin::new(io).poll_shutdown(cx);
                if matches!(result, Poll::Ready(Ok(()))) {
                    handle.record_local_close();
                }
                result
            }
        }
    }
}

impl<IO: SessionIo> SessionIo for Tapped<IO> {
    fn abort(&mut self) {
        match self {
            Self::Plain(io) => io.abort(),
            Self::Recording { io, handle } => {
                handle.record_local_reset();
                io.abort();
            }
        }
    }
}

/// A datagram session with an optional recorder, see [`Tapped`].
#[derive(Debug)]
pub enum TappedDatagram<D> {
    /// The session is not sampled.
    Plain(D),
    /// The session is sampled, and each datagram goes to the handle as well.
    Recording {
        /// The wrapped socket.
        io: D,
        /// The recorder of the session.
        handle: SessionHandle,
    },
}

impl<D> TappedDatagram<D> {
    /// Wraps `io`, recording into `handle` when there is one.
    pub fn new(io: D, handle: Option<SessionHandle>) -> Self {
        match handle {
            Some(handle) => Self::Recording { io, handle },
            None => Self::Plain(io),
        }
    }
}

impl<D: DatagramIo> DatagramIo for TappedDatagram<D> {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        match self {
            Self::Plain(io) => io.poll_send(cx, datagram),
            Self::Recording { io, handle } => {
                let result = io.poll_send(cx, datagram);
                if matches!(result, Poll::Ready(Ok(()))) {
                    handle.record_out(datagram);
                }
                result
            }
        }
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self {
            Self::Plain(io) => io.poll_recv(cx, buf),
            Self::Recording { io, handle } => {
                let result = io.poll_recv(cx, buf);
                if let Poll::Ready(Ok(n)) = &result {
                    // The returned length names the full datagram, and the
                    // buffer holds at most its first bytes.
                    handle.record_in(&buf[..(*n).min(buf.len())]);
                }
                result
            }
        }
    }

    fn close(&mut self) {
        match self {
            Self::Plain(io) => io.close(),
            Self::Recording { io, handle } => {
                handle.record_local_close();
                io.close();
            }
        }
    }
}
