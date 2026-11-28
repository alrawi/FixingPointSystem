// SPDX-License-Identifier: GPL-3.0-only
//! The user space datapath as a [`Transport`] of the workers (AGT-25).
//!
//! The workers of `fps-runtime` and the UDP workers of [`crate::udp`] open
//! every session through a [`Transport`] (ENG-31). On the kernel backend each
//! worker binds its own sockets. The datapath of TRN-27 serves a port once for
//! all server hosts of all lanes, while the agent starts one server worker for
//! each plan worker and mix entry on the same port, each with its own slice of
//! the server hosts. [`DatapathTransport`] bridges the two:
//!
//! * Clients dial with [`Connector::connect`] and [`Connector::connect_udp`].
//!   The source is a client host of the plan and the destination a server
//!   host, which the workers pick from the pools.
//! * A server `listen` call names the addresses of one worker. The first call
//!   for a port starts `Datapath::listen` or `Datapath::listen_udp` for it,
//!   once. A task of the *hub* takes every session that the datapath accepts
//!   on the port and hands it to the call whose addresses hold the session's
//!   `local_addr()`, so two workers that listen on one port with different
//!   hosts each get the sessions of their own hosts, as two kernel sockets
//!   bound to different addresses would.
//! * A TCP session whose host no call names is kept for [`PENDING_GRACE`] for
//!   the next call that names it (bug #688), and reset after that. A UDP
//!   session whose host no call names, and a session whose queue is full, is
//!   reset (TCP) or dropped (UDP). The kernel does the same for a SYN to an
//!   address with no listener and for a full accept queue.
//!
//! # The hub
//!
//! `Datapath::listen` spawns its accept tasks on the Tokio runtime of the
//! caller, and a worker runtime ends with its worker, so a listener made by a
//! worker would die with it and take the port away from the others. The
//! transport therefore owns a small runtime on a thread of its own, the hub.
//! It holds the datapath listeners and the routing tasks for the whole run.
//! [`DatapathTransport::shutdown`] stops it, which frees every port.
//!
//! # Ports that stay open
//!
//! A port that a worker asked for by number stays open on the datapath until
//! the transport shuts down, even when the last worker that listened on it is
//! gone. The data ports of the instances (`fps-runtime`) bind a port when a
//! waiter comes and release it when the last waiter leaves, and a second bind
//! on the datapath would fail while the first listener lives, so one listener
//! that lasts the run lets every later waiter bind again at once. While no
//! worker listens, the stack still accepts the handshake of a connection, and
//! the hub resets it after [`PENDING_GRACE`], so the peer sees a reset
//! connection, as it does on the kernel while the port is closed. Nothing is left to the kernel.
//!
//! A port that the transport chose for a `listen` on port 0 (a SIP media port,
//! say) is different: it closes when its last listener drops, because every
//! call would otherwise leave one more listener behind.
//!
//! # What this transport does not do
//!
//! DHCP hosts are refused when the backend starts, see [`crate::backend`].
//! The addresses are IPv4 or IPv6 (AGT-28): both pools of a run use one
//! family, and the datapath finds the lane of a session by the family of its
//! hosts.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use fps_datapath::{Connector, Datapath, DatapathError, Listener, UdpListener};
use fps_hosts::{HostPool, HostRange};
use fps_io::SessionIo;
use fps_runtime::transport::{
    BoxFuture, DatagramListener, DatagramSession, StreamListener, StreamSession, Transport,
};
use fps_tcp_own::LeaseBoard;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, Notify};

/// The sessions that wait for one `listen` call, at most.
const QUEUE: usize = 1024;

/// How long the hub keeps a TCP session that arrived while no call held its
/// host, for the next call that subscribes the host (bug #688). A server worker
/// binds its port when its scenario reaches the listen step, and a client that
/// starts a moment earlier completes the handshake first; a kernel answers such
/// a SYN with a reset before the handshake, which the client retries, but the
/// stack of the datapath has already completed it, so a reset now fails the
/// session. The grace equals `OPEN_GRACE` of `fps-runtime`, which is how long
/// the server side of a spawned flow retries its bind. After the grace the session is reset, as a port that no call
/// listens on resets it.
const PENDING_GRACE: Duration = Duration::from_secs(2);

/// The sessions that wait for a call on one port, at most. A playback of
/// `hls/vod_segments` has 33 flows on one port.
const PENDING_MAX: usize = 256;

/// The ports that the transport picks for a `listen` on port 0.
const EPHEMERAL_FIRST: u32 = 32_768;
const EPHEMERAL_COUNT: u32 = 28_000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A critical section here only pushes, removes, or reads whole values, so a
    // panic in another one leaves the data consistent.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Maps a datapath error to an I/O error that keeps the error as its cause
/// and the kind that the runtime acts on: it retries a refused connect and a
/// bind that finds the port in use.
fn to_io(e: DatapathError) -> io::Error {
    let kind = e.io_kind().unwrap_or(match &e {
        DatapathError::UnknownSource(_) | DatapathError::UnknownDestination(_) => {
            io::ErrorKind::InvalidInput
        }
        DatapathError::Unsupported(_) => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::Other,
    });
    io::Error::new(kind, e)
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail.to_owned())
}

fn stopped() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the datapath transport was shut down",
    )
}

// ---------------------------------------------------------------------------
// One datapath port and the listen calls that share it
// ---------------------------------------------------------------------------

/// One `listen` call that waits on a port: the addresses it named and the
/// queue that the hub fills.
struct Sub<T> {
    id: u64,
    hosts: Vec<IpAddr>,
    tx: mpsc::Sender<io::Result<T>>,
}

struct PortState<T> {
    subs: Vec<Sub<T>>,
    /// Sessions that arrived before a call held their host, oldest first, with
    /// the time that they arrived.
    pending: VecDeque<(Instant, IpAddr, T)>,
    /// Set when the last call of a port of the transport's own choice left.
    /// A call that finds it set takes another port.
    dead: bool,
}

/// A port of the datapath that the hub serves.
struct Port<T> {
    number: u16,
    /// How long a session that no call holds waits for one. Zero refuses it at
    /// once.
    grace: Duration,
    /// A port that the transport chose, which closes with its last call.
    ephemeral: bool,
    state: Mutex<PortState<T>>,
    /// Ends the task of the hub that serves the port.
    close: Notify,
    registry: Weak<Registry<T>>,
}

type Ports<T> = Mutex<HashMap<u16, Arc<Port<T>>>>;

/// The open ports of one protocol.
struct Registry<T> {
    ports: Ports<T>,
}

impl<T> Port<T> {
    /// Adds a call for `addresses`, whose hosts no other call of the port
    /// holds.
    fn subscribe(
        self: &Arc<Self>,
        ids: &AtomicU64,
        addresses: &[SocketAddr],
    ) -> io::Result<Subscription<T>> {
        let hosts: Vec<IpAddr> = addresses.iter().map(SocketAddr::ip).collect();
        let (tx, rx) = mpsc::channel(QUEUE);
        let id = ids.fetch_add(1, Ordering::Relaxed);
        {
            let mut state = lock(&self.state);
            if state.dead {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "the port is closing",
                ));
            }
            if let Some(held) = state
                .subs
                .iter()
                .flat_map(|s| s.hosts.iter())
                .find(|h| hosts.contains(h))
            {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("{held}:{} is already listened on", self.number),
                ));
            }
            // Give the call the sessions of its hosts that came first. A
            // session that the call's queue cannot take stays parked.
            let mut rest = VecDeque::with_capacity(state.pending.len());
            for (at, host, item) in std::mem::take(&mut state.pending) {
                if hosts.contains(&host) {
                    match tx.try_send(Ok(item)) {
                        Ok(()) => continue,
                        Err(e) => {
                            if let Ok(item) = e.into_inner() {
                                rest.push_back((at, host, item));
                            }
                            continue;
                        }
                    }
                }
                rest.push_back((at, host, item));
            }
            state.pending = rest;
            state.subs.push(Sub { id, hosts, tx });
        }
        Ok(Subscription {
            port: Arc::clone(self),
            id,
            addresses: addresses.to_vec(),
            rx,
        })
    }

    /// Hands `item`, which arrived at `host`, to the call that holds the host.
    /// When no call holds it, the port keeps it for `grace` and gives it to the
    /// next call that holds the host (see [`Port::expire`]). Gives it back when
    /// the port cannot keep it or the call's queue is full or gone.
    fn route(&self, host: IpAddr, item: T) -> Result<(), T> {
        let mut state = lock(&self.state);
        let Some(sub) = state.subs.iter().find(|s| s.hosts.contains(&host)) else {
            if self.grace.is_zero() || state.pending.len() >= PENDING_MAX {
                return Err(item);
            }
            state.pending.push_back((Instant::now(), host, item));
            return Ok(());
        };
        match sub.tx.try_send(Ok(item)) {
            Ok(()) => Ok(()),
            // A refused send holds the `Ok` that was sent.
            Err(e) => e.into_inner().map_or(Ok(()), Err),
        }
    }

    /// Takes the parked sessions whose grace is over, and returns them with the
    /// time that the next one expires, if any.
    fn expire(&self, now: Instant) -> (Vec<T>, Option<Instant>) {
        let mut state = lock(&self.state);
        let mut expired = Vec::new();
        // The queue is in arrival order, so the expired ones are at the front.
        while state
            .pending
            .front()
            .is_some_and(|(at, _, _)| now.saturating_duration_since(*at) >= self.grace)
        {
            if let Some((_, _, item)) = state.pending.pop_front() {
                expired.push(item);
            }
        }
        let next = state.pending.front().map(|(at, _, _)| *at + self.grace);
        (expired, next)
    }

    /// Drops every call, which makes their `accept` fail. The task of the hub
    /// calls it when it ends.
    fn clear(&self) {
        let mut state = lock(&self.state);
        state.subs.clear();
        state.pending.clear();
        state.dead = true;
    }
}

/// One call to `listen`: the addresses, and the queue of the sessions that
/// arrive at them. Dropping it ends the call.
struct Subscription<T> {
    port: Arc<Port<T>>,
    id: u64,
    addresses: Vec<SocketAddr>,
    rx: mpsc::Receiver<io::Result<T>>,
}

impl<T> Subscription<T> {
    async fn recv(&mut self) -> io::Result<T> {
        self.rx.recv().await.unwrap_or_else(|| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the datapath listener ended",
            ))
        })
    }
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        let last = {
            let mut state = lock(&self.port.state);
            state.subs.retain(|s| s.id != self.id);
            let last = self.port.ephemeral && state.subs.is_empty() && !state.dead;
            if last {
                state.dead = true;
            }
            last
        };
        if last {
            if let Some(registry) = self.port.registry.upgrade() {
                let mut ports = lock(&registry.ports);
                if ports
                    .get(&self.port.number)
                    .is_some_and(|p| Arc::ptr_eq(p, &self.port))
                {
                    ports.remove(&self.port.number);
                }
            }
            // The task of the hub drops the datapath listener, which frees the
            // port.
            self.port.close.notify_one();
        }
    }
}

/// Clears the calls of a port when its task ends, however it ends, so a call
/// that waits in `accept` fails instead of waiting for ever.
struct EndGuard<T>(Arc<Port<T>>);

impl<T> Drop for EndGuard<T> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

/// Resets the parked sessions of `port` whose grace is over and returns the
/// time that the next one expires.
fn reap(port: &Port<StreamSession>) -> Option<Instant> {
    let (expired, next) = port.expire(Instant::now());
    for mut lost in expired {
        lost.abort();
    }
    next
}

/// Takes the sessions of a TCP port and routes each to its call.
///
/// The guard is made before the future exists, so it also runs when the hub
/// drops a task that never ran.
fn serve_tcp(
    mut listener: Listener,
    port: Arc<Port<StreamSession>>,
) -> impl std::future::Future<Output = ()> {
    let end = EndGuard(Arc::clone(&port));
    async move {
        let _end = end;
        let mut next_expiry: Option<Instant> = None;
        loop {
            let accepted = tokio::select! {
                () = port.close.notified() => return,
                () = async {
                    match next_expiry {
                        Some(at) => tokio::time::sleep_until(at.into()).await,
                        None => std::future::pending().await,
                    }
                } => {
                    // A session that no call took within the grace is reset.
                    next_expiry = reap(&port);
                    continue;
                }
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((mut session, peer)) => {
                    let Ok(local) = session.local_addr() else {
                        session.abort();
                        continue;
                    };
                    let routed = StreamSession::new(session, Some(local), Some(peer));
                    if let Err(mut lost) = port.route(local.ip(), routed) {
                        // The port cannot keep it, or the queue of the call is
                        // full: refuse.
                        lost.abort();
                    }
                    // A parked session needs its deadline; a call may also have
                    // taken the earlier ones, which only makes a wake up idle.
                    next_expiry = reap(&port);
                }
                Err(e) if e.io_kind() == Some(io::ErrorKind::BrokenPipe) => return,
                Err(_) => tokio::task::yield_now().await,
            }
        }
    }
}

/// Takes the sessions of a UDP port and routes each to its call.
fn serve_udp(
    mut listener: UdpListener,
    port: Arc<Port<DatagramSession>>,
) -> impl std::future::Future<Output = ()> {
    let end = EndGuard(Arc::clone(&port));
    async move {
        let _end = end;
        loop {
            let accepted = tokio::select! {
                () = port.close.notified() => return,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((session, peer)) => {
                    let local = session.local_addr();
                    // A session that no call takes is dropped, which closes it.
                    let _ = port.route(local.ip(), DatagramSession::new(session, local, peer));
                }
                Err(_) => tokio::task::yield_now().await,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The hub: a runtime on a thread of its own
// ---------------------------------------------------------------------------

struct Hub {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Hub {
    fn start() -> io::Result<(Handle, Self)> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let handle = runtime.handle().clone();
        let (stop, stopped) = oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("fps-dp-hub".to_owned())
            .spawn(move || {
                runtime.block_on(async {
                    let _ = stopped.await;
                });
                // Dropping the runtime drops the tasks, so every datapath
                // listener leaves its port here.
            })?;
        Ok((
            handle,
            Self {
                stop: Some(stop),
                thread: Some(thread),
            },
        ))
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ---------------------------------------------------------------------------
// The leased sources of a client pool that leases (AGT-23)
// ---------------------------------------------------------------------------

/// What the transport reads of a lease board: the hosts it covers and the
/// address that a host holds now. [`LeaseBoard`] is the implementation of a
/// run, and a test supplies one that changes its answer.
pub trait LeaseLookup: Send + Sync {
    /// The pool indexes that the board covers.
    fn range(&self) -> HostRange;
    /// The address that host `index` of the pool holds now.
    fn addr_of(&self, index: u32) -> Option<Ipv4Addr>;
}

impl LeaseLookup for LeaseBoard {
    fn range(&self) -> HostRange {
        LeaseBoard::range(self)
    }

    fn addr_of(&self, index: u32) -> Option<Ipv4Addr> {
        LeaseBoard::addr_of(self, index)
    }
}

/// Turns the source of a session of a client pool that leases into the address
/// that its host holds on the live lease board.
///
/// The workers of such a pool pick a host index, and the runtime turns it into
/// a source with the virtual pool of [`crate::assemble`], which only encodes
/// the index in 240.0.0.0/4. The transport holds a `LeaseMap` exactly when the
/// client pool of the run leases, so the mapping follows from the pool and
/// never from a test of the address. The board is read at every connect, so a
/// host that renewed onto another address connects from it.
///
/// A host without a lease fails its session before any frame is sent. The
/// virtual address does not appear in an error: the text names `dhcp4`, the
/// pool, and the host index.
#[derive(Clone)]
pub struct LeaseMap {
    pool: String,
    virtual_pool: HostPool,
    boards: Vec<Arc<dyn LeaseLookup>>,
}

impl fmt::Debug for LeaseMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeaseMap")
            .field("pool", &self.pool)
            .field("boards", &self.boards.len())
            .finish()
    }
}

impl LeaseMap {
    /// A map for the client pool `pool` whose workers use `virtual_pool`, over
    /// the boards of the lanes.
    #[must_use]
    pub fn new(virtual_pool: HostPool, boards: Vec<Arc<dyn LeaseLookup>>) -> Self {
        Self {
            pool: virtual_pool.name().to_owned(),
            virtual_pool,
            boards,
        }
    }

    /// The address that the host named by the virtual `source` holds now.
    ///
    /// # Errors
    ///
    /// `AddrNotAvailable` when the host holds no lease, and `InvalidInput`
    /// when the source names no host of the pool. Both texts name dhcp4 and
    /// the pool.
    pub fn source_of(&self, source: IpAddr) -> io::Result<IpAddr> {
        let index = match source {
            IpAddr::V4(a) => self.virtual_pool.index_of_ipv4(a),
            IpAddr::V6(_) => None,
        }
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "dhcp4: a session of host pool {} names no host of the pool",
                    self.pool
                ),
            )
        })?;
        self.boards
            .iter()
            .find(|b| {
                let r = b.range();
                index >= r.offset && index - r.offset < r.count
            })
            .and_then(|b| b.addr_of(index))
            .map(IpAddr::V4)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    format!(
                        "dhcp4: host {index} of host pool {} holds no lease, so it opens no session",
                        self.pool
                    ),
                )
            })
    }
}

// ---------------------------------------------------------------------------
// The transport
// ---------------------------------------------------------------------------

struct Inner {
    connector: Connector,
    datapath: Mutex<Option<Arc<Datapath>>>,
    hub: Mutex<Option<Hub>>,
    handle: Handle,
    tcp: Arc<Registry<StreamSession>>,
    udp: Arc<Registry<DatagramSession>>,
    ids: AtomicU64,
    ephemeral: AtomicU32,
    leases: Option<LeaseMap>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(hub) = lock(&self.hub).as_mut() {
            hub.stop();
        }
    }
}

/// The datapath as a [`Transport`]. Clones share one hub and one set of
/// ports, so every worker of a run takes the same value.
#[derive(Clone)]
pub struct DatapathTransport {
    inner: Arc<Inner>,
}

impl fmt::Debug for DatapathTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatapathTransport")
            .field("tcp_ports", &lock(&self.inner.tcp.ports).len())
            .field("udp_ports", &lock(&self.inner.udp.ports).len())
            .finish_non_exhaustive()
    }
}

impl DatapathTransport {
    /// Returns a transport over `datapath` and starts its hub.
    ///
    /// # Errors
    ///
    /// Returns the error of the operating system when the hub thread or its
    /// runtime does not start.
    pub fn new(datapath: Arc<Datapath>) -> io::Result<Self> {
        Self::build(datapath, None)
    }

    /// Like [`DatapathTransport::new`] for a run whose client pool leases its
    /// addresses: every source of a session goes through `leases` (AGT-23).
    ///
    /// # Errors
    ///
    /// As [`DatapathTransport::new`].
    pub fn with_leases(datapath: Arc<Datapath>, leases: LeaseMap) -> io::Result<Self> {
        Self::build(datapath, Some(leases))
    }

    fn build(datapath: Arc<Datapath>, leases: Option<LeaseMap>) -> io::Result<Self> {
        let (handle, hub) = Hub::start()?;
        Ok(Self {
            inner: Arc::new(Inner {
                connector: datapath.connector(),
                datapath: Mutex::new(Some(datapath)),
                hub: Mutex::new(Some(hub)),
                handle,
                tcp: Arc::new(Registry {
                    ports: Mutex::new(HashMap::new()),
                }),
                udp: Arc::new(Registry {
                    ports: Mutex::new(HashMap::new()),
                }),
                ids: AtomicU64::new(0),
                ephemeral: AtomicU32::new(0),
                leases,
            }),
        })
    }

    /// Stops the hub, which closes every port, and lets go of the datapath.
    /// Later `listen` calls fail with `ConnectionAborted`, the `accept` of a
    /// listener that exists fails, and a `connect` waits until it times out,
    /// because no lane runs any more. Calling it again changes nothing.
    pub fn shutdown(&self) {
        if let Some(hub) = lock(&self.inner.hub).as_mut() {
            hub.stop();
        }
        lock(&self.inner.datapath).take();
        lock(&self.inner.tcp.ports).clear();
        lock(&self.inner.udp.ports).clear();
    }

    /// The number of ports that the hub serves for TCP and for UDP.
    #[must_use]
    pub fn open_ports(&self) -> (usize, usize) {
        (
            lock(&self.inner.tcp.ports).len(),
            lock(&self.inner.udp.ports).len(),
        )
    }

    /// The address that a session opens from: the source that the worker
    /// picked, or for a run whose client pool leases the address that the
    /// host holds now.
    fn source(&self, source: IpAddr) -> io::Result<IpAddr> {
        match &self.inner.leases {
            Some(map) => map.source_of(source),
            None => Ok(source),
        }
    }

    fn datapath(&self) -> io::Result<Arc<Datapath>> {
        lock(&self.inner.datapath).clone().ok_or_else(stopped)
    }

    /// Returns the TCP port, started on the datapath when no call holds it.
    fn tcp_port(&self, number: u16, ephemeral: bool) -> io::Result<Arc<Port<StreamSession>>> {
        let mut ports = lock(&self.inner.tcp.ports);
        if let Some(port) = ports.get(&number) {
            return Ok(Arc::clone(port));
        }
        let datapath = self.datapath()?;
        // `Datapath::listen` spawns its accept tasks on the current runtime.
        let _enter = self.inner.handle.enter();
        let listener = datapath.listen(number).map_err(to_io)?;
        let port = self.new_port(number, ephemeral, PENDING_GRACE, &self.inner.tcp);
        self.inner
            .handle
            .spawn(serve_tcp(listener, Arc::clone(&port)));
        ports.insert(number, Arc::clone(&port));
        Ok(port)
    }

    /// Starts the hub of each TCP port in `ports` now, so a client that dials
    /// before any call listens on the port reaches the stack's listener, and the
    /// hub parks the session for `PENDING_GRACE` until a call subscribes
    /// (bug #689). A port that is open already is left alone.
    ///
    /// # Errors
    ///
    /// Returns the error of the first port that cannot start.
    pub fn prelisten_tcp(&self, ports: &[u16]) -> io::Result<()> {
        for &number in ports {
            self.tcp_port(number, false)?;
        }
        Ok(())
    }

    /// Returns the UDP port, started on the datapath when no call holds it.
    fn udp_port(&self, number: u16, ephemeral: bool) -> io::Result<Arc<Port<DatagramSession>>> {
        let mut ports = lock(&self.inner.udp.ports);
        if let Some(port) = ports.get(&number) {
            return Ok(Arc::clone(port));
        }
        let datapath = self.datapath()?;
        let listener = datapath.listen_udp(number).map_err(to_io)?;
        let port = self.new_port(number, ephemeral, Duration::ZERO, &self.inner.udp);
        self.inner
            .handle
            .spawn(serve_udp(listener, Arc::clone(&port)));
        ports.insert(number, Arc::clone(&port));
        Ok(port)
    }

    fn new_port<T>(
        &self,
        number: u16,
        ephemeral: bool,
        grace: Duration,
        registry: &Arc<Registry<T>>,
    ) -> Arc<Port<T>> {
        Arc::new(Port {
            number,
            grace,
            ephemeral,
            state: Mutex::new(PortState {
                subs: Vec::new(),
                pending: VecDeque::new(),
                dead: false,
            }),
            close: Notify::new(),
            registry: Arc::downgrade(registry),
        })
    }

    fn next_ephemeral(&self) -> u16 {
        let n = self.inner.ephemeral.fetch_add(1, Ordering::Relaxed);
        // Below 61 000, so it always fits.
        u16::try_from(EPHEMERAL_FIRST + n % EPHEMERAL_COUNT).unwrap_or(u16::MAX)
    }

    /// Adds a call for `addresses` on a port that `open` returns.
    ///
    /// A port 0 takes a port of the transport's choice: a free one that no call
    /// and no other listener of the datapath holds.
    fn subscribe<T>(
        &self,
        addresses: &[SocketAddr],
        open: impl Fn(u16, bool) -> io::Result<Arc<Port<T>>>,
    ) -> io::Result<Subscription<T>> {
        let Some(first) = addresses.first() else {
            return Err(invalid("listen needs at least one address"));
        };
        let number = first.port();
        if addresses.iter().any(|a| a.port() != number) {
            return Err(invalid("the addresses of one listen call share one port"));
        }
        if number != 0 {
            return open(number, false)?.subscribe(&self.inner.ids, addresses);
        }
        for _ in 0..EPHEMERAL_COUNT {
            let candidate = self.next_ephemeral();
            let named: Vec<SocketAddr> = addresses
                .iter()
                .map(|a| SocketAddr::new(a.ip(), candidate))
                .collect();
            let attempt = open(candidate, true).and_then(|p| p.subscribe(&self.inner.ids, &named));
            match attempt {
                Ok(sub) => return Ok(sub),
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {}
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "the datapath has no free port for a listen on port 0",
        ))
    }
}

impl Transport for DatapathTransport {
    fn connect(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> BoxFuture<'_, io::Result<StreamSession>> {
        let connector = self.inner.connector.clone();
        let source = self.source(source);
        Box::pin(async move {
            let source = source?;
            let session = connector
                .connect(source, destination)
                .await
                .map_err(to_io)?;
            let (local, peer) = (session.local_addr().ok(), session.peer_addr().ok());
            Ok(StreamSession::new(session, local, peer))
        })
    }

    fn listen(&self, addresses: &[SocketAddr]) -> io::Result<Box<dyn StreamListener>> {
        let sub = self.subscribe(addresses, |n, e| self.tcp_port(n, e))?;
        Ok(Box::new(HubStreamListener { sub }))
    }

    fn connect_datagram(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> io::Result<DatagramSession> {
        let source = self.source(source)?;
        let session = self
            .inner
            .connector
            .connect_udp(source, destination)
            .map_err(to_io)?;
        let (local, peer) = (session.local_addr(), session.peer_addr());
        Ok(DatagramSession::new(session, local, peer))
    }

    fn listen_datagram(&self, addresses: &[SocketAddr]) -> io::Result<Box<dyn DatagramListener>> {
        let sub = self.subscribe(addresses, |n, e| self.udp_port(n, e))?;
        Ok(Box::new(HubDatagramListener { sub }))
    }
}

struct HubStreamListener {
    sub: Subscription<StreamSession>,
}

impl StreamListener for HubStreamListener {
    fn local_addrs(&self) -> Vec<SocketAddr> {
        self.sub.addresses.clone()
    }

    fn accept(&mut self) -> BoxFuture<'_, io::Result<StreamSession>> {
        Box::pin(self.sub.recv())
    }
}

struct HubDatagramListener {
    sub: Subscription<DatagramSession>,
}

impl DatagramListener for HubDatagramListener {
    fn local_addrs(&self) -> Vec<SocketAddr> {
        self.sub.addresses.clone()
    }

    fn accept(&mut self) -> BoxFuture<'_, io::Result<DatagramSession>> {
        Box::pin(self.sub.recv())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(last: u8, port: u16) -> SocketAddr {
        SocketAddr::from(([10, 2, 0, last], port))
    }

    /// A port with no datapath behind it, to test the routing alone.
    fn port(ephemeral: bool) -> (Arc<Registry<u32>>, Arc<Port<u32>>) {
        let registry = Arc::new(Registry {
            ports: Mutex::new(HashMap::new()),
        });
        let port = Arc::new(Port {
            number: 80,
            grace: PENDING_GRACE,
            ephemeral,
            state: Mutex::new(PortState {
                subs: Vec::new(),
                pending: VecDeque::new(),
                dead: false,
            }),
            close: Notify::new(),
            registry: Arc::downgrade(&registry),
        });
        lock(&registry.ports).insert(80, Arc::clone(&port));
        (registry, port)
    }

    #[test]
    fn a_session_goes_to_the_call_that_holds_its_host() {
        let (_registry, port) = port(false);
        let ids = AtomicU64::new(0);
        let mut a = port.subscribe(&ids, &[addr(1, 80), addr(2, 80)]).unwrap();
        let mut b = port.subscribe(&ids, &[addr(3, 80)]).unwrap();
        assert!(port.route(addr(2, 80).ip(), 7).is_ok());
        assert!(port.route(addr(3, 80).ip(), 8).is_ok());
        assert_eq!(a.rx.try_recv().unwrap().unwrap(), 7);
        assert_eq!(b.rx.try_recv().unwrap().unwrap(), 8);
        assert!(a.rx.try_recv().is_err());
        // A host that no call holds is kept for the grace, not refused.
        assert!(port.route(addr(9, 80).ip(), 9).is_ok());
        assert_eq!(lock(&port.state).pending.len(), 1);
    }

    #[test]
    fn a_session_that_came_first_goes_to_the_next_call_that_holds_its_host() {
        let (_registry, port) = port(false);
        let ids = AtomicU64::new(0);
        for i in 0..33 {
            assert!(port.route(addr(1, 80).ip(), i).is_ok());
        }
        assert!(port.route(addr(2, 80).ip(), 99).is_ok());
        let mut a = port.subscribe(&ids, &[addr(1, 80)]).unwrap();
        // All 33 arrive in order; the session of another host stays parked.
        for i in 0..33 {
            assert_eq!(a.rx.try_recv().unwrap().unwrap(), i);
        }
        assert!(a.rx.try_recv().is_err());
        assert_eq!(lock(&port.state).pending.len(), 1);
    }

    #[test]
    fn a_parked_session_is_given_back_when_its_grace_is_over() {
        let (_registry, port) = port(false);
        assert!(port.route(addr(1, 80).ip(), 1).is_ok());
        let now = Instant::now();
        let (expired, next) = port.expire(now);
        assert!(expired.is_empty());
        assert!(next.is_some_and(|t| t > now));
        let (expired, next) = port.expire(now + PENDING_GRACE);
        assert_eq!(expired, [1]);
        assert!(next.is_none());
    }

    #[test]
    fn a_port_without_grace_or_with_a_full_park_refuses_at_once() {
        let (_registry, mut_port) = port(false);
        let none = Port {
            number: 80,
            grace: Duration::ZERO,
            ephemeral: false,
            state: Mutex::new(PortState {
                subs: Vec::new(),
                pending: VecDeque::new(),
                dead: false,
            }),
            close: Notify::new(),
            registry: Weak::new(),
        };
        assert_eq!(none.route(addr(1, 80).ip(), 4), Err(4));
        for i in 0..PENDING_MAX {
            assert!(mut_port
                .route(addr(1, 80).ip(), u32::try_from(i).unwrap())
                .is_ok());
        }
        assert_eq!(mut_port.route(addr(1, 80).ip(), 5), Err(5));
    }

    #[test]
    fn two_calls_cannot_hold_one_host_and_a_dropped_call_frees_its_hosts() {
        let (_registry, port) = port(false);
        let ids = AtomicU64::new(0);
        let a = port.subscribe(&ids, &[addr(1, 80)]).unwrap();
        let e = port
            .subscribe(&ids, &[addr(2, 80), addr(1, 80)])
            .err()
            .unwrap();
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
        drop(a);
        port.subscribe(&ids, &[addr(1, 80)]).unwrap();
        // A port that was asked for by number stays open.
        assert!(!lock(&port.state).dead);
    }

    #[test]
    fn a_full_queue_gives_the_session_back() {
        let (_registry, port) = port(false);
        let ids = AtomicU64::new(0);
        let _a = port.subscribe(&ids, &[addr(1, 80)]).unwrap();
        for i in 0..QUEUE {
            assert!(port
                .route(addr(1, 80).ip(), u32::try_from(i).unwrap())
                .is_ok());
        }
        assert_eq!(port.route(addr(1, 80).ip(), 5), Err(5));
    }

    #[test]
    fn the_last_call_of_a_port_of_our_choice_closes_it() {
        let (registry, port) = port(true);
        let ids = AtomicU64::new(0);
        let a = port.subscribe(&ids, &[addr(1, 80)]).unwrap();
        let b = port.subscribe(&ids, &[addr(2, 80)]).unwrap();
        drop(a);
        assert!(!lock(&port.state).dead, "one call is left");
        drop(b);
        assert!(lock(&port.state).dead);
        assert!(lock(&registry.ports).is_empty());
        let e = port.subscribe(&ids, &[addr(1, 80)]).err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
    }

    #[tokio::test]
    async fn the_end_of_a_port_makes_its_accept_fail() {
        let (_registry, port) = port(false);
        let ids = AtomicU64::new(0);
        let mut a = port.subscribe(&ids, &[addr(1, 80)]).unwrap();
        drop(EndGuard(Arc::clone(&port)));
        let e = a.recv().await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
    }

    /// A board whose addresses the test changes between two connects.
    struct Board {
        range: HostRange,
        addrs: Mutex<HashMap<u32, Ipv4Addr>>,
    }

    impl LeaseLookup for Board {
        fn range(&self) -> HostRange {
            self.range
        }

        fn addr_of(&self, index: u32) -> Option<Ipv4Addr> {
            lock(&self.addrs).get(&index).copied()
        }
    }

    fn map(boards: Vec<Arc<dyn LeaseLookup>>) -> (LeaseMap, HostPool) {
        let slice = crate::plan::HostPoolSlice {
            name: "clients".into(),
            pool_count: 8,
            ..crate::plan::HostPoolSlice::default()
        };
        let virtual_pool = crate::assemble::virtual_pool(&slice).unwrap();
        (LeaseMap::new(virtual_pool.clone(), boards), virtual_pool)
    }

    fn virt(pool: &HostPool, index: u32) -> IpAddr {
        IpAddr::V4(pool.ipv4_addr(index).unwrap().unwrap())
    }

    #[test]
    fn the_mapping_reads_the_board_at_each_connect() {
        let board = Arc::new(Board {
            range: HostRange::new(0, 8),
            addrs: Mutex::new(HashMap::from([(3, Ipv4Addr::new(10, 201, 0, 103))])),
        });
        let (map, pool) = map(vec![board.clone()]);
        let source = virt(&pool, 3);
        assert_eq!(
            map.source_of(source).unwrap(),
            IpAddr::from([10, 201, 0, 103])
        );
        // The host renews onto another address, and the next connect takes it.
        lock(&board.addrs).insert(3, Ipv4Addr::new(10, 201, 0, 150));
        assert_eq!(
            map.source_of(source).unwrap(),
            IpAddr::from([10, 201, 0, 150])
        );
    }

    #[test]
    fn a_host_picks_the_board_of_its_lane_by_the_absolute_index() {
        let low = Arc::new(Board {
            range: HostRange::new(0, 4),
            addrs: Mutex::new(HashMap::from([(1, Ipv4Addr::new(10, 201, 0, 101))])),
        });
        let high = Arc::new(Board {
            range: HostRange::new(4, 4),
            addrs: Mutex::new(HashMap::from([(5, Ipv4Addr::new(10, 201, 0, 105))])),
        });
        let (map, pool) = map(vec![low, high]);
        assert_eq!(
            map.source_of(virt(&pool, 5)).unwrap(),
            IpAddr::from([10, 201, 0, 105])
        );
        assert_eq!(
            map.source_of(virt(&pool, 1)).unwrap(),
            IpAddr::from([10, 201, 0, 101])
        );
    }

    #[test]
    fn a_host_without_a_lease_fails_naming_dhcp4_the_pool_and_the_index_and_not_the_virtual_address(
    ) {
        let board = Arc::new(Board {
            range: HostRange::new(0, 8),
            addrs: Mutex::new(HashMap::new()),
        });
        let (map, pool) = map(vec![board]);
        let source = virt(&pool, 6);
        let e = map.source_of(source).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AddrNotAvailable);
        let text = e.to_string();
        assert!(
            text.contains("dhcp4") && text.contains("clients") && text.contains("host 6"),
            "{text}"
        );
        assert!(!text.contains(&source.to_string()), "{text}");
        assert!(!text.contains("240."), "{text}");
    }

    #[test]
    fn a_source_that_names_no_host_of_the_pool_fails_without_a_frame() {
        let (map, _) = map(Vec::new());
        let e = map.source_of(IpAddr::from([10, 201, 0, 100])).unwrap_err();
        assert!(e.to_string().contains("dhcp4"), "{e}");
        assert!(!e.to_string().contains("10.201.0.100"), "{e}");
    }
}
