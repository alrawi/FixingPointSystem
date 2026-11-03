// SPDX-License-Identifier: GPL-3.0-only
//! UDP sessions over kernel datagram sockets.
//!
//! A client session is a UDP socket bound to a pool source address and
//! connected to the peer, so the kernel filters out datagrams from other
//! addresses. A server binds one [`KernelDatagramListener`] per pool address.
//! The listener reads the first datagram of each new peer, opens a second
//! socket on the same address and port with `SO_REUSEPORT`, connects it to
//! the peer, and hands it out with that first datagram queued in user space.
//! Linux delivers every later datagram of the peer to the connected socket,
//! because a connected socket outranks the unconnected listener in the
//! lookup of a reuse port group.
//!
//! Between its `bind` and its `connect` a session socket is an unconnected
//! member of the group, and Linux may deliver the first datagram of another
//! peer to it. Such a datagram stays queued after the connect. The session
//! therefore receives with the source address, hands every datagram of
//! another peer back to its listener as a stray, and drains its queue the
//! same way right after the connect and again on close, so the listener
//! still opens a session for that peer.

use std::collections::{HashMap, VecDeque};
use std::future::{poll_fn, Future};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Waker};

use fps_interp::{DatagramOpener, OpenKind, OpenRequest};
use fps_io::{DatagramIo, MAX_DATAGRAM};
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::io::{Interest, ReadBuf, Ready};
use tokio::net::UdpSocket;

use crate::{sys, BackendError, Connector, SocketOptions};

/// The number of datagrams that a server session queues in user space.
///
/// The queue holds the first datagram of the session and the datagrams of
/// the peer that reach the listener or the unconnected session socket before
/// the connect takes effect. Datagrams beyond it are dropped, as a full
/// kernel receive queue drops them.
pub const SESSION_QUEUE: usize = 64;

/// The number of datagrams of new peers that a listener holds in user
/// space, from its read ahead and from strays that its sessions hand back.
/// Datagrams beyond it are dropped.
const LISTENER_PENDING: usize = 1024;

/// The receive buffer of a listener, which holds any UDP payload over IPv4
/// and any IPv6 payload without jumbograms.
const LISTENER_BUFFER: usize = 65_536;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding a lock of this module leaves the queue
    // consistent, because every critical section only pushes, pops, or
    // replaces whole values.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The datagrams that a listener hands to one server session.
#[derive(Debug, Default)]
struct Inbox {
    state: Mutex<InboxState>,
}

#[derive(Debug, Default)]
struct InboxState {
    datagrams: VecDeque<Vec<u8>>,
    waker: Option<Waker>,
    closed: bool,
}

impl Inbox {
    /// Queues `datagram` for the session, or hands it back when the session
    /// was closed, so the listener opens a new session for the peer.
    fn push(&self, datagram: Vec<u8>) -> Result<(), Vec<u8>> {
        let waker = {
            let mut state = lock(&self.state);
            if state.closed {
                return Err(datagram);
            }
            if state.datagrams.len() < SESSION_QUEUE {
                state.datagrams.push_back(datagram);
            }
            state.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
        Ok(())
    }
}

/// Datagrams of other peers that server session sockets received, which
/// wait for the next `accept` of their listener.
#[derive(Debug, Default)]
struct Strays {
    state: Mutex<StrayState>,
}

#[derive(Debug, Default)]
struct StrayState {
    datagrams: VecDeque<(SocketAddr, Vec<u8>)>,
    waker: Option<Waker>,
}

impl Strays {
    fn push(&self, from: SocketAddr, datagram: Vec<u8>) {
        let waker = {
            let mut state = lock(&self.state);
            if state.datagrams.len() < LISTENER_PENDING {
                state.datagrams.push_back((from, datagram));
            }
            state.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

/// The user space queues of a server session.
#[derive(Debug)]
struct Server {
    inbox: Arc<Inbox>,
    strays: Arc<Strays>,
}

/// Receives one datagram of `peer` from a server session socket and hands
/// datagrams of other peers to `strays`.
///
/// A buffer that holds any datagram takes the datagram in one call. A
/// smaller buffer could cut a stray short, so the call first peeks at the
/// source and the length.
fn recv_server(
    sock: &Socket,
    buf: &mut [u8],
    peer: SocketAddr,
    strays: &Strays,
) -> io::Result<usize> {
    loop {
        if buf.len() >= LISTENER_BUFFER {
            let (n, from) = sys::recv_from(sock, buf, 0)?;
            if from == peer {
                return Ok(n);
            }
            strays.push(from, buf.get(..n).unwrap_or(buf).to_vec());
        } else {
            let (_, from) = sys::recv_from(sock, &mut [], libc::MSG_PEEK)?;
            if from == peer {
                return sys::recv_from(sock, buf, 0).map(|(n, _)| n);
            }
            let (d, from) = recv_owned(sock)?;
            strays.push(from, d);
        }
    }
}

/// Receives the next datagram into a buffer of its own size.
fn recv_owned(sock: &Socket) -> io::Result<(Vec<u8>, SocketAddr)> {
    let (n, _) = sys::recv_from(sock, &mut [], libc::MSG_PEEK)?;
    let mut d = vec![0; n];
    let (m, from) = sys::recv_from(sock, &mut d, 0)?;
    d.truncate(m);
    Ok((d, from))
}

/// Moves every datagram that waits on a server session socket to the
/// inbox when it comes from `peer` and to `strays` otherwise.
fn drain_session(sock: &Socket, peer: SocketAddr, server: &Server) {
    while let Ok((d, from)) = recv_owned(sock) {
        if from == peer {
            // A closed session drops the datagrams of its own peer.
            let _ = server.inbox.push(d);
        } else {
            server.strays.push(from, d);
        }
    }
}

/// One kernel UDP session, on the client or the server side.
///
/// The socket is connected to the peer, so [`DatagramIo::poll_send`] sends
/// to the peer and [`DatagramIo::poll_recv`] returns only its datagrams.
/// Receives use `MSG_TRUNC`, so an oversized datagram returns its full
/// length, as [`DatagramIo`] requires. A connected socket reports ICMP port
/// unreachable from the peer as [`io::ErrorKind::ConnectionRefused`] on the
/// next send or receive. [`DatagramIo::close`] and dropping the session close
/// the socket, and every operation after `close` fails with
/// [`io::ErrorKind::NotConnected`].
pub struct KernelDatagram {
    /// The socket, or `None` after `close`.
    inner: Option<Arc<UdpSocket>>,
    /// Waits for the error readiness of the socket while a receive waits.
    ///
    /// The poll path of Tokio wakes a reader only on read readiness, but
    /// Linux signals ICMP port unreachable on a connected UDP socket with
    /// `EPOLLERR` alone, so a receive also waits on this future, which holds
    /// a second reference to the socket.
    error_watch: Option<ErrorWatch>,
    /// The user space queues of a server session.
    server: Option<Server>,
    local: SocketAddr,
    peer: SocketAddr,
}

type ErrorWatch = Pin<Box<dyn Future<Output = io::Result<Ready>> + Send>>;

impl std::fmt::Debug for KernelDatagram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelDatagram")
            .field("inner", &self.inner)
            .field("server", &self.server)
            .field("local", &self.local)
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl KernelDatagram {
    fn new(socket: UdpSocket, server: Option<Server>, local: SocketAddr, peer: SocketAddr) -> Self {
        Self {
            inner: Some(Arc::new(socket)),
            error_watch: None,
            server,
            local,
            peer,
        }
    }

    /// Returns the local address and port of the session.
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Returns the address and port of the peer.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    /// Returns the socket for inspection, or `None` after `close`.
    pub fn udp_socket(&self) -> Option<&UdpSocket> {
        self.inner.as_deref()
    }
}

fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        "the kernel UDP session was closed",
    )
}

impl DatagramIo for KernelDatagram {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        let socket = self.inner.as_ref().ok_or_else(closed)?;
        if datagram.len() > MAX_DATAGRAM {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "datagram of {} bytes exceeds {MAX_DATAGRAM} bytes",
                    datagram.len()
                ),
            )));
        }
        // A UDP send transfers the whole datagram or fails, so the count
        // carries no information.
        socket.poll_send(cx, datagram).map_ok(|_| ())
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        let socket = self.inner.as_ref().ok_or_else(closed)?;
        if let Some(server) = &self.server {
            let mut state = lock(&server.inbox.state);
            if let Some(d) = state.datagrams.pop_front() {
                let n = d.len().min(buf.len());
                buf[..n].copy_from_slice(&d[..n]);
                return Poll::Ready(Ok(d.len()));
            }
            state.waker = Some(cx.waker().clone());
        }
        let peer = self.peer;
        loop {
            // `try_io` takes one interest, because Tokio masks a combined
            // interest to nothing, so the receive names the readiness that
            // woke it.
            let interest = if socket.poll_recv_ready(cx)?.is_ready() {
                Interest::READABLE
            } else {
                let watch = self.error_watch.get_or_insert_with(|| {
                    let socket = Arc::clone(socket);
                    Box::pin(async move { socket.ready(Interest::ERROR).await })
                });
                std::task::ready!(watch.as_mut().poll(cx))?;
                self.error_watch = None;
                Interest::ERROR
            };
            let received = socket.try_io(interest, || {
                let sock = SockRef::from(socket.as_ref());
                match &self.server {
                    Some(server) => recv_server(&sock, buf, peer, &server.strays),
                    None => sys::recv_from(&sock, buf, 0).map(|(n, _)| n),
                }
            });
            match received {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                other => return Poll::Ready(other),
            }
        }
    }

    fn close(&mut self) {
        self.error_watch = None;
        let socket = self.inner.take();
        if let Some(server) = &self.server {
            {
                let mut state = lock(&server.inbox.state);
                state.closed = true;
                state.datagrams.clear();
                state.waker = None;
            }
            if let Some(socket) = &socket {
                drain_session(&SockRef::from(socket.as_ref()), self.peer, server);
            }
        }
    }
}

impl Drop for KernelDatagram {
    fn drop(&mut self) {
        self.close();
    }
}

/// Creates a nonblocking UDP socket for `address` with the options applied.
fn udp_socket(
    address: SocketAddr,
    options: &SocketOptions,
    reuse_port: bool,
) -> Result<Socket, (&'static str, io::Error)> {
    let sock = Socket::new(
        Domain::for_address(address),
        Type::DGRAM,
        Some(Protocol::UDP),
    )
    .map_err(|e| ("socket", e))?;
    if address.is_ipv6() {
        sock.set_only_v6(true).map_err(|e| ("set IPV6_V6ONLY", e))?;
    }
    if reuse_port {
        sock.set_reuse_address(true)
            .map_err(|e| ("set SO_REUSEADDR", e))?;
        sock.set_reuse_port(true)
            .map_err(|e| ("set SO_REUSEPORT", e))?;
    }
    options.apply_datagram(SockRef::from(&sock))?;
    sock.set_nonblocking(true)
        .map_err(|e| ("set O_NONBLOCK", e))?;
    Ok(sock)
}

fn register(sock: Socket) -> Result<UdpSocket, (&'static str, io::Error)> {
    UdpSocket::from_std(std::net::UdpSocket::from(sock)).map_err(|e| ("register", e))
}

impl Connector {
    /// Opens a UDP session from `source` to `destination`.
    ///
    /// The socket binds `source` with a port that the kernel picks from the
    /// configured [`crate::PortRange`], connects to `destination`, and takes
    /// the buffer sizes and the TTL of the [`SocketOptions`] of the
    /// connector. Connecting a UDP socket sends nothing, so the connect
    /// timeout does not apply. The source address must belong to the address
    /// family of the destination and must be local to the host. An exhausted
    /// port range fails with [`std::io::ErrorKind::AddrNotAvailable`] or
    /// [`std::io::ErrorKind::AddrInUse`]. The call must run inside a Tokio
    /// runtime with I/O enabled.
    pub fn connect_datagram(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> Result<KernelDatagram, BackendError> {
        let fail = |(op, e): (&'static str, io::Error)| BackendError::Connect {
            source_addr: source,
            destination,
            op,
            source: e,
        };
        if source.is_ipv4() != destination.is_ipv4() {
            return Err(BackendError::AddressFamily {
                source_addr: source,
                destination,
            });
        }
        let sock = udp_socket(destination, &self.config().socket, false).map_err(fail)?;
        if let Some(range) = self.config().local_ports {
            sys::set_local_port_range(&sock, range)
                .map_err(|e| fail(("set IP_LOCAL_PORT_RANGE", e)))?;
        }
        sock.bind(&SocketAddr::new(source, 0).into())
            .map_err(|e| fail(("bind", e)))?;
        sock.connect(&destination.into())
            .map_err(|e| fail(("connect", e)))?;
        let local = local_addr(&sock).map_err(fail)?;
        let socket = register(sock).map_err(fail)?;
        Ok(KernelDatagram::new(socket, None, local, destination))
    }
}

fn local_addr(sock: &Socket) -> Result<SocketAddr, (&'static str, io::Error)> {
    sock.local_addr()
        .map_err(|e| ("getsockname", e))?
        .as_socket()
        .ok_or_else(|| {
            (
                "getsockname",
                io::Error::new(io::ErrorKind::InvalidData, "not an IP address"),
            )
        })
}
/// A bound kernel UDP socket on one pool address that opens server sessions.
///
/// [`KernelDatagramListener::accept`] waits for a datagram from a peer
/// without an open session and returns a [`KernelDatagram`] connected to
/// that peer, with the datagram queued for its first receive. A datagram
/// from a peer whose session is still open joins the queue of that session,
/// which happens only when it raced the connect of the session socket. Each
/// `accept` moves such datagrams to their sessions right after it connects
/// the new socket, so they arrive without a further `accept`. After the
/// session closes, the next datagram of the peer opens a new session.
///
/// Every socket of the listener sets `SO_REUSEADDR` and `SO_REUSEPORT`, so
/// another listener of the same user on the same address and port shares
/// its datagrams. Bind a specific pool address and not the unspecified
/// address, because the session socket replies from the address that the
/// listener binds.
#[derive(Debug)]
pub struct KernelDatagramListener {
    inner: UdpSocket,
    address: SocketAddr,
    options: SocketOptions,
    sessions: HashMap<SocketAddr, Weak<Inbox>>,
    /// The session count at which `accept` drops the entries of closed sessions.
    prune_at: usize,
    /// Datagrams of new peers that the read ahead of `accept` took.
    pending: VecDeque<(SocketAddr, Vec<u8>)>,
    /// Datagrams of new peers that session sockets handed back.
    strays: Arc<Strays>,
    buf: Vec<u8>,
}

impl KernelDatagramListener {
    /// Binds `address` with the buffer sizes and the TTL of `options`, which
    /// every session socket of the listener also takes.
    ///
    /// Port 0 lets the kernel pick a port, which
    /// [`KernelDatagramListener::local_addr`] then reports. An IPv6 listener
    /// receives only IPv6 datagrams. The call must run inside a Tokio runtime
    /// with I/O enabled.
    pub fn bind(address: SocketAddr, options: &SocketOptions) -> Result<Self, BackendError> {
        let fail = |(op, e): (&'static str, io::Error)| BackendError::Listen {
            address,
            op,
            source: e,
        };
        let sock = udp_socket(address, options, true).map_err(fail)?;
        sock.bind(&address.into()).map_err(|e| fail(("bind", e)))?;
        let bound = local_addr(&sock).map_err(fail)?;
        let inner = register(sock).map_err(fail)?;
        Ok(Self {
            inner,
            address: bound,
            options: options.clone(),
            sessions: HashMap::new(),
            prune_at: 64,
            pending: VecDeque::new(),
            strays: Arc::default(),
            buf: vec![0; LISTENER_BUFFER],
        })
    }

    /// Returns the bound address and port.
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    /// Returns the number of sessions of the listener that are still open.
    pub fn open_sessions(&self) -> usize {
        self.sessions
            .values()
            .filter_map(Weak::upgrade)
            .filter(|i| !lock(&i.state).closed)
            .count()
    }

    /// Waits for the first datagram of a new peer and returns its session
    /// with the peer address.
    ///
    /// The future is cancel safe, so a timeout around it loses no datagram.
    /// An error concerns one datagram or one session, for example `EMFILE`,
    /// and the listener stays usable.
    pub async fn accept(&mut self) -> Result<(KernelDatagram, SocketAddr), BackendError> {
        loop {
            let (peer, datagram) =
                poll_fn(|cx| self.poll_next(cx))
                    .await
                    .map_err(|source| BackendError::Accept {
                        address: self.address,
                        source,
                    })?;
            let Some(datagram) = self.route(peer, datagram) else {
                continue;
            };
            let session = self.open_session(peer, datagram)?;
            self.drain();
            return Ok((session, peer));
        }
    }

    /// Returns the next datagram from the read ahead, the strays, or the
    /// socket, in that order.
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<(SocketAddr, Vec<u8>)>> {
        if let Some(next) = self.pending.pop_front() {
            return Poll::Ready(Ok(next));
        }
        {
            let mut strays = lock(&self.strays.state);
            if let Some(next) = strays.datagrams.pop_front() {
                return Poll::Ready(Ok(next));
            }
            strays.waker = Some(cx.waker().clone());
        }
        let mut buf = ReadBuf::new(&mut self.buf);
        let peer = std::task::ready!(self.inner.poll_recv_from(cx, &mut buf))?;
        Poll::Ready(Ok((peer, buf.filled().to_vec())))
    }

    /// Queues `datagram` on the open session of `peer`, or hands it back
    /// when the peer has none.
    fn route(&self, peer: SocketAddr, datagram: Vec<u8>) -> Option<Vec<u8>> {
        match self.sessions.get(&peer).and_then(Weak::upgrade) {
            Some(inbox) => inbox.push(datagram).err(),
            None => Some(datagram),
        }
    }

    /// Moves the datagrams that wait on the listener socket to their open
    /// sessions, and those of new peers to the read ahead of the next
    /// `accept`.
    ///
    /// The drain runs after a session socket connects, so datagrams of the
    /// peer that raced the connect reach the session without another
    /// `accept`. It stops when the read ahead holds [`LISTENER_PENDING`]
    /// datagrams and leaves the rest in the kernel queue.
    fn drain(&mut self) {
        while self.pending.len() < LISTENER_PENDING {
            let Ok((n, peer)) = self.inner.try_recv_from(&mut self.buf) else {
                return;
            };
            let datagram = self.buf.get(..n).unwrap_or(&self.buf).to_vec();
            if let Some(datagram) = self.route(peer, datagram) {
                self.pending.push_back((peer, datagram));
            }
        }
    }

    /// Opens the connected socket of a new session for `peer`.
    fn open_session(
        &mut self,
        peer: SocketAddr,
        first: Vec<u8>,
    ) -> Result<KernelDatagram, BackendError> {
        let fail = |(op, e): (&'static str, io::Error)| BackendError::DatagramSession {
            address: self.address,
            peer,
            op,
            source: e,
        };
        let sock = udp_socket(self.address, &self.options, true).map_err(fail)?;
        sock.bind(&self.address.into())
            .map_err(|e| fail(("bind", e)))?;
        sock.connect(&peer.into())
            .map_err(|e| fail(("connect", e)))?;
        let server = Server {
            inbox: Arc::default(),
            strays: Arc::clone(&self.strays),
        };
        lock(&server.inbox.state).datagrams.push_back(first);
        // Datagrams that reached the socket before the connect took effect.
        drain_session(&sock, peer, &server);
        let socket = register(sock).map_err(fail)?;
        if self.sessions.len() >= self.prune_at {
            self.sessions.retain(|_, w| w.strong_count() > 0);
            self.prune_at = (self.sessions.len() * 2).max(64);
        }
        self.sessions.insert(peer, Arc::downgrade(&server.inbox));
        Ok(KernelDatagram::new(
            socket,
            Some(server),
            self.address,
            peer,
        ))
    }
}

/// Binds one datagram listener per pool address on the same port.
///
/// With port 0 each listener gets its own kernel chosen port. The call fails
/// on the first address that cannot bind and closes the listeners it opened
/// before.
pub fn bind_datagram_pool<I>(
    addresses: I,
    port: u16,
    options: &SocketOptions,
) -> Result<Vec<KernelDatagramListener>, BackendError>
where
    I: IntoIterator<Item = IpAddr>,
{
    addresses
        .into_iter()
        .map(|ip| KernelDatagramListener::bind(SocketAddr::new(ip, port), options))
        .collect()
}

/// Opens the client side of UDP sessions for the interpreter.
///
/// The destination is the rendered `connect` host when the program sets
/// one, which must then be an IP address, and otherwise the server address
/// of the opener. The port is the override when set and otherwise the
/// rendered port. Every session binds the source address of the opener.
#[derive(Debug, Clone)]
pub struct DatagramConnectOpener<'a> {
    connector: &'a Connector,
    source: IpAddr,
    server: Option<IpAddr>,
    port: Option<u16>,
}

impl<'a> DatagramConnectOpener<'a> {
    /// Returns an opener that binds `source` and has no server address or
    /// port override.
    pub fn new(connector: &'a Connector, source: IpAddr) -> Self {
        Self {
            connector,
            source,
            server: None,
            port: None,
        }
    }

    /// Sets the destination of a `connect` without host.
    pub fn with_server(mut self, server: IpAddr) -> Self {
        self.server = Some(server);
        self
    }

    /// Replaces the rendered port of every `connect` with `port`.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    fn destination(&self, request: &OpenRequest<'_>) -> io::Result<SocketAddr> {
        if request.kind != OpenKind::Connect {
            return Err(invalid("a datagram connect opener cannot accept".into()));
        }
        let ip = match request.host {
            Some(host) => std::str::from_utf8(host)
                .ok()
                .and_then(|h| h.parse::<IpAddr>().ok())
                .ok_or_else(|| {
                    invalid(format!(
                        "connect host {} is not an IP address",
                        String::from_utf8_lossy(host)
                    ))
                })?,
            None => self
                .server
                .ok_or_else(|| invalid("no server address for a connect without host".into()))?,
        };
        Ok(SocketAddr::new(ip, self.port.unwrap_or(request.port)))
    }
}

impl DatagramOpener for DatagramConnectOpener<'_> {
    type Io = KernelDatagram;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<KernelDatagram>> {
        let opened = self.destination(&request).and_then(|destination| {
            self.connector
                .connect_datagram(self.source, destination)
                .map_err(to_io)
        });
        std::future::ready(opened)
    }
}

/// Opens the server side of UDP sessions for the interpreter from one
/// listener.
///
/// `accept` waits for the first datagram of a new peer on the listener,
/// whose port takes the place of the rendered port, as a port override of
/// the runtime does.
#[derive(Debug)]
pub struct DatagramAcceptOpener<'a> {
    listener: &'a mut KernelDatagramListener,
}

impl<'a> DatagramAcceptOpener<'a> {
    /// Returns an opener that accepts on `listener`.
    pub fn new(listener: &'a mut KernelDatagramListener) -> Self {
        Self { listener }
    }
}

impl DatagramOpener for DatagramAcceptOpener<'_> {
    type Io = KernelDatagram;

    async fn open(&mut self, request: OpenRequest<'_>) -> io::Result<KernelDatagram> {
        if request.kind != OpenKind::Accept {
            return Err(invalid("a datagram accept opener cannot connect".into()));
        }
        self.listener
            .accept()
            .await
            .map(|(session, _)| session)
            .map_err(to_io)
    }
}

fn to_io(e: BackendError) -> io::Error {
    io::Error::new(e.io_kind().unwrap_or(io::ErrorKind::Other), e)
}

fn invalid(detail: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_caps_its_queue_and_refuses_after_close() {
        let inbox = Inbox::default();
        for i in 0..SESSION_QUEUE + 5 {
            inbox.push(vec![u8::try_from(i % 256).unwrap()]).unwrap();
        }
        assert_eq!(lock(&inbox.state).datagrams.len(), SESSION_QUEUE);
        assert_eq!(lock(&inbox.state).datagrams.front(), Some(&vec![0]));
        lock(&inbox.state).closed = true;
        assert_eq!(inbox.push(vec![1]), Err(vec![1]));
    }

    #[test]
    fn connect_opener_picks_host_then_server_and_applies_the_port_override() {
        let connector = Connector::new(Default::default());
        let source: IpAddr = "127.0.0.1".parse().unwrap();
        let req = OpenRequest {
            kind: OpenKind::Connect,
            host: None,
            port: 53,
        };
        let bare = DatagramConnectOpener::new(&connector, source);
        assert_eq!(
            bare.destination(&req).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let o = bare
            .clone()
            .with_server("127.31.0.1".parse().unwrap())
            .with_port(5353);
        assert_eq!(
            o.destination(&req).unwrap(),
            "127.31.0.1:5353".parse().unwrap()
        );
        let host = OpenRequest {
            host: Some(b"127.32.0.9"),
            ..req
        };
        assert_eq!(
            bare.destination(&host).unwrap(),
            "127.32.0.9:53".parse().unwrap()
        );
        let name = OpenRequest {
            host: Some(b"ns.example.test"),
            ..req
        };
        assert_eq!(
            o.destination(&name).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let accept = OpenRequest {
            kind: OpenKind::Accept,
            ..req
        };
        assert_eq!(
            o.destination(&accept).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
