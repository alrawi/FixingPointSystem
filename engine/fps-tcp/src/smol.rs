// SPDX-License-Identifier: GPL-3.0-only
//! The smoltcp adapter, which implements [`SessionIo`] over smoltcp sockets.
//!
//! A [`SmolStack`] holds two smoltcp interfaces, a client host at 10.0.0.1 and
//! a server host at 10.0.0.2, joined by the in memory link of
//! [`crate::link`]. Each host owns its own [`SocketSet`], because smoltcp walks
//! every socket of a set on every packet and every poll, so one set per host
//! is the layout that an engine worker would use too.
//!
//! The adapter follows the rules below.
//!
//! 1. A [`SmolSession`] never drives the stack. The future of
//!    [`TcpStack::drive`] polls both interfaces, and smoltcp wakes the tasks
//!    that wait on a socket through the wakers of its `async` feature.
//! 2. The driver polls only after a session did something, and otherwise once
//!    per `idle_poll_micros`, so timers still run while all sessions idle.
//! 3. A server socket is a listening socket until a SYN arrives, so each
//!    listened port keeps `backlog` listening sockets and replaces every one
//!    that a SYN used, between two packets of the same poll.
//! 4. Dropping a session closes it gracefully and hands the socket to a reap
//!    list, which removes it from the set once it is closed. A socket in
//!    TIME-WAIT is removed at once under [`TimeWait::Skip`], which models a
//!    stack that reuses a 4-tuple at once, and it stays for ten seconds under
//!    [`TimeWait::Keep`], which is what smoltcp does by itself.

use std::collections::VecDeque;
use std::future::{poll_fn, Future};
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use fps_io::SessionIo;
use smoltcp::iface::{
    Config, Interface, PollIngressSingleResult, PollResult, SocketHandle, SocketSet,
};
use smoltcp::socket::tcp::{self, RecvError, SendError, State};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::clock::Clock;
use crate::impair::{Framing, ImpairConfig, ImpairedLink, LinkStats};
use crate::link::{LinkConfig, QueueDevice};
use crate::stack::{StackStats, TcpStack};

/// The smoltcp version that the harness measures, as `Cargo.toml` requires it.
pub const SMOLTCP_VERSION: &str = "0.14";

const CLIENT_ADDR: IpAddress = IpAddress::v4(10, 0, 0, 1);
const SERVER_ADDR: IpAddress = IpAddress::v4(10, 0, 0, 2);
/// The prefix length of the interface of a smoltcp host that plays one end of
/// an interop pair (TRN-26), wide enough that both addresses of a pair are on
/// link, because the medium has no gateway and no ARP.
const BRIDGED_PREFIX: u8 = 8;
const CLIENT_HOST: usize = 0;
const SERVER_HOST: usize = 1;
const FIRST_CLIENT_PORT: u16 = 10_000;

/// What the pair does with a socket in TIME-WAIT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeWait {
    /// Removes the socket as soon as it reaches TIME-WAIT.
    Skip,
    /// Keeps the socket for the ten seconds that smoltcp waits.
    Keep,
}

/// Settings of a [`SmolStack`].
#[derive(Clone, Debug)]
pub struct SmolConfig {
    /// The link between the two hosts.
    pub link: LinkConfig,
    /// The receive buffer of every socket in bytes.
    ///
    /// A value above 65535 turns on the window scale option.
    pub rx_buffer: usize,
    /// The transmit buffer of every socket in bytes.
    pub tx_buffer: usize,
    /// The listening sockets that each listened port keeps ready.
    pub backlog: usize,
    /// The treatment of sockets in TIME-WAIT.
    pub time_wait: TimeWait,
    /// Whether Nagle's algorithm is on. The kernel backend turns it off.
    pub nagle: bool,
    /// The delayed acknowledgment time in microseconds, or `None` to
    /// acknowledge at once. The kernel backend acknowledges at once in a
    /// request and response exchange, so `None` is the default.
    pub ack_delay_micros: Option<u64>,
    /// The longest time in microseconds between two polls of an idle pair.
    ///
    /// smoltcp visits every socket of a host on every poll, so a short
    /// interval costs more as the socket count grows. The link of a pair never
    /// loses a frame, so no retransmission timer needs a fine interval.
    pub idle_poll_micros: i64,
    /// The seed of the initial sequence numbers.
    pub seed: u64,
    /// Loss, delay, reordering, and a rate limit on the link (TRN-22), or
    /// `None` for the lossless link. A pair with an impaired link needs a
    /// short `idle_poll_micros`, because a frame that falls due must reach
    /// its host without a session acting.
    pub impair: Option<ImpairConfig>,
}

impl Default for SmolConfig {
    fn default() -> Self {
        Self {
            link: LinkConfig::default(),
            rx_buffer: 64 * 1024,
            tx_buffer: 64 * 1024,
            backlog: 8,
            time_wait: TimeWait::Skip,
            nagle: false,
            ack_delay_micros: None,
            idle_poll_micros: 100_000,
            seed: 1,
            impair: None,
        }
    }
}

/// Which end of a connection a smoltcp host plays when the other end is
/// another stack behind an [`IpBridge`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The host opens connections.
    Client,
    /// The host listens.
    Server,
}

impl Role {
    fn index(self) -> usize {
        match self {
            Self::Client => CLIENT_HOST,
            Self::Server => SERVER_HOST,
        }
    }
}

/// The link between a smoltcp host and a peer that is not smoltcp (TRN-26).
/// The packets are IPv4 packets, as the medium of the smoltcp interface wants
/// them; a bridge that joins them to a link with Ethernet frames adds and
/// strips the header.
pub trait IpBridge: Send {
    /// Takes a packet that the smoltcp host sent.
    fn send(&mut self, now_micros: i64, packet: Vec<u8>);
    /// Gives the next packet for the smoltcp host that is due at `now_micros`.
    fn recv(&mut self, now_micros: i64) -> Option<Vec<u8>>;
    /// The time at which a held packet falls due, if any.
    fn next_release(&self) -> Option<i64>;
    /// The counters of the link.
    fn stats(&self) -> LinkStats;
}

struct External {
    role: Role,
    bridge: Box<dyn IpBridge>,
}

/// The socket settings that a host applies to every socket it creates.
#[derive(Clone, Copy, Debug)]
struct SocketParams {
    rx_buffer: usize,
    tx_buffer: usize,
    nagle: bool,
    ack_delay_micros: Option<u64>,
}

impl SocketParams {
    fn socket(&self) -> tcp::Socket<'static> {
        let mut socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; self.rx_buffer]),
            tcp::SocketBuffer::new(vec![0u8; self.tx_buffer]),
        );
        socket.set_nagle_enabled(self.nagle);
        socket.set_ack_delay(self.ack_delay_micros.map(Duration::from_micros));
        socket
    }
}

/// The sockets of one listened port.
#[derive(Debug, Default)]
struct Listener {
    port: u16,
    /// Sockets in LISTEN.
    idle: Vec<SocketHandle>,
    /// Sockets that received a SYN and wait for the final ACK.
    pending: Vec<SocketHandle>,
    /// Established sockets that no `accept` took yet.
    ready: VecDeque<SocketHandle>,
    /// Tasks that wait in `accept`.
    wakers: Vec<Waker>,
}

/// One host of the pair, with its interface, link end, and sockets.
struct Host {
    iface: Interface,
    dev: QueueDevice,
    sockets: SocketSet<'static>,
    listeners: Vec<Listener>,
    /// Sockets whose session ended and that wait for removal.
    reap: Vec<SocketHandle>,
    /// Sockets in the set.
    open: u64,
}

impl Host {
    fn new(address: IpAddress, prefix: u8, config: &SmolConfig, seed: u64) -> Self {
        let mut dev = QueueDevice::new(config.link);
        let mut iface_config = Config::new(HardwareAddress::Ip);
        iface_config.random_seed = seed;
        let mut iface = Interface::new(iface_config, &mut dev, Instant::from_micros(0));
        iface.update_ip_addrs(|addrs| {
            addrs.push(IpCidr::new(address, prefix)).ok();
        });
        Self {
            iface,
            dev,
            sockets: SocketSet::new(Vec::new()),
            listeners: Vec::new(),
            reap: Vec::new(),
            open: 0,
        }
    }

    fn add(&mut self, socket: tcp::Socket<'static>) -> SocketHandle {
        self.open += 1;
        self.sockets.add(socket)
    }

    fn remove(&mut self, handle: SocketHandle) {
        self.sockets.remove(handle);
        self.open = self.open.saturating_sub(1);
    }

    fn socket(&mut self, handle: SocketHandle) -> &mut tcp::Socket<'static> {
        self.sockets.get_mut::<tcp::Socket>(handle)
    }

    fn add_listening(&mut self, params: &SocketParams, port: u16) -> io::Result<SocketHandle> {
        let mut socket = params.socket();
        socket
            .listen(port)
            .map_err(|e| io::Error::new(io::ErrorKind::AddrInUse, format!("listen: {e}")))?;
        Ok(self.add(socket))
    }

    /// Replaces the listening sockets that a SYN used and moves established
    /// sockets to the accept queue.
    fn scan_listeners(&mut self, params: &SocketParams) {
        for index in 0..self.listeners.len() {
            let mut listener = std::mem::take(&mut self.listeners[index]);
            let mut used = 0;
            let mut position = 0;
            while position < listener.idle.len() {
                let handle = listener.idle[position];
                if self.socket(handle).state() == State::Listen {
                    position += 1;
                } else {
                    listener.idle.swap_remove(position);
                    listener.pending.push(handle);
                    used += 1;
                }
            }
            for _ in 0..used {
                if let Ok(handle) = self.add_listening(params, listener.port) {
                    listener.idle.push(handle);
                }
            }
            let mut position = 0;
            let mut arrived = false;
            while position < listener.pending.len() {
                let handle = listener.pending[position];
                match self.socket(handle).state() {
                    State::SynReceived => position += 1,
                    State::Closed | State::Listen => {
                        listener.pending.swap_remove(position);
                        self.remove(handle);
                    }
                    _ => {
                        listener.pending.swap_remove(position);
                        listener.ready.push_back(handle);
                        arrived = true;
                    }
                }
            }
            if arrived {
                for waker in listener.wakers.drain(..) {
                    waker.wake();
                }
            }
            self.listeners[index] = listener;
        }
    }

    /// Removes the sockets of ended sessions that finished closing.
    fn reap(&mut self, time_wait: TimeWait) {
        let mut position = 0;
        while position < self.reap.len() {
            let handle = self.reap[position];
            let socket = self.socket(handle);
            let done = match socket.state() {
                State::Closed => socket.remote_endpoint().is_none(),
                State::TimeWait => time_wait == TimeWait::Skip,
                _ => false,
            };
            if done {
                self.reap.swap_remove(position);
                self.remove(handle);
            } else {
                position += 1;
            }
        }
    }

    /// Runs the interface until it has nothing left to process or send, and
    /// returns true when a packet arrived.
    fn step(&mut self, now: Instant, params: &SocketParams, time_wait: TimeWait) -> bool {
        let mut arrived = false;
        let mut changed = false;
        loop {
            match self
                .iface
                .poll_ingress_single(now, &mut self.dev, &mut self.sockets)
            {
                PollIngressSingleResult::None => break,
                PollIngressSingleResult::PacketProcessed => arrived = true,
                PollIngressSingleResult::SocketStateChanged => {
                    arrived = true;
                    changed = true;
                    self.scan_listeners(params);
                }
            }
        }
        loop {
            match self
                .iface
                .poll_egress(now, &mut self.dev, &mut self.sockets)
            {
                PollResult::None => break,
                PollResult::SocketStateChanged => changed = true,
            }
        }
        if changed || !self.reap.is_empty() {
            self.reap(time_wait);
        }
        arrived
    }
}

/// Moves the frames that both devices transmitted into the impaired link and
/// the frames that fell due out of it, and returns how many frames arrived.
fn move_impaired(
    link: &mut ImpairedLink,
    now_micros: i64,
    client: &mut QueueDevice,
    server: &mut QueueDevice,
) -> usize {
    while let Some(frame) = client.take_tx() {
        link.dirs[0].offer(now_micros, frame);
    }
    while let Some(frame) = server.take_tx() {
        link.dirs[1].offer(now_micros, frame);
    }
    let mut moved = 0;
    while let Some(frame) = link.dirs[0].pop_due(now_micros) {
        server.push_rx(frame);
        moved += 1;
    }
    while let Some(frame) = link.dirs[1].pop_due(now_micros) {
        client.push_rx(frame);
        moved += 1;
    }
    moved
}

/// Both hosts and the state that the driver keeps between passes.
struct Net {
    hosts: [Host; 2],
    params: SocketParams,
    time_wait: TimeWait,
    idle_poll_micros: i64,
    clock: Arc<dyn Clock>,
    next_client_port: u16,
    /// True when a session acted since the last pass.
    dirty: bool,
    last_pass_micros: i64,
    passes: u64,
    /// The impaired link, which replaces the direct move of frames.
    link: Option<ImpairedLink>,
    client_addr: IpAddress,
    server_addr: IpAddress,
    /// The peer, when it is another stack (TRN-26).
    external: Option<External>,
}

impl Net {
    fn new(
        config: &SmolConfig,
        clock: Arc<dyn Clock>,
        addresses: (IpAddress, IpAddress),
        prefix: u8,
        external: Option<External>,
    ) -> Self {
        let (client_addr, server_addr) = addresses;
        let client = Host::new(client_addr, prefix, config, config.seed);
        let server = Host::new(server_addr, prefix, config, config.seed.wrapping_add(1));
        Self {
            hosts: [client, server],
            params: SocketParams {
                rx_buffer: config.rx_buffer,
                tx_buffer: config.tx_buffer,
                nagle: config.nagle,
                ack_delay_micros: config.ack_delay_micros,
            },
            time_wait: config.time_wait,
            idle_poll_micros: config.idle_poll_micros,
            clock,
            next_client_port: FIRST_CLIENT_PORT,
            dirty: true,
            last_pass_micros: i64::MIN,
            passes: 0,
            link: config
                .impair
                .map(|impair| ImpairedLink::new(impair, Framing::Ip)),
            client_addr,
            server_addr,
            external,
        }
    }

    /// Moves frames and socket events until both hosts are quiet, and returns
    /// true when any frame moved.
    fn pump(&mut self) -> bool {
        let now_micros = self.clock.now_micros();
        let now = Instant::from_micros(now_micros);
        self.dirty = false;
        self.last_pass_micros = now_micros;
        let mut any = false;
        if let Some(external) = &mut self.external {
            let host = &mut self.hosts[external.role.index()];
            loop {
                host.step(now, &self.params, self.time_wait);
                let mut moved = 0;
                while let Some(packet) = host.dev.take_tx() {
                    external.bridge.send(now_micros, packet);
                    moved += 1;
                }
                while let Some(packet) = external.bridge.recv(now_micros) {
                    host.dev.push_rx(packet);
                    moved += 1;
                }
                if moved == 0 {
                    break;
                }
                any = true;
            }
            if any {
                self.passes += 1;
            }
            return any;
        }
        loop {
            let [client, server] = &mut self.hosts;
            client.step(now, &self.params, self.time_wait);
            server.step(now, &self.params, self.time_wait);
            let moved = match &mut self.link {
                None => {
                    client.dev.deliver_to(&mut server.dev) + server.dev.deliver_to(&mut client.dev)
                }
                Some(link) => move_impaired(link, now_micros, &mut client.dev, &mut server.dev),
            };
            if moved == 0 {
                break;
            }
            any = true;
        }
        if any {
            self.passes += 1;
        }
        any
    }

    fn due(&self) -> bool {
        let now = self.clock.now_micros();
        self.dirty
            || self
                .link
                .as_ref()
                .and_then(ImpairedLink::next_release)
                .is_some_and(|at| at <= now)
            || self
                .external
                .as_ref()
                .and_then(|e| e.bridge.next_release())
                .is_some_and(|at| at <= now)
            || self
                .clock
                .now_micros()
                .saturating_sub(self.last_pass_micros)
                >= self.idle_poll_micros
    }

    fn allocate_client_port(&mut self) -> u16 {
        let port = self.next_client_port;
        self.next_client_port = if port == u16::MAX {
            FIRST_CLIENT_PORT
        } else {
            port + 1
        };
        port
    }

    fn start_connect(&mut self, port: u16) -> io::Result<SocketHandle> {
        let local_port = self.allocate_client_port();
        let mut socket = self.params.socket();
        let host = &mut self.hosts[CLIENT_HOST];
        socket
            .connect(
                host.iface.context(),
                IpEndpoint::new(self.server_addr, port),
                (self.client_addr, local_port),
            )
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("connect: {e}")))?;
        let handle = host.add(socket);
        self.dirty = true;
        Ok(handle)
    }

    fn listen(&mut self, port: u16, backlog: usize) -> io::Result<()> {
        let params = self.params;
        let host = &mut self.hosts[SERVER_HOST];
        if host.listeners.iter().any(|l| l.port == port) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("port {port} is already listened on"),
            ));
        }
        let mut listener = Listener {
            port,
            ..Listener::default()
        };
        for _ in 0..backlog.max(1) {
            listener.idle.push(host.add_listening(&params, port)?);
        }
        host.listeners.push(listener);
        self.dirty = true;
        Ok(())
    }

    fn poll_accept(&mut self, port: u16, waker: &Waker) -> Poll<io::Result<SocketHandle>> {
        let host = &mut self.hosts[SERVER_HOST];
        let Some(listener) = host.listeners.iter_mut().find(|l| l.port == port) else {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("port {port} is not listened on"),
            )));
        };
        if let Some(handle) = listener.ready.pop_front() {
            return Poll::Ready(Ok(handle));
        }
        if !listener.wakers.iter().any(|w| w.will_wake(waker)) {
            listener.wakers.push(waker.clone());
        }
        Poll::Pending
    }
}

struct Shared {
    net: Mutex<Net>,
    backlog: usize,
    notes: Vec<String>,
}

/// Lists the facts about a smoltcp pair that a reader needs for its numbers.
fn describe(config: &SmolConfig) -> Vec<String> {
    let time_wait = match config.time_wait {
        TimeWait::Skip => "a socket in TIME-WAIT is removed at once, which models a stack that reuses a 4-tuple at once",
        TimeWait::Keep => "a socket in TIME-WAIT stays for the ten seconds that smoltcp waits",
    };
    vec![
        format!(
            "the client host and the server host are two smoltcp interfaces in one process, joined by an in memory IP link {} (mtu {}, software checksums {}), so there is no kernel socket and no descriptor",
            config.impair.map_or_else(
                || "without delay or loss".to_string(),
                |impair| format!(
                    "that is impaired ({}); retransmissions are counted at the link as data segments that start below the highest sequence number sent before",
                    impair.describe()
                ),
            ),
            config.link.mtu,
            if config.link.checksums { "on" } else { "off" },
        ),
        "smoltcp walks every socket of a host on every poll, so the poll with idle sockets grows with the socket count, and a run with many sockets splits them over shards, each shard with its own pair of socket sets".to_string(),
        format!(
            "the driver polls only after a session did something, and otherwise once per {} ms, so timers run while all sessions idle",
            config.idle_poll_micros / 1000
        ),
        time_wait.to_string(),
        format!(
            "every socket owns a receive buffer of {} bytes and a transmit buffer of {} bytes, which smoltcp allocates at once, so bytes_per_socket is mostly buffer; Nagle is {}",
            config.rx_buffer,
            config.tx_buffer,
            if config.nagle { "on" } else { "off" },
        ),
        "bytes_per_socket is the growth of the resident set of the process and includes the socket sets, the buffers, and the link".to_string(),
    ]
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Net> {
        self.net.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A client host and a server host of smoltcp joined by an in memory link.
#[derive(Clone)]
pub struct SmolStack {
    shared: Arc<Shared>,
}

impl SmolStack {
    /// Creates the pair. Its time comes from `clock`.
    pub fn new(config: SmolConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            shared: Arc::new(Shared {
                net: Mutex::new(Net::new(
                    &config,
                    clock,
                    (CLIENT_ADDR, SERVER_ADDR),
                    24,
                    None,
                )),
                backlog: config.backlog,
                notes: describe(&config),
            }),
        }
    }

    /// One smoltcp host that plays `role` at its address in `addresses` (the
    /// client address and the server address of the pair), joined by `bridge`
    /// to a peer that another stack plays (TRN-26). Only the host of `role` is
    /// polled, and `config.impair` does not apply: the bridge owns the link.
    pub fn bridged(
        config: SmolConfig,
        clock: Arc<dyn Clock>,
        role: Role,
        addresses: (std::net::Ipv4Addr, std::net::Ipv4Addr),
        bridge: Box<dyn IpBridge>,
    ) -> Self {
        let address = |a: std::net::Ipv4Addr| {
            let [a, b, c, d] = a.octets();
            IpAddress::v4(a, b, c, d)
        };
        let addresses = (address(addresses.0), address(addresses.1));
        let mut notes = describe(&config);
        notes.push(format!(
            "this host plays the {} of a connection with a peer that is not smoltcp, joined by an IP bridge (TRN-26)",
            if role == Role::Client {
                "client"
            } else {
                "server"
            }
        ));
        let external = External { role, bridge };
        Self {
            shared: Arc::new(Shared {
                net: Mutex::new(Net::new(
                    &config,
                    clock,
                    addresses,
                    BRIDGED_PREFIX,
                    Some(external),
                )),
                backlog: config.backlog,
                notes,
            }),
        }
    }
}

impl TcpStack for SmolStack {
    type Session = SmolSession;

    fn name(&self) -> String {
        format!("smoltcp {SMOLTCP_VERSION}")
    }

    fn notes(&self) -> Vec<String> {
        self.shared.notes.clone()
    }

    fn listen(&self, port: u16) -> io::Result<()> {
        self.shared.lock().listen(port, self.shared.backlog)
    }

    fn connect(&self, port: u16) -> impl Future<Output = io::Result<SmolSession>> + Send {
        let shared = Arc::clone(&self.shared);
        async move {
            let handle = shared.lock().start_connect(port)?;
            // The session owns the socket from here, so a dropped future
            // closes and reaps it.
            let mut session = SmolSession::new(shared, CLIENT_HOST, handle);
            poll_fn(|cx| session.poll_established(cx)).await?;
            Ok(session)
        }
    }

    fn accept(&self, port: u16) -> impl Future<Output = io::Result<SmolSession>> + Send {
        let shared = Arc::clone(&self.shared);
        async move {
            let handle = poll_fn(|cx| shared.lock().poll_accept(port, cx.waker())).await?;
            Ok(SmolSession::new(shared, SERVER_HOST, handle))
        }
    }

    fn drive(&self) -> impl Future<Output = ()> + Send {
        let shared = Arc::clone(&self.shared);
        async move {
            loop {
                {
                    let mut net = shared.lock();
                    if net.due() {
                        net.pump();
                    }
                }
                tokio::task::yield_now().await;
            }
        }
    }

    fn poll_once(&self) {
        let mut net = self.shared.lock();
        net.pump();
    }

    fn stats(&self) -> StackStats {
        let net = self.shared.lock();
        let link = net
            .link
            .as_ref()
            .map(ImpairedLink::stats)
            .unwrap_or_else(|| {
                net.external
                    .as_ref()
                    .map(|e| e.bridge.stats())
                    .unwrap_or_default()
            });
        StackStats {
            sockets_open: net.hosts[CLIENT_HOST].open + net.hosts[SERVER_HOST].open,
            pump_passes: net.passes,
            frames: net.hosts[CLIENT_HOST].dev.frames_sent()
                + net.hosts[SERVER_HOST].dev.frames_sent(),
            retransmits: link.retransmits,
            link_lost: link.lost,
            link_queue_dropped: link.queue_dropped,
        }
    }
}

/// One TCP session of a [`SmolStack`], which implements [`SessionIo`].
///
/// Closing the write side with `poll_shutdown` sends a FIN. Dropping the
/// session closes it, and [`SessionIo::abort`] resets it.
pub struct SmolSession {
    shared: Arc<Shared>,
    host: usize,
    handle: SocketHandle,
    /// True after `abort`, which ends every later operation.
    aborted: bool,
}

impl SmolSession {
    fn new(shared: Arc<Shared>, host: usize, handle: SocketHandle) -> Self {
        Self {
            shared,
            host,
            handle,
            aborted: false,
        }
    }

    fn poll_established(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut net = self.shared.lock();
        let socket = net.hosts[self.host].socket(self.handle);
        match socket.state() {
            State::SynSent | State::SynReceived => {
                socket.register_send_waker(cx.waker());
                socket.register_recv_waker(cx.waker());
                Poll::Pending
            }
            State::Closed | State::Listen => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "the server did not accept the connection",
            ))),
            _ => Poll::Ready(Ok(())),
        }
    }
}

fn not_connected() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "the session was aborted")
}

impl AsyncRead for SmolSession {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.aborted {
            return Poll::Ready(Err(not_connected()));
        }
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut net = this.shared.lock();
        let socket = net.hosts[this.host].socket(this.handle);
        let outcome = match socket.recv_slice(buf.initialize_unfilled()) {
            Ok(0) => {
                socket.register_recv_waker(cx.waker());
                return Poll::Pending;
            }
            Ok(read) => {
                buf.advance(read);
                Ok(())
            }
            Err(RecvError::Finished) => Ok(()),
            Err(RecvError::InvalidState) => Err(if socket.state() == State::Closed {
                io::Error::new(io::ErrorKind::ConnectionReset, "the peer reset the session")
            } else {
                io::Error::new(io::ErrorKind::NotConnected, "the session is not open")
            }),
        };
        // A read frees buffer space, so the window update needs a pass.
        net.dirty = true;
        Poll::Ready(outcome)
    }
}

impl AsyncWrite for SmolSession {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.aborted {
            return Poll::Ready(Err(not_connected()));
        }
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let mut net = this.shared.lock();
        let socket = net.hosts[this.host].socket(this.handle);
        let outcome = match socket.send_slice(data) {
            Ok(0) => {
                socket.register_send_waker(cx.waker());
                return Poll::Pending;
            }
            Ok(written) => Ok(written),
            Err(SendError::InvalidState) => Err(if socket.state() == State::Closed {
                io::Error::new(io::ErrorKind::ConnectionReset, "the peer reset the session")
            } else {
                io::Error::new(io::ErrorKind::BrokenPipe, "the write side is closed")
            }),
        };
        net.dirty = true;
        Poll::Ready(outcome)
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // The send buffer of the socket is the only buffer, and the driver
        // sends from it.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.aborted {
            return Poll::Ready(Err(not_connected()));
        }
        let mut net = this.shared.lock();
        net.hosts[this.host].socket(this.handle).close();
        net.dirty = true;
        Poll::Ready(Ok(()))
    }
}

impl SessionIo for SmolSession {
    fn abort(&mut self) {
        if self.aborted {
            return;
        }
        self.aborted = true;
        let mut net = self.shared.lock();
        let host = &mut net.hosts[self.host];
        host.socket(self.handle).abort();
        host.reap.push(self.handle);
        net.dirty = true;
    }
}

impl Drop for SmolSession {
    fn drop(&mut self) {
        if self.aborted {
            return;
        }
        let mut net = self.shared.lock();
        let host = &mut net.hosts[self.host];
        host.socket(self.handle).close();
        host.reap.push(self.handle);
        net.dirty = true;
    }
}
