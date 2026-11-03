// SPDX-License-Identifier: GPL-3.0-only
//! In memory transports for tests.
//!
//! [`duplex`] returns two connected [`MemoryIo`] ends. Each end implements
//! [`SessionIo`], including [`SessionIo::abort`], so tests can run a client
//! and a server program against each other without sockets and can observe
//! resets the way a TCP peer observes them.
//!
//! [`datagram_pair`] returns two connected [`MemoryDatagram`] ends, which
//! implement [`DatagramIo`] with UDP semantics, so datagrams keep their
//! boundaries and a full queue drops datagrams in place of waiting.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

use crate::datagram::{DatagramIo, MAX_DATAGRAM};
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

/// Returns two connected ends of an in memory datagram socket.
///
/// Each direction queues at most `queue` datagrams. A datagram sent while
/// its direction is full, or after the receiving end closed, is dropped
/// without an error, as UDP drops it, and [`MemoryDatagram::dropped`]
/// counts it. A `queue` of 0 drops every datagram.
pub fn datagram_pair(queue: usize) -> (MemoryDatagram, MemoryDatagram) {
    let shared = Arc::new(Datagrams {
        inbox: [Mutex::default(), Mutex::default()],
        closed: [AtomicBool::new(false), AtomicBool::new(false)],
        dropped: [AtomicU64::new(0), AtomicU64::new(0)],
        queue,
    });
    (
        MemoryDatagram {
            shared: Arc::clone(&shared),
            side: 0,
        },
        MemoryDatagram { shared, side: 1 },
    )
}

#[derive(Debug, Default)]
struct Inbox {
    datagrams: VecDeque<Vec<u8>>,
    /// The task that waits for the next datagram.
    waker: Option<Waker>,
}

#[derive(Debug)]
struct Datagrams {
    /// `inbox[i]` holds the datagrams that end `i` has yet to receive.
    inbox: [Mutex<Inbox>; 2],
    /// `closed[i]` is true once end `i` called [`DatagramIo::close`].
    closed: [AtomicBool; 2],
    /// `dropped[i]` counts the datagrams that end `i` sent and nobody received.
    dropped: [AtomicU64; 2],
    queue: usize,
}

impl Datagrams {
    fn inbox(&self, side: usize) -> MutexGuard<'_, Inbox> {
        // A test that panicked while it held the lock leaves a usable queue.
        self.inbox[side]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// One end of an in memory datagram socket.
///
/// [`DatagramIo::poll_recv`] truncates a datagram that does not fit the
/// buffer and returns its full length, as `recv` with `MSG_TRUNC` does.
/// Dropping an end closes it.
#[derive(Debug)]
pub struct MemoryDatagram {
    shared: Arc<Datagrams>,
    /// Index of this end, 0 or 1.
    side: usize,
}

impl MemoryDatagram {
    /// Returns the number of datagrams that this end sent and that the peer
    /// never received, because its queue was full or it had closed.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped[self.side].load(Ordering::Acquire)
    }

    /// Returns the number of datagrams that wait for this end.
    pub fn queued(&self) -> usize {
        self.shared.inbox(self.side).datagrams.len()
    }

    /// Waits until a datagram waits for this end, without consuming it.
    ///
    /// A server opener calls it to implement `accept`, which waits for the
    /// first datagram and leaves it queued for the first `expect`.
    pub async fn readable(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_readable(cx)).await
    }

    fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.check_open()?;
        let mut inbox = self.shared.inbox(self.side);
        if inbox.datagrams.is_empty() {
            inbox.waker = Some(cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn check_open(&self) -> io::Result<()> {
        if self.shared.closed[self.side].load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "this end of the memory datagram socket was closed",
            ))
        } else {
            Ok(())
        }
    }
}

impl DatagramIo for MemoryDatagram {
    fn poll_send(&mut self, _cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        self.check_open()?;
        if datagram.len() > MAX_DATAGRAM {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "datagram of {} bytes exceeds {MAX_DATAGRAM} bytes",
                    datagram.len()
                ),
            )));
        }
        let peer = 1 - self.side;
        let waker = {
            let mut inbox = self.shared.inbox(peer);
            if self.shared.closed[peer].load(Ordering::Acquire)
                || inbox.datagrams.len() >= self.shared.queue
            {
                self.shared.dropped[self.side].fetch_add(1, Ordering::AcqRel);
                return Poll::Ready(Ok(()));
            }
            inbox.datagrams.push_back(datagram.to_vec());
            inbox.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
        Poll::Ready(Ok(()))
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        self.check_open()?;
        let mut inbox = self.shared.inbox(self.side);
        match inbox.datagrams.pop_front() {
            Some(d) => {
                let n = d.len().min(buf.len());
                buf[..n].copy_from_slice(&d[..n]);
                Poll::Ready(Ok(d.len()))
            }
            None => {
                inbox.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    fn close(&mut self) {
        if !self.shared.closed[self.side].swap(true, Ordering::AcqRel) {
            let waker = {
                let mut inbox = self.shared.inbox(self.side);
                let unread = inbox.datagrams.len() as u64;
                inbox.datagrams.clear();
                // Datagrams that arrived but were never read count as dropped by the peer.
                self.shared.dropped[1 - self.side].fetch_add(unread, Ordering::AcqRel);
                inbox.waker.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        }
    }
}

impl Drop for MemoryDatagram {
    fn drop(&mut self) {
        self.close();
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

    use crate::datagram::DatagramIoExt;

    #[tokio::test]
    async fn datagrams_keep_their_boundaries() {
        let (mut a, mut b) = datagram_pair(8);
        a.send_datagram(b"one").await.unwrap();
        a.send_datagram(b"").await.unwrap();
        a.send_datagram(b"three").await.unwrap();
        let mut buf = [0u8; 64];
        assert_eq!(b.recv_datagram(&mut buf).await.unwrap(), 3);
        assert_eq!(&buf[..3], b"one");
        assert_eq!(b.recv_datagram(&mut buf).await.unwrap(), 0);
        assert_eq!(b.recv_datagram(&mut buf).await.unwrap(), 5);
        assert_eq!(&buf[..5], b"three");
        b.send_datagram(b"back").await.unwrap();
        assert_eq!(a.recv_datagram(&mut buf).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn an_oversized_datagram_is_truncated_and_reports_its_length() {
        let (mut a, mut b) = datagram_pair(8);
        a.send_datagram(b"0123456789").await.unwrap();
        a.send_datagram(b"next").await.unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(b.recv_datagram(&mut buf).await.unwrap(), 10);
        assert_eq!(&buf, b"0123");
        // The rest of the datagram is gone, not left for the next receive.
        assert_eq!(b.recv_datagram(&mut buf).await.unwrap(), 4);
        assert_eq!(&buf, b"next");
    }

    #[tokio::test]
    async fn send_rejects_a_datagram_above_the_udp_limit() {
        let (mut a, b) = datagram_pair(8);
        a.send_datagram(&vec![0u8; MAX_DATAGRAM]).await.unwrap();
        let err = a
            .send_datagram(&vec![0u8; MAX_DATAGRAM + 1])
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(b.queued(), 1);
    }

    #[tokio::test]
    async fn a_full_queue_drops_datagrams() {
        let (mut a, mut b) = datagram_pair(2);
        for d in [b"1", b"2", b"3"] {
            a.send_datagram(d).await.unwrap();
        }
        assert_eq!(a.dropped(), 1);
        let mut buf = [0u8; 1];
        b.recv_datagram(&mut buf).await.unwrap();
        assert_eq!(&buf, b"1");
        b.recv_datagram(&mut buf).await.unwrap();
        assert_eq!(&buf, b"2");
    }

    #[tokio::test]
    async fn recv_waits_for_a_datagram_and_readable_does_not_consume() {
        let (mut a, mut b) = datagram_pair(4);
        let reader = tokio::spawn(async move {
            b.readable().await.unwrap();
            assert_eq!(b.queued(), 1);
            let mut buf = [0u8; 8];
            let n = b.recv_datagram(&mut buf).await.unwrap();
            buf[..n].to_vec()
        });
        tokio::task::yield_now().await;
        a.send_datagram(b"late").await.unwrap();
        assert_eq!(reader.await.unwrap(), b"late");
    }

    #[tokio::test]
    async fn close_is_silent_for_the_peer_and_final_for_this_end() {
        let (mut a, mut b) = datagram_pair(4);
        b.send_datagram(b"unread").await.unwrap();
        a.close();
        a.close();
        let mut buf = [0u8; 8];
        let err = a.recv_datagram(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        let err = a.send_datagram(b"x").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        // The peer sends into the void, as UDP to a closed port does.
        b.send_datagram(b"lost").await.unwrap();
        assert_eq!(b.dropped(), 2);
    }
}
