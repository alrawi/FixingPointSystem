// SPDX-License-Identifier: GPL-3.0-only
//! The kernel backend as a [`Transport`] (ENG-31).
//!
//! TCP goes through [`fps_backend_kernel::Connector`] and
//! [`fps_backend_kernel::KernelListener`], so a worker over this transport
//! behaves as the workers did before the seam. A client UDP session is a
//! connected socket of its own, see
//! [`fps_backend_kernel::Connector::connect_datagram`].
//!
//! The UDP listener is the one of the runtime and not
//! [`fps_backend_kernel::KernelDatagramListener`]. One socket serves an
//! address, a reader sorts every datagram by its source address, and the
//! sessions send from the shared socket, so a reply carries the address that
//! the peer sent to. A datagram of a known peer goes to the queue of its
//! session, and the first datagram of a new peer opens a session that
//! [`DatagramListener::accept`] gives out. After a session closes, the next
//! datagram of its peer opens a new one.

use std::collections::HashMap;
use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use fps_backend_kernel::{
    BackendError, ConnectConfig, Connector, KernelListener, KernelStream, ListenConfig,
};
use fps_io::{DatagramIo, MAX_DATAGRAM};
use tokio::net::UdpSocket;

use crate::transport::{
    BoxFuture, DatagramListener, DatagramSession, StreamListener, StreamSession, Transport,
};

/// The number of datagrams that a server session queues. Datagrams beyond it
/// are dropped, as a full receive queue of the kernel drops them.
const SESSION_QUEUE: usize = 64;

/// The receive buffer of a port, which holds any UDP payload.
const PORT_BUFFER: usize = 65_536;

/// The kernel backend: TCP and UDP over the sockets of the operating system.
///
/// One value serves every session of a worker, or of several workers, because
/// it holds only configuration.
#[derive(Debug, Clone)]
pub struct KernelTransport {
    connector: Connector,
    listen: ListenConfig,
}

impl Default for KernelTransport {
    fn default() -> Self {
        Self::new(ConnectConfig::default(), ListenConfig::default())
    }
}

impl KernelTransport {
    /// Returns a transport whose connections take `connect` and whose
    /// listeners take `listen`.
    #[must_use]
    pub fn new(connect: ConnectConfig, listen: ListenConfig) -> Self {
        Self {
            connector: Connector::new(connect),
            listen,
        }
    }
}

/// Maps a backend error to an I/O error of the same kind that keeps the
/// backend error as its cause.
pub(crate) fn to_io(e: BackendError) -> io::Error {
    io::Error::new(e.io_kind().unwrap_or(io::ErrorKind::Other), e)
}

fn kernel_session(stream: KernelStream) -> StreamSession {
    let (local, peer) = (stream.local_addr().ok(), stream.peer_addr().ok());
    StreamSession::new(stream, local, peer)
}

impl Transport for KernelTransport {
    fn connect(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> BoxFuture<'_, io::Result<StreamSession>> {
        Box::pin(async move {
            self.connector
                .connect(source, destination)
                .await
                .map(kernel_session)
                .map_err(to_io)
        })
    }

    fn listen(&self, addresses: &[SocketAddr]) -> io::Result<Box<dyn StreamListener>> {
        let listeners = addresses
            .iter()
            .map(|a| KernelListener::bind(*a, &self.listen).map(Arc::new))
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_io)?;
        Ok(Box::new(KernelStreamListener::new(listeners)))
    }

    fn connect_datagram(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> io::Result<DatagramSession> {
        let io = self
            .connector
            .connect_datagram(source, destination)
            .map_err(to_io)?;
        let (local, peer) = (io.local_addr(), io.peer_addr());
        Ok(DatagramSession::new(io, local, peer))
    }

    fn listen_datagram(&self, addresses: &[SocketAddr]) -> io::Result<Box<dyn DatagramListener>> {
        match addresses {
            [address] => Ok(Box::new(SharedSocketListener::bind(*address)?)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the kernel UDP listener serves one address per call",
            )),
        }
    }
}

type Accepting = Pin<Box<dyn std::future::Future<Output = Result<KernelStream, BackendError>>>>;

fn arm(listener: &Arc<KernelListener>) -> Accepting {
    let listener = Arc::clone(listener);
    Box::pin(async move { listener.accept().await.map(|(stream, _peer)| stream) })
}

/// The kernel listeners of one `listen` call, which accept in turn.
struct KernelStreamListener {
    listeners: Vec<Arc<KernelListener>>,
    /// One pending accept per listener. A future that is not ready stays
    /// armed between two calls, which keeps `accept` cancel safe.
    pending: Vec<Accepting>,
    /// The listener that the next poll starts with, so that a busy listener
    /// does not starve the others.
    next: usize,
}

impl KernelStreamListener {
    fn new(listeners: Vec<Arc<KernelListener>>) -> Self {
        let pending = listeners.iter().map(arm).collect();
        Self {
            listeners,
            pending,
            next: 0,
        }
    }

    fn poll_accept(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<StreamSession>> {
        let n = self.pending.len();
        for k in 0..n {
            let i = (self.next + k) % n;
            let (Some(future), Some(listener)) = (self.pending.get_mut(i), self.listeners.get(i))
            else {
                continue;
            };
            if let Poll::Ready(accepted) = future.as_mut().poll(cx) {
                *future = arm(listener);
                self.next = (i + 1) % n;
                return Poll::Ready(accepted.map(kernel_session).map_err(to_io));
            }
        }
        Poll::Pending
    }
}

impl StreamListener for KernelStreamListener {
    fn local_addrs(&self) -> Vec<SocketAddr> {
        self.listeners.iter().map(|l| l.local_addr()).collect()
    }

    fn accept(&mut self) -> BoxFuture<'_, io::Result<StreamSession>> {
        Box::pin(poll_fn(move |cx| self.poll_accept(cx)))
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A critical section of this module only pushes, pops, or replaces whole
    // values, so a panic in another one leaves the data consistent.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "the udp session was closed")
}

#[derive(Default)]
struct InboxState {
    datagrams: std::collections::VecDeque<Vec<u8>>,
    waker: Option<Waker>,
}

/// The datagrams of one peer that the listener sorted to a session.
#[derive(Default)]
struct Inbox {
    state: Mutex<InboxState>,
}

impl Inbox {
    fn push(&self, datagram: Vec<u8>) {
        let waker = {
            let mut state = lock(&self.state);
            if state.datagrams.len() < SESSION_QUEUE {
                state.datagrams.push_back(datagram);
            }
            state.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

/// What a bound socket and its sessions share. The sessions are `Send`, so
/// this part is too, and its mutex is only ever taken by the thread of the
/// worker.
struct Shared {
    socket: UdpSocket,
    local: SocketAddr,
    sessions: Mutex<HashMap<SocketAddr, Arc<Inbox>>>,
}

/// One server side session: the datagrams of one peer. It receives from the
/// queue that the listener fills and sends from the shared socket to its peer.
struct ServerDatagram {
    shared: Arc<Shared>,
    peer: SocketAddr,
    inbox: Arc<Inbox>,
    closed: bool,
}

impl DatagramIo for ServerDatagram {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        if self.closed {
            return Poll::Ready(Err(closed()));
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
        self.shared
            .socket
            .poll_send_to(cx, datagram, self.peer)
            .map_ok(|_| ())
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(closed()));
        }
        let mut state = lock(&self.inbox.state);
        match state.datagrams.pop_front() {
            Some(d) => {
                let n = d.len().min(buf.len());
                if let (Some(to), Some(from)) = (buf.get_mut(..n), d.get(..n)) {
                    to.copy_from_slice(from);
                }
                Poll::Ready(Ok(d.len()))
            }
            None => {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    fn close(&mut self) {
        if std::mem::replace(&mut self.closed, true) {
            return;
        }
        {
            let mut sessions = lock(&self.shared.sessions);
            // A later session of the same peer may have replaced this entry.
            if sessions
                .get(&self.peer)
                .is_some_and(|i| Arc::ptr_eq(i, &self.inbox))
            {
                sessions.remove(&self.peer);
            }
        }
        lock(&self.inbox.state).datagrams.clear();
    }
}

impl Drop for ServerDatagram {
    fn drop(&mut self) {
        self.close();
    }
}

/// One bound UDP socket that opens a session for each peer.
struct SharedSocketListener {
    shared: Arc<Shared>,
    buf: Vec<u8>,
}

impl SharedSocketListener {
    fn bind(address: SocketAddr) -> io::Result<Self> {
        let socket = std::net::UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        let socket = UdpSocket::from_std(socket)?;
        let local = socket.local_addr()?;
        Ok(Self {
            shared: Arc::new(Shared {
                socket,
                local,
                sessions: Mutex::new(HashMap::new()),
            }),
            buf: vec![0; PORT_BUFFER],
        })
    }

    /// Sorts the datagram in the buffer to its session, or opens the session
    /// of a new peer and returns it.
    fn route(&self, from: SocketAddr, n: usize) -> Option<DatagramSession> {
        let datagram = self.buf.get(..n).unwrap_or(&self.buf).to_vec();
        let known = lock(&self.shared.sessions).get(&from).cloned();
        if let Some(inbox) = known {
            inbox.push(datagram);
            return None;
        }
        let inbox = Arc::new(Inbox::default());
        inbox.push(datagram);
        lock(&self.shared.sessions).insert(from, Arc::clone(&inbox));
        let session = ServerDatagram {
            shared: Arc::clone(&self.shared),
            peer: from,
            inbox,
            closed: false,
        };
        Some(DatagramSession::new(session, self.shared.local, from))
    }

    /// Reads datagrams until one opens a session.
    async fn read_until_session(&mut self) -> io::Result<DatagramSession> {
        loop {
            let (n, from) = self.shared.socket.recv_from(&mut self.buf).await?;
            if let Some(session) = self.route(from, n) {
                return Ok(session);
            }
        }
    }
}

impl DatagramListener for SharedSocketListener {
    fn local_addrs(&self) -> Vec<SocketAddr> {
        vec![self.shared.local]
    }

    fn accept(&mut self) -> BoxFuture<'_, io::Result<DatagramSession>> {
        Box::pin(self.read_until_session())
    }
}
