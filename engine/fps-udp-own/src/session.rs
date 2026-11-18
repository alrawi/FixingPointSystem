// SPDX-License-Identifier: GPL-3.0-only
//! Sessions and listeners: the `DatagramIo` side that tasks use.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use fps_io::{DatagramIo, MAX_DATAGRAM};

use crate::arm::{Key, Shared, UdpArm};
use crate::lock;

/// What happened to a datagram that [`SessionQueue::push`] was given.
pub(crate) enum Push {
    Queued,
    Full,
    Closed,
}

struct QueueState {
    datagrams: VecDeque<Vec<u8>>,
    waker: Option<Waker>,
    closed: bool,
}

/// The datagrams that wait for the reader of one session.
pub(crate) struct SessionQueue {
    state: Mutex<QueueState>,
}

impl SessionQueue {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                datagrams: VecDeque::new(),
                waker: None,
                closed: false,
            }),
        }
    }

    /// Queues a copy of `payload` unless the queue holds `cap` datagrams or
    /// the session closed, and wakes the reader.
    pub(crate) fn push(&self, payload: &[u8], cap: usize) -> Push {
        let waker = {
            let mut s = lock(&self.state);
            if s.closed {
                return Push::Closed;
            }
            if s.datagrams.len() >= cap {
                return Push::Full;
            }
            s.datagrams.push_back(payload.to_vec());
            s.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Push::Queued
    }

    fn poll_pop(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<usize> {
        let mut s = lock(&self.state);
        if let Some(datagram) = s.datagrams.pop_front() {
            let n = datagram.len().min(buf.len());
            buf[..n].copy_from_slice(&datagram[..n]);
            return Poll::Ready(datagram.len());
        }
        s.waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn close(&self) {
        let mut s = lock(&self.state);
        s.closed = true;
        s.datagrams.clear();
        s.waker = None;
    }
}

/// One UDP session of the user space layer, on the client or the server side.
///
/// The session is connected: [`DatagramIo::poll_send`] sends to the peer and
/// [`DatagramIo::poll_recv`] returns only the datagrams of the peer, as
/// `KernelDatagram` of `fps-backend-kernel` does. A datagram that is longer
/// than the buffer of `poll_recv` is cut and the full length is returned. The
/// layer never reports ICMP, so a send does not fail because the peer's port
/// is closed. [`DatagramIo::close`] and dropping the session remove it from
/// the table of its arm; after `close`, every operation fails with
/// `NotConnected`.
pub struct UdpSession {
    shared: Arc<Shared>,
    queue: Arc<SessionQueue>,
    key: Key,
    /// The Ethernet header of the frames of this session.
    head: Vec<u8>,
    closed: bool,
}

impl std::fmt::Debug for UdpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpSession")
            .field("local", &self.key.local_addr())
            .field("peer", &self.key.peer_addr())
            .field("closed", &self.closed)
            .finish()
    }
}

impl UdpSession {
    pub(crate) fn new(
        shared: Arc<Shared>,
        queue: Arc<SessionQueue>,
        key: Key,
        head: Vec<u8>,
    ) -> Self {
        Self {
            shared,
            queue,
            key,
            head,
            closed: false,
        }
    }

    /// The local address and port of the session.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.key.local_addr()
    }

    /// The address and port of the peer.
    #[must_use]
    pub fn peer_addr(&self) -> SocketAddr {
        self.key.peer_addr()
    }

    fn release(&mut self) {
        if !self.closed {
            self.closed = true;
            self.shared.remove(&self.key, &self.queue);
            self.queue.close();
        }
    }
}

fn not_connected() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        "the user space UDP session was closed",
    )
}

impl DatagramIo for UdpSession {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        if self.closed {
            return Poll::Ready(Err(not_connected()));
        }
        if datagram.len() > MAX_DATAGRAM {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "datagram of {} bytes exceeds {MAX_DATAGRAM} bytes",
                    datagram.len()
                ),
            )));
        }
        self.shared.send(cx, &self.head, self.key, datagram)
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(not_connected()));
        }
        self.queue.poll_pop(cx, buf).map(Ok)
    }

    fn close(&mut self) {
        self.release();
    }
}

impl Drop for UdpSession {
    fn drop(&mut self) {
        self.release();
    }
}

struct AcceptState {
    ready: VecDeque<(UdpSession, SocketAddr)>,
    waker: Option<Waker>,
}

/// The sessions that wait for `accept`, shared by the arms of one listener.
pub(crate) struct AcceptQueue {
    cap: usize,
    state: Mutex<AcceptState>,
}

impl AcceptQueue {
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            state: Mutex::new(AcceptState {
                ready: VecDeque::new(),
                waker: None,
            }),
        }
    }

    /// Queues a session, or gives it back when the queue is full.
    pub(crate) fn push(&self, session: UdpSession, peer: SocketAddr) -> Result<(), UdpSession> {
        let waker = {
            let mut s = lock(&self.state);
            if s.ready.len() >= self.cap {
                return Err(session);
            }
            s.ready.push_back((session, peer));
            s.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }

    fn drain(&self) -> VecDeque<(UdpSession, SocketAddr)> {
        std::mem::take(&mut lock(&self.state).ready)
    }
}

/// A listener on one UDP port at every host of one or more arms.
///
/// [`UdpListener::accept`] waits for a datagram from a peer without an open
/// session and returns a [`UdpSession`] connected to that peer, with the
/// datagram queued for its first receive, as `KernelDatagramListener` does.
/// The session answers from the address that the peer sent to. After the
/// session closes, the next datagram of the peer opens a new one.
///
/// Dropping the listener frees the port and drops the sessions that were not
/// accepted yet.
pub struct UdpListener {
    port: u16,
    queue: Arc<AcceptQueue>,
    arms: Vec<UdpArm>,
}

impl std::fmt::Debug for UdpListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpListener")
            .field("port", &self.port)
            .field("arms", &self.arms.len())
            .finish_non_exhaustive()
    }
}

impl UdpListener {
    /// A listener on `port` that holds up to `accept_queue` sessions and
    /// listens on no arm yet. [`UdpListener::attach`] adds arms.
    #[must_use]
    pub fn new(port: u16, accept_queue: usize) -> Self {
        Self {
            port,
            queue: Arc::new(AcceptQueue::new(accept_queue)),
            arms: Vec::new(),
        }
    }

    /// A listener on `port` at every host of `arm`, with the accept queue
    /// that the settings of the arm give.
    ///
    /// # Errors
    ///
    /// See [`UdpListener::attach`].
    pub fn bind(arm: &UdpArm, port: u16) -> io::Result<Self> {
        let mut listener = Self::new(port, arm.shared.accept_queue());
        listener.attach(arm)?;
        Ok(listener)
    }

    /// Listens on the port at every host of `arm` too. Sessions of all arms
    /// come out of the same [`UdpListener::accept`].
    ///
    /// # Errors
    ///
    /// `InvalidInput` for port 0, and `AddrInUse` when the arm already has a
    /// listener on the port.
    pub fn attach(&mut self, arm: &UdpArm) -> io::Result<()> {
        arm.shared.listen(self.port, &self.queue)?;
        self.arms.push(arm.clone());
        Ok(())
    }

    /// The port of the listener.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Polls for the next session and the address of its peer.
    pub fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<(UdpSession, SocketAddr)>> {
        let mut s = lock(&self.queue.state);
        if let Some(next) = s.ready.pop_front() {
            return Poll::Ready(Ok(next));
        }
        s.waker = Some(cx.waker().clone());
        Poll::Pending
    }

    /// Completes with the next session and the address of its peer.
    ///
    /// # Errors
    ///
    /// Never fails today. The result type matches `KernelDatagramListener`.
    pub async fn accept(&mut self) -> io::Result<(UdpSession, SocketAddr)> {
        poll_fn(|cx| self.poll_accept(cx)).await
    }
}

impl Drop for UdpListener {
    fn drop(&mut self) {
        for arm in &self.arms {
            arm.shared.unlisten(self.port, &self.queue);
        }
        // The sessions that were never accepted leave their arms here, with
        // no lock held.
        drop(self.queue.drain());
    }
}
