// SPDX-License-Identifier: GPL-3.0-only
//! In memory duplex transport for tests.
//!
//! [`duplex`] returns two connected [`MemoryIo`] ends. Each end implements
//! [`SessionIo`], including [`SessionIo::abort`], so tests can run a client
//! and a server program against each other without sockets and can observe
//! resets the way a TCP peer observes them.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

use crate::SessionIo;

/// Returns two connected ends of an in memory byte stream.
///
/// Each direction buffers at most `max_buf_size` bytes, and a writer waits
/// while its direction is full, so a small value exercises partial reads and
/// back pressure. A value of 1 delivers every byte in its own read.
pub fn duplex(max_buf_size: usize) -> (MemoryIo, MemoryIo) {
    let (a, b) = tokio::io::duplex(max_buf_size);
    let state = Arc::new(State::default());
    (
        MemoryIo {
            inner: Some(a),
            state: Arc::clone(&state),
            side: 0,
        },
        MemoryIo {
            inner: Some(b),
            state,
            side: 1,
        },
    )
}

#[derive(Debug, Default)]
struct State {
    /// `aborted[i]` is true once end `i` called [`SessionIo::abort`].
    aborted: [AtomicBool; 2],
}

/// One end of an in memory duplex.
///
/// [`AsyncWrite::poll_shutdown`] closes the write direction, and the peer then
/// reads end of stream after the buffered bytes, which matches a TCP FIN.
/// [`SessionIo::abort`] drops both directions at once, and the peer's next read
/// or write fails with [`io::ErrorKind::ConnectionReset`], which matches a TCP
/// RST.
#[derive(Debug)]
pub struct MemoryIo {
    /// The pipe, or `None` after this end aborted.
    inner: Option<DuplexStream>,
    state: Arc<State>,
    /// Index of this end in `State::aborted`, 0 or 1.
    side: usize,
}

impl MemoryIo {
    fn peer_aborted(&self) -> bool {
        self.state.aborted[1 - self.side].load(Ordering::Acquire)
    }

    /// Returns the pipe, or the error that an operation on this end reports.
    fn pipe(&mut self) -> io::Result<Pin<&mut DuplexStream>> {
        if self.peer_aborted() {
            return Err(reset());
        }
        match self.inner.as_mut() {
            Some(inner) => Ok(Pin::new(inner)),
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "this end of the memory duplex was aborted",
            )),
        }
    }

    /// Maps the outcome of a pipe operation, turning any outcome that raced
    /// with a peer abort into a reset.
    fn after<T>(&self, outcome: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        match outcome {
            Poll::Ready(_) if self.peer_aborted() => Poll::Ready(Err(reset())),
            other => other,
        }
    }
}

fn reset() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionReset,
        "the peer of the memory duplex aborted",
    )
}

impl AsyncRead for MemoryIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let outcome = match this.pipe() {
            Ok(pipe) => pipe.poll_read(cx, buf),
            Err(e) => return Poll::Ready(Err(e)),
        };
        let outcome = this.after(outcome);
        if matches!(outcome, Poll::Ready(Err(_))) {
            // A reset discards the bytes that arrived before it.
            buf.set_filled(before);
        }
        outcome
    }
}

impl AsyncWrite for MemoryIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let outcome = match this.pipe() {
            Ok(pipe) => pipe.poll_write(cx, data),
            Err(e) => return Poll::Ready(Err(e)),
        };
        this.after(outcome)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let outcome = match this.pipe() {
            Ok(pipe) => pipe.poll_flush(cx),
            Err(e) => return Poll::Ready(Err(e)),
        };
        this.after(outcome)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let outcome = match this.pipe() {
            Ok(pipe) => pipe.poll_shutdown(cx),
            Err(e) => return Poll::Ready(Err(e)),
        };
        this.after(outcome)
    }
}

impl SessionIo for MemoryIo {
    fn abort(&mut self) {
        if let Some(inner) = self.inner.take() {
            // Publish the flag before the drop wakes the peer, so the woken
            // peer reads the flag and reports a reset in place of end of stream.
            self.state.aborted[self.side].store(true, Ordering::Release);
            drop(inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn bytes_cross_in_both_directions() {
        let (mut a, mut b) = duplex(64);
        a.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        b.write_all(b"pong").await.unwrap();
        a.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"pong");
    }

    #[tokio::test]
    async fn shutdown_delivers_buffered_bytes_then_end_of_stream() {
        let (mut a, mut b) = duplex(64);
        a.write_all(b"last").await.unwrap();
        a.shutdown().await.unwrap();
        let mut out = Vec::new();
        b.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"last");
        // The other direction stays open after a half close.
        b.write_all(b"reply").await.unwrap();
        let mut buf = [0u8; 5];
        a.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"reply");
    }

    #[tokio::test]
    async fn abort_resets_the_peer_and_discards_buffered_bytes() {
        let (mut a, mut b) = duplex(64);
        a.write_all(b"unread").await.unwrap();
        a.abort();
        let mut buf = [0u8; 16];
        let err = b.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
        let err = b.write_all(b"late").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    }

    #[tokio::test]
    async fn abort_wakes_a_pending_reader() {
        let (mut a, mut b) = duplex(64);
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 16];
            b.read(&mut buf).await
        });
        tokio::task::yield_now().await;
        a.abort();
        let err = reader.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    }

    #[tokio::test]
    async fn aborted_end_reports_not_connected_and_abort_is_idempotent() {
        let (mut a, _b) = duplex(64);
        a.abort();
        a.abort();
        let err = a.write_all(b"x").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        let mut buf = [0u8; 1];
        let err = a.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    #[tokio::test]
    async fn dropping_an_end_is_end_of_stream_not_reset() {
        let (a, mut b) = duplex(64);
        drop(a);
        let mut buf = [0u8; 1];
        assert_eq!(b.read(&mut buf).await.unwrap(), 0);
    }
}
