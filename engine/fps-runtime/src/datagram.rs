// SPDX-License-Identifier: GPL-3.0-only
//! The datagram sockets of the instances of one worker (ENG-26).
//!
//! A UDP flow of an instance opens in one of three ways.
//!
//! - A client side dials with [`fps_backend_kernel::Connector::connect_datagram`],
//!   which binds a socket of its own on a source address of the host range,
//!   takes a source port of the configured range, and connects the socket to
//!   the server. Every flow therefore has its own source port and its own
//!   5-tuple.
//! - A server side that is the entry flow takes the first session of a UDP
//!   port that the worker listens on, the way a TCP server worker takes an
//!   accepted connection.
//! - Any other server side accepts on the port that its program renders, for
//!   example the RTP port that a SIP answer announces. The socket binds when
//!   the flow starts and the port is free again when the last session of the
//!   port ends.
//!
//! All server sides of one port share one bound socket. A task per port reads
//! it and sorts every datagram by its source address: a datagram of a known
//! peer goes to the queue of that session, and the first datagram of a new
//! peer opens a session that waits for the instance that expects its source
//! host, see [`crate::rendezvous`]. Two instances that draw the same RTP port
//! therefore share the socket and each takes the sessions of its own host.
//!
//! The sessions send from the shared socket, so a reply carries the address
//! that the peer sent to. Bind the listen address of a worker to one host
//! address and not to the unspecified address when the host has several.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use fps_backend_kernel::KernelDatagram;
use fps_capture::{CaptureCell, Side as TapSide, TappedDatagram};
use fps_interp::{DatagramOpener, OpenKind, OpenRequest, Side};
use fps_io::{DatagramIo, MAX_DATAGRAM};
use tokio::net::UdpSocket;
use tokio::sync::Notify;
use tokio::time::{sleep, sleep_until, Instant};

use crate::instance::{OPEN_GRACE, RETRY_FIRST, RETRY_MAX};
use crate::opener::Endpoints;
use crate::rendezvous::Rendezvous;

/// The number of datagrams that a server session queues. Datagrams beyond it
/// are dropped, as a full receive queue of the kernel drops them.
const SESSION_QUEUE: usize = 64;

/// The receive buffer of a port, which holds any UDP payload.
const PORT_BUFFER: usize = 65_536;

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
    datagrams: VecDeque<Vec<u8>>,
    waker: Option<Waker>,
}

/// The datagrams of one peer that the port task sorted to a session.
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

/// What a bound port and its sessions share. The sessions are `Send`, so this
/// part of a port is too, and its mutex is only ever taken by the thread of
/// the worker.
struct Shared {
    socket: UdpSocket,
    local: SocketAddr,
    sessions: Mutex<HashMap<SocketAddr, Arc<Inbox>>>,
    /// Wakes the port task when a session closed.
    changed: Notify,
}

/// One server side session of a UDP port: the datagrams of one peer.
///
/// It receives from the queue that the port task fills and sends from the
/// shared socket to its peer.
pub(crate) struct ServerDatagram {
    shared: Arc<Shared>,
    peer: SocketAddr,
    inbox: Arc<Inbox>,
    closed: bool,
}

impl ServerDatagram {
    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.shared.local
    }

    pub(crate) fn peer_addr(&self) -> SocketAddr {
        self.peer
    }
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
        self.shared.changed.notify_one();
    }
}

impl Drop for ServerDatagram {
    fn drop(&mut self) {
        self.close();
    }
}

/// A UDP session of an instance on either side.
pub(crate) enum RuntimeDatagram {
    /// A connected socket of its own.
    Client(KernelDatagram),
    /// A peer of a shared port.
    Server(ServerDatagram),
}

impl DatagramIo for RuntimeDatagram {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        match self {
            Self::Client(io) => io.poll_send(cx, datagram),
            Self::Server(io) => io.poll_send(cx, datagram),
        }
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self {
            Self::Client(io) => io.poll_recv(cx, buf),
            Self::Server(io) => io.poll_recv(cx, buf),
        }
    }

    fn close(&mut self) {
        match self {
            Self::Client(io) => io.close(),
            Self::Server(io) => io.close(),
        }
    }
}

impl Endpoints for RuntimeDatagram {
    fn endpoints(&self) -> Option<(SocketAddr, SocketAddr)> {
        Some(match self {
            Self::Client(io) => (io.local_addr(), io.peer_addr()),
            Self::Server(io) => (io.local_addr(), io.peer_addr()),
        })
    }
}

/// The ports that the server sides of one worker listen on.
#[derive(Clone)]
pub(crate) struct DatagramPorts {
    inner: Rc<Inner>,
}

struct Inner {
    ports: RefCell<HashMap<SocketAddr, Rc<Port>>>,
}

struct Port {
    shared: Arc<Shared>,
    waiting: RefCell<Rendezvous<ServerDatagram>>,
    /// The number of [`Listening`] guards, which keep a port open while no
    /// session and no waiter holds it.
    pins: Cell<usize>,
    changed: Notify,
}

impl Port {
    fn is_idle(&self) -> bool {
        self.pins.get() == 0
            && self.waiting.borrow().is_idle()
            && lock(&self.shared.sessions).is_empty()
    }

    /// Sorts one datagram to its session, or opens the session of a new peer.
    fn route(&self, from: SocketAddr, datagram: &[u8]) {
        let known = lock(&self.shared.sessions).get(&from).cloned();
        if let Some(inbox) = known {
            inbox.push(datagram.to_vec());
            return;
        }
        let inbox = Arc::new(Inbox::default());
        inbox.push(datagram.to_vec());
        lock(&self.shared.sessions).insert(from, Arc::clone(&inbox));
        let session = ServerDatagram {
            shared: Arc::clone(&self.shared),
            peer: from,
            inbox,
            closed: false,
        };
        self.waiting
            .borrow_mut()
            .offer(from.ip(), session, Instant::now());
    }
}

/// Keeps the port of a worker open and bound, see [`DatagramPorts::listen`].
pub(crate) struct Listening {
    port: Rc<Port>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.port.pins.set(self.port.pins.get().saturating_sub(1));
        self.port.changed.notify_one();
    }
}

/// Removes a waiter when its `accept` ends, however it ends.
struct Registered<'a> {
    port: &'a Port,
    id: u64,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        self.port.waiting.borrow_mut().cancel(self.id);
        self.port.changed.notify_one();
    }
}

impl DatagramPorts {
    /// Returns the ports of a worker, none of them bound yet.
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(Inner {
                ports: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// Returns the port at `addr`, bound and served by a task, and binds it
    /// when no instance of the worker holds it. A bind that finds the port in
    /// use is retried for [`OPEN_GRACE`], because a socket of another process
    /// may still hold it.
    async fn port(&self, addr: SocketAddr) -> io::Result<Rc<Port>> {
        let deadline = Instant::now() + OPEN_GRACE;
        let mut wait = RETRY_FIRST;
        loop {
            // A port 0 asks for a socket of its own, so only a real port is shared.
            let held = (addr.port() != 0)
                .then(|| self.inner.ports.borrow().get(&addr).map(Rc::clone))
                .flatten();
            if let Some(port) = held {
                return Ok(port);
            }
            match UdpSocket::bind(addr).await {
                Ok(socket) => {
                    let local = socket.local_addr()?;
                    let port = Rc::new(Port {
                        shared: Arc::new(Shared {
                            socket,
                            local,
                            sessions: Mutex::new(HashMap::new()),
                            changed: Notify::new(),
                        }),
                        waiting: RefCell::new(Rendezvous::new()),
                        pins: Cell::new(0),
                        changed: Notify::new(),
                    });
                    self.inner.ports.borrow_mut().insert(local, Rc::clone(&port));
                    tokio::task::spawn_local(serve(Rc::clone(&self.inner), Rc::clone(&port)));
                    return Ok(port);
                }
                Err(e) if e.kind() == io::ErrorKind::AddrInUse && Instant::now() + wait < deadline => {
                    sleep(wait).await;
                    wait = (wait * 2).min(RETRY_MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Binds `addr` for a server worker and keeps it bound until the guard
    /// drops. The server sides of its flows take the sessions with
    /// [`DatagramPorts::accept`]. Returns the address that the socket bound,
    /// which differs from `addr` when `addr` has port 0.
    pub(crate) async fn listen(&self, addr: SocketAddr) -> io::Result<(Listening, SocketAddr)> {
        let port = self.port(addr).await?;
        port.pins.set(port.pins.get() + 1);
        let local = port.shared.local;
        Ok((Listening { port }, local))
    }

    /// Returns the session that `expect` opens on the port at `addr`, or the
    /// first session of any peer when `expect` is `None`. The call waits for
    /// a datagram, so a peer that never sends leaves it waiting until the
    /// caller gives up.
    pub(crate) async fn accept(
        &self,
        addr: SocketAddr,
        expect: Option<IpAddr>,
    ) -> io::Result<ServerDatagram> {
        let port = self.port(addr).await?;
        let (id, rx) = port.waiting.borrow_mut().register(expect);
        // A session that already waited completes the receiver now.
        port.changed.notify_one();
        let _registered = Registered { port: &port, id };
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::ConnectionAborted, "the udp port closed"))
    }
}

/// What woke the task of a port.
enum Event {
    Datagram(usize, SocketAddr),
    Failed,
    Woken,
}

/// Reads the socket of one port and sorts its datagrams, until the port has
/// no pin, no waiter, no waiting session, and no open session.
async fn serve(inner: Rc<Inner>, port: Rc<Port>) {
    let mut buf = vec![0u8; PORT_BUFFER];
    loop {
        let deadline = {
            let mut waiting = port.waiting.borrow_mut();
            waiting.expire(Instant::now());
            // Dropping an expired session closes it, which empties the map.
            drop(waiting);
            if port.is_idle() {
                // The entry leaves the map in the same step as the socket
                // leaves this task, so a later accept binds the port again.
                inner.ports.borrow_mut().retain(|_, p| !Rc::ptr_eq(p, &port));
                return;
            }
            port.waiting.borrow().next_deadline()
        };
        let event = tokio::select! {
            received = port.shared.socket.recv_from(&mut buf) => match received {
                Ok((n, from)) => Event::Datagram(n, from),
                Err(_) => Event::Failed,
            },
            () = port.changed.notified() => Event::Woken,
            () = port.shared.changed.notified() => Event::Woken,
            () = async {
                match deadline {
                    Some(d) => sleep_until(d).await,
                    None => std::future::pending().await,
                }
            } => Event::Woken,
        };
        match event {
            Event::Datagram(n, from) => port.route(from, buf.get(..n).unwrap_or(&buf)),
            // One failed receive leaves the socket usable.
            Event::Failed => tokio::task::yield_now().await,
            Event::Woken => {}
        }
    }
}

/// What a datagram flow side does when it opens, decided before the future
/// starts.
pub(crate) enum DatagramNet {
    /// Dials the server from a socket of its own.
    Connect {
        ctx: Rc<crate::worker::ClientCtx>,
        source: Result<IpAddr, String>,
        session: u64,
        entry: bool,
    },
    /// Hands out the session that the worker accepted for the entry flow.
    Accepted(Option<ServerDatagram>),
    /// Accepts on the port that the program renders.
    Bind {
        ip: IpAddr,
        ports: DatagramPorts,
        expect: Option<IpAddr>,
    },
}

/// Opens the UDP session of one flow side.
pub(crate) struct DatagramFlowOpener {
    pub(crate) kind: DatagramNet,
    pub(crate) ended: Rc<Cell<Option<Instant>>>,
}

impl Drop for DatagramFlowOpener {
    fn drop(&mut self) {
        if self.ended.get().is_none() {
            self.ended.set(Some(Instant::now()));
        }
    }
}

enum Plan {
    Connect {
        ctx: Rc<crate::worker::ClientCtx>,
        source: IpAddr,
        to: SocketAddr,
    },
    Accepted(Option<ServerDatagram>),
    Bind {
        addr: SocketAddr,
        ports: DatagramPorts,
        expect: Option<IpAddr>,
    },
    Fail(io::Error),
}

impl DatagramOpener for DatagramFlowOpener {
    type Io = RuntimeDatagram;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<RuntimeDatagram>> {
        let plan = match (&mut self.kind, request.kind) {
            (
                DatagramNet::Connect {
                    ctx,
                    source,
                    session,
                    entry,
                },
                OpenKind::Connect,
            ) => {
                // The override replaces the port of the entry connect only,
                // because a media flow goes to the port that its flow renders.
                let port = if *entry { ctx.config.port } else { None };
                match (
                    source.clone().map_err(crate::opener::invalid),
                    crate::opener::destination(&ctx.config.servers, *session, port, &request),
                ) {
                    (Ok(source), Ok(to)) => Plan::Connect {
                        ctx: Rc::clone(ctx),
                        source,
                        to,
                    },
                    (Err(e), _) | (_, Err(e)) => Plan::Fail(e),
                }
            }
            (DatagramNet::Accepted(session), OpenKind::Accept) => Plan::Accepted(session.take()),
            (
                DatagramNet::Bind {
                    ip,
                    ports,
                    expect,
                },
                OpenKind::Accept,
            ) => Plan::Bind {
                addr: SocketAddr::new(*ip, request.port),
                ports: ports.clone(),
                expect: *expect,
            },
            (_, OpenKind::Connect) => Plan::Fail(crate::opener::invalid(
                "a server opener cannot connect".into(),
            )),
            (_, OpenKind::Accept) => Plan::Fail(crate::opener::invalid(
                "a client opener cannot accept".into(),
            )),
        };
        async move {
            match plan {
                Plan::Fail(e) => Err(e),
                Plan::Accepted(session) => session
                    .map(RuntimeDatagram::Server)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "session used")),
                Plan::Connect { ctx, source, to } => ctx
                    .connector
                    .connect_datagram(source, to)
                    .map(RuntimeDatagram::Client)
                    .map_err(|e| io::Error::new(e.io_kind().unwrap_or(io::ErrorKind::Other), e)),
                Plan::Bind {
                    addr,
                    ports,
                    expect,
                } => ports
                    .accept(addr, expect)
                    .await
                    .map(RuntimeDatagram::Server),
            }
        }
    }
}

/// Wraps the sessions that a datagram opener opens in the tap of the current
/// capture, as [`crate::opener::TapOpener`] does for streams.
pub(crate) struct TapDatagramOpener<O> {
    inner: O,
    cell: CaptureCell,
    session_id: u64,
    side: Side,
}

impl<O> TapDatagramOpener<O> {
    pub(crate) fn new(inner: O, cell: CaptureCell, session_id: u64, side: Side) -> Self {
        Self {
            inner,
            cell,
            session_id,
            side,
        }
    }
}

impl<O: DatagramOpener<Io = RuntimeDatagram>> DatagramOpener for TapDatagramOpener<O> {
    type Io = TappedDatagram<RuntimeDatagram>;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<TappedDatagram<RuntimeDatagram>>> {
        let (cell, id, side) = (self.cell.clone(), self.session_id, self.side);
        let opened = self.inner.open(request);
        async move {
            let io = opened.await?;
            let side = match side {
                Side::Client => TapSide::Client,
                Side::Server => TapSide::Server,
            };
            Ok(match io.endpoints() {
                Some((local, peer)) => cell.tap_datagram(id, side, local, peer, io),
                None => TappedDatagram::new(io, None),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use fps_io::DatagramIoExt;
    use tokio::task::LocalSet;

    use super::*;

    fn lo(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    async fn peer(from: u8) -> UdpSocket {
        UdpSocket::bind(SocketAddr::from(([127, 0, 0, from], 0)))
            .await
            .expect("a source address")
    }

    async fn first(session: &mut ServerDatagram) -> Vec<u8> {
        let mut buf = vec![0u8; 64];
        let n = session.recv_datagram(&mut buf).await.expect("a datagram");
        buf.truncate(n);
        buf
    }

    #[tokio::test(flavor = "current_thread")]
    async fn each_instance_gets_the_peer_of_its_host_on_a_shared_port() {
        LocalSet::new()
            .run_until(async {
                let ports = DatagramPorts::new();
                let (_pin, addr) = ports.listen(lo(0)).await.expect("bound");
                let one = tokio::task::spawn_local({
                    let ports = ports.clone();
                    async move { ports.accept(addr, Some(IpAddr::from([127, 0, 0, 2]))).await }
                });
                let two = tokio::task::spawn_local({
                    let ports = ports.clone();
                    async move { ports.accept(addr, Some(IpAddr::from([127, 0, 0, 3]))).await }
                });
                tokio::task::yield_now().await;
                let (a, b) = (peer(2).await, peer(3).await);
                b.send_to(b"from b", addr).await.expect("sent");
                a.send_to(b"from a", addr).await.expect("sent");
                let mut one = one.await.expect("task").expect("first");
                let mut two = two.await.expect("task").expect("second");
                assert_eq!(first(&mut one).await, b"from a");
                assert_eq!(first(&mut two).await, b"from b");
                // The sessions answer their own peers and keep their own queues.
                one.send_datagram(b"to a").await.expect("sent");
                two.send_datagram(b"to b").await.expect("sent");
                let mut buf = [0u8; 16];
                let (n, from) = a.recv_from(&mut buf).await.expect("an answer");
                assert_eq!((&buf[..n], from), (&b"to a"[..], addr));
                let (n, _) = b.recv_from(&mut buf).await.expect("an answer");
                assert_eq!(&buf[..n], b"to b");
                a.send_to(b"again a", addr).await.expect("sent");
                b.send_to(b"again b", addr).await.expect("sent");
                assert_eq!(first(&mut one).await, b"again a");
                assert_eq!(first(&mut two).await, b"again b");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_datagram_that_comes_before_its_waiter_waits_for_it() {
        LocalSet::new()
            .run_until(async {
                let ports = DatagramPorts::new();
                let (_pin, addr) = ports.listen(lo(0)).await.expect("bound");
                let a = peer(2).await;
                a.send_to(b"one", addr).await.expect("sent");
                a.send_to(b"two", addr).await.expect("sent");
                tokio::time::sleep(Duration::from_millis(20)).await;
                let mut s = ports
                    .accept(addr, Some(IpAddr::from([127, 0, 0, 2])))
                    .await
                    .expect("the early session");
                assert_eq!(first(&mut s).await, b"one");
                assert_eq!(first(&mut s).await, b"two");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_port_is_free_when_its_last_session_ends() {
        LocalSet::new()
            .run_until(async {
                let ports = DatagramPorts::new();
                let probe = std::net::UdpSocket::bind("127.0.0.1:0").expect("a free port");
                let addr = probe.local_addr().expect("an address");
                drop(probe);
                let accept = tokio::task::spawn_local({
                    let ports = ports.clone();
                    async move { ports.accept(addr, None).await }
                });
                tokio::task::yield_now().await;
                assert!(std::net::UdpSocket::bind(addr).is_err(), "the port is bound");
                peer(2).await.send_to(b"hi", addr).await.expect("sent");
                let mut session = accept.await.expect("task").expect("a session");
                assert_eq!(first(&mut session).await, b"hi");
                assert!(std::net::UdpSocket::bind(addr).is_err(), "a session holds it");
                drop(session);
                tokio::time::sleep(Duration::from_millis(20)).await;
                std::net::UdpSocket::bind(addr).expect("the port is free");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_pinned_port_stays_bound_between_sessions() {
        LocalSet::new()
            .run_until(async {
                let ports = DatagramPorts::new();
                let (pin, addr) = ports.listen(lo(0)).await.expect("bound");
                let accept = ports.accept(addr, None);
                peer(2).await.send_to(b"a", addr).await.expect("sent");
                drop(accept.await.expect("a session"));
                tokio::time::sleep(Duration::from_millis(20)).await;
                assert!(std::net::UdpSocket::bind(addr).is_err(), "still bound");
                drop(pin);
                tokio::time::sleep(Duration::from_millis(20)).await;
                std::net::UdpSocket::bind(addr).expect("released");
            })
            .await;
    }
}
