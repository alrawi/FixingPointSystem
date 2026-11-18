// SPDX-License-Identifier: GPL-3.0-only
//! The own TCP stack of the engine, the second measured candidate of TRN-10.
//!
//! The stack is two [`host::Host`]s, a client side and a server side, each
//! serving a pool of hosts ([`Pools`]; by default one host at `10.0.0.1` and
//! one at `10.0.1.1`), that exchange Ethernet frames (IPv4 or IPv6, optionally with an
//! 802.1Q tag, see [`net`]) over a pair of
//! [`fps_backend_afxdp::FrameIo`] queues. The default link is the in memory
//! pair of the AF_XDP crate, so the stack needs no root, and the same code
//! runs over an AF_XDP queue in TRN-12. Sessions leave the stack as
//! [`SessionIo`].
//!
//! # What the stack does
//!
//! * Connection setup with SYN, SYN-ACK, and ACK, an MSS option, and an
//!   initial sequence number of the kind RFC 6528 describes: a counter that
//!   ticks every 4 microseconds plus a hash of the tuple and a secret.
//! * Data transfer with sequence and acknowledgment numbers, a receive window
//!   that follows the free space of the receive buffer, and window updates
//!   when the application reads.
//! * Teardown with FIN in both directions, the states FIN-WAIT-1, FIN-WAIT-2,
//!   CLOSE-WAIT, CLOSING, LAST-ACK, and TIME-WAIT, and RST for aborts, for
//!   segments to unknown connections, and for refused connections.
//! * IPv4 and IPv6 (TRN-30): the headers without options or extension headers,
//!   the IPv4 header checksum, and the TCP checksum of both families, computed
//!   and verified on every frame. The family of a connection is the family of
//!   its addresses, and a pool may have prefixes of both.
//!
//! # Loss recovery (TRN-23)
//!
//! The pure mechanisms are in [`recovery`], the state machine that uses them in
//! [`host`].
//!
//! * A round trip estimator with the retransmission timeout of RFC 6298
//!   (`SRTT`, `RTTVAR`, a timeout of `SRTT + max(G, 4 RTTVAR)` between a
//!   minimum of 200 ms and a maximum of 2 s), Karn's rule (one segment is
//!   timed at a time, and none that was sent again), and a timeout that doubles
//!   on every expiry and keeps its backoff until a valid sample arrives.
//! * Fast retransmit on the third duplicate acknowledgment (RFC 5681), with
//!   Limited Transmit (RFC 3042) on the first two.
//! * An out of order queue: segments inside the receive window that arrive
//!   behind a hole are held, the receiver repeats its ACK at once for every
//!   one of them (so the sender counts duplicates), and a FIN behind a hole is
//!   remembered until the hole fills.
//! * NewReno: slow start and congestion avoidance (RFC 5681), fast recovery
//!   with partial acknowledgments (RFC 6582), a window of 10 segments to start
//!   with (RFC 6928, [`OwnConfig::initial_cwnd_segments`]), and a window of one
//!   segment after a timeout.
//!
//! The state of these mechanisms lives in a box that a connection holds only
//! while data is in flight or queued to send, or while data waits behind a
//! hole. An idle connection holds a `None` of 8 bytes, so the per socket cost
//! stays flat; the price is that an idle connection restarts slow start from
//! the initial window and forgets `SRTT` and the slow start threshold (its
//! retransmission timeout stays).
//!
//! # What the stack leaves out, on purpose
//!
//! These are the limits that decision record 0014 has to weigh, because every
//! one of them is work that a production stack would still have to do.
//!
//! * No selective acknowledgment (TRN-24), so a window with several losses
//!   recovers one hole per round trip, and no persist timer beyond the
//!   retransmission probe. A connection is reset after 8 timeouts without any
//!   segment from the peer.
//! * No window scaling, so a window is at most 65,535 bytes, which caps the
//!   goodput at 13 Mbit/s on a round trip of 40 ms. No timestamps, no delayed
//!   acknowledgments, no Nagle (the stack sends what it has). The receiver
//!   sends a window update when a read opened the window by a segment or half
//!   the buffer since the last one it sent.
//! * No simultaneous open, no urgent data, no IP fragmentation or options, no
//!   keepalive, and no IPv6 extension headers, path MTU discovery, or ICMP.
//! * No ARP requests of its own: the neighbors are read from the tables of
//!   `fps-hosts` (HST-03), and the stack answers ARP for its own hosts. The
//!   connection table is keyed by both addresses and both ports (TRN-32).
//!
//! The timers sit in a heap with at most one entry per connection, so a poll
//! with idle sockets costs the same for ten sockets and for a million, which
//! is the reason `idle_poll_us` of this stack is flat where a stack that walks
//! its socket set grows with the socket count.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod clock;
pub mod cubic;
pub mod host;
pub mod net;
pub mod pools;
pub mod recovery;
pub mod sack;
pub mod wire;

use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use fps_backend_afxdp::memory::frame_pair;
use fps_backend_afxdp::FrameIo;
use fps_io::SessionIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;

use crate::clock::{Clock, MonotonicClock};

use host::{Host, HostCounters};
use net::{ETH_LEN, VLAN_TAG_LEN};
pub use pools::{HostSlice, PoolError, Pools};
pub use recovery::CongestionControl;

/// The frames that one call of the link moves at most.
const BATCH: usize = 64;

/// Settings of an [`OwnStack`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnConfig {
    /// The largest IP packet, which sets the MSS to this minus 40 bytes. The
    /// link carries this plus the Ethernet header (14 bytes, 18 with a VLAN
    /// tag). A link with a smaller frame limit lowers it.
    pub mtu: usize,
    /// The receive buffer of a connection in bytes, which is also the
    /// advertised window. Above 65,535 the window needs the scale option
    /// (`window_scale`) and the peer's agreement.
    pub rcv_buffer: usize,
    /// The send buffer of a connection in bytes.
    pub snd_buffer: usize,
    /// The retransmission timeout before the first round trip sample, in
    /// microseconds.
    pub rto_us: i64,
    /// The smallest retransmission timeout, in microseconds. RFC 6298 asks
    /// for one second, which the kernel lowers to 200 ms, and so does this
    /// stack.
    pub min_rto_us: i64,
    /// The largest retransmission timeout, in microseconds.
    pub max_rto_us: i64,
    /// The initial congestion window in segments (RFC 6928 allows ten).
    pub initial_cwnd_segments: u32,
    /// The congestion control: CUBIC (RFC 9438, the default of the Linux
    /// kernel) or NewReno (RFC 5681, RFC 6582). Both run on the same loss
    /// recovery; only the window rules differ.
    pub congestion_control: CongestionControl,
    /// The timeouts in a row after which a connection is reset.
    pub max_retries: u8,
    /// How long a closed tuple is held, in microseconds. RFC 9293 says twice
    /// the maximum segment lifetime, which is minutes, and the default of the
    /// stack is one second, so a rate run does not run out of ports.
    pub time_wait_us: i64,
    /// How long an orphaned connection waits for the FIN of its peer, in
    /// microseconds.
    pub fin_wait2_us: i64,
    /// The connections of a port that handshake or wait for an accept.
    pub backlog: usize,
    /// The secret of the initial sequence number.
    pub isn_secret: u64,
    /// The frames that the in memory link holds per direction.
    pub link_depth: usize,
    /// Offer the window scale option (RFC 7323) in the handshake. It is used
    /// when the peer offers it too.
    pub window_scale: bool,
    /// Offer the timestamp option (RFC 7323), which gives a round trip sample
    /// per acknowledgment and protects against old duplicate segments (PAWS).
    pub timestamps: bool,
    /// Offer selective acknowledgment (RFC 2018) and recover with the
    /// scoreboard of RFC 6675 when the peer offers it too.
    pub sack: bool,
    /// Delay the acknowledgment of in order data until a second full segment
    /// arrives or `delayed_ack_us` has passed (RFC 1122 section 4.2.3.2,
    /// RFC 5681 section 4.2).
    pub delayed_ack: bool,
    /// The longest an acknowledgment waits, in microseconds. RFC 1122 allows
    /// 500 ms, the kernel waits 40 ms at the shortest.
    pub delayed_ack_us: i64,
}

impl Default for OwnConfig {
    fn default() -> Self {
        Self {
            mtu: 1500,
            rcv_buffer: 1024 * 1024,
            snd_buffer: 1024 * 1024,
            rto_us: 200_000,
            min_rto_us: 200_000,
            max_rto_us: 2_000_000,
            initial_cwnd_segments: 10,
            congestion_control: CongestionControl::Cubic,
            max_retries: 8,
            time_wait_us: 1_000_000,
            fin_wait2_us: 10_000_000,
            backlog: 4096,
            isn_secret: 0x005E_ED0F_F1C5_0001,
            link_depth: 8192,
            window_scale: true,
            timestamps: true,
            sack: true,
            delayed_ack: true,
            delayed_ack_us: 40_000,
        }
    }
}

/// The counters of both hosts, for tests and reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnCounters {
    /// The client host.
    pub client: HostCounters,
    /// The server host.
    pub server: HostCounters,
}

/// The totals of an [`OwnStack`], for the reports of the bench harness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnStats {
    /// Sockets that the pair holds now, counted on both hosts.
    pub sockets_open: u64,
    /// The hosts that serve the client side, which is the size of its pool.
    pub client_hosts: u64,
    /// The hosts that serve the server side.
    pub server_hosts: u64,
    /// Passes of the driver that moved at least one frame or socket event.
    pub pump_passes: u64,
    /// Frames that both hosts transmitted since the pair was created.
    pub frames: u64,
}

/// A host and its end of the link.
struct Endpoint {
    host: Host,
    link: Box<dyn FrameIo>,
}

impl Endpoint {
    /// Takes every waiting frame from the link and processes it.
    fn receive(&mut self, now: i64) -> usize {
        let Self { host, link } = self;
        let mut total = 0;
        loop {
            let taken = link
                .recv_batch(BATCH, &mut |frame| host.input(frame, now))
                .unwrap_or(0);
            if taken == 0 {
                return total;
            }
            total += taken;
        }
    }

    /// Fires timers, builds frames for dirty connections, and hands the
    /// frames to the link. Returns the amount of work done.
    fn transmit(&mut self, now: i64) -> usize {
        let Self { host, link } = self;
        let mut work = host.expire(now) + host.flush_dirty(now);
        let mut sent = 0;
        while !host.out.queue.is_empty() {
            let queued = {
                let frames: Vec<&[u8]> = host
                    .out
                    .queue
                    .iter()
                    .take(BATCH)
                    .map(Vec::as_slice)
                    .collect();
                link.send_batch(&frames)
            };
            match queued {
                Ok(0) => break,
                Ok(count) => {
                    host.out.queue.drain(..count);
                    sent += count;
                }
                Err(_) => {
                    // A frame that the link refuses can never be sent.
                    host.out.queue.pop_front();
                    host.counters.bad_frames += 1;
                }
            }
        }
        if sent > 0 {
            let _ = link.flush();
        }
        work += sent;
        work
    }
}

struct Inner {
    client: Endpoint,
    server: Endpoint,
    client_hosts: HostSlice,
    server_hosts: HostSlice,
    /// The number of connects so far, which picks the next pair of hosts.
    connects: u64,
    passes: u64,
}

impl Inner {
    fn endpoint(&mut self, side: Side) -> &mut Host {
        match side {
            Side::Client => &mut self.client.host,
            Side::Server => &mut self.server.host,
        }
    }

    /// Runs passes over both hosts until no frame, timer, or connection has
    /// work left. Returns true when anything moved.
    fn pump(&mut self, now: i64) -> bool {
        let mut moved = false;
        loop {
            self.passes += 1;
            let work = self.client.receive(now)
                + self.server.receive(now)
                + self.client.transmit(now)
                + self.server.transmit(now);
            if work == 0 {
                return moved;
            }
            moved = true;
        }
    }

    /// The pair of addresses of the next connect: the client hosts take
    /// turns, and the server host changes after each full round of them.
    fn next_pair(&mut self) -> Option<(IpAddr, IpAddr)> {
        let (n, m) = (
            u64::from(self.client_hosts.hosts()),
            u64::from(self.server_hosts.hosts()),
        );
        if n == 0 || m == 0 {
            return None;
        }
        let c = self.connects;
        self.connects = self.connects.wrapping_add(1);
        let local = self.client_hosts.ip((c % n) as u32)?;
        let remote = self.server_hosts.ip(((c / n) % m) as u32)?;
        Some((local, remote))
    }

    fn blocked(&self) -> bool {
        !self.client.host.out.queue.is_empty() || !self.server.host.out.queue.is_empty()
    }

    fn next_deadline(&self) -> Option<i64> {
        match (
            self.client.host.next_deadline(),
            self.server.host.next_deadline(),
        ) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn has_dirty(&self) -> bool {
        self.client.host.has_dirty() || self.server.host.has_dirty()
    }
}

/// Which hosts a connect uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// The next pair in turn.
    Next,
    /// A client host and a server host by index.
    Hosts(usize, usize),
    /// A client host and a server host by address.
    Addrs(IpAddr, IpAddr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Client,
    Server,
}

struct Shared {
    inner: Mutex<Inner>,
    work: Notify,
    clock: Arc<dyn Clock>,
    pumped: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> i64 {
        self.clock.now_micros()
    }

    /// Wakes the driver when the last operation left work for it.
    fn kick(&self, inner: &Inner) {
        if inner.has_dirty() {
            self.work.notify_one();
        }
    }
}

/// The own stack: a client host and a server host joined by a frame link.
///
/// The stack is cheap to clone, and all clones share the two hosts. It makes
/// progress while a task polls [`OwnStack::drive`], or when a test calls
/// [`OwnStack::pump`].
#[derive(Clone)]
pub struct OwnStack {
    shared: Arc<Shared>,
}

impl OwnStack {
    /// Creates a stack over an in memory link, with the wall clock.
    pub fn new(config: OwnConfig) -> Self {
        Self::with_clock(config, Arc::new(MonotonicClock::new()))
    }

    /// Creates a stack over an in memory link that reads `clock`, with one
    /// client host and one server host.
    #[allow(clippy::expect_used)]
    pub fn with_clock(config: OwnConfig, clock: Arc<dyn Clock>) -> Self {
        Self::with_pools(config, Pools::pair(), clock).expect("the constant pair of pools is valid")
    }

    /// Creates a stack whose sides serve the host pools `pools`, joined by
    /// the in memory link.
    pub fn with_pools(
        config: OwnConfig,
        pools: Pools,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, PoolError> {
        let frame = config.mtu + ETH_LEN + VLAN_TAG_LEN;
        let (client, server) = frame_pair(config.link_depth.max(1), frame);
        Self::with_links_and_pools(Box::new(client), Box::new(server), config, pools, clock)
    }

    /// Creates a stack over two ends of a link, which tests use to lose or
    /// delay frames and TRN-12 uses for AF_XDP queues, with one client host
    /// and one server host.
    #[allow(clippy::expect_used)]
    pub fn with_links(
        client: Box<dyn FrameIo>,
        server: Box<dyn FrameIo>,
        config: OwnConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::with_links_and_pools(client, server, config, Pools::pair(), clock)
            .expect("the constant pair of pools is valid")
    }

    /// Creates a stack over the given links whose sides serve `pools`.
    pub fn with_links_and_pools(
        client: Box<dyn FrameIo>,
        server: Box<dyn FrameIo>,
        config: OwnConfig,
        pools: Pools,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, PoolError> {
        let built = pools.stations()?;
        let overhead = ETH_LEN
            + if pools.vlan.is_some() {
                VLAN_TAG_LEN
            } else {
                0
            };
        let mtu = config
            .mtu
            .min(client.max_frame_len().saturating_sub(overhead))
            .min(server.max_frame_len().saturating_sub(overhead));
        Ok(Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    client: Endpoint {
                        host: Host::new(built.client, config.clone(), mtu),
                        link: client,
                    },
                    server: Endpoint {
                        host: Host::new(built.server, config, mtu),
                        link: server,
                    },
                    client_hosts: pools.client,
                    server_hosts: pools.server,
                    connects: 0,
                    passes: 0,
                }),
                work: Notify::new(),
                clock,
                pumped: AtomicU64::new(0),
            }),
        })
    }

    /// Runs the hosts until nothing moves and returns whether anything did.
    /// This is one poll of the poll mode driver.
    pub fn pump(&self) -> bool {
        let now = self.shared.now();
        self.shared.pumped.fetch_add(1, Ordering::Relaxed);
        let mut inner = self.shared.lock();
        inner.pump(now)
    }

    /// The counters of both hosts.
    pub fn counters(&self) -> OwnCounters {
        let inner = self.shared.lock();
        OwnCounters {
            client: inner.client.host.counters,
            server: inner.server.host.counters,
        }
    }

    fn session(&self, side: Side, handle: (u32, u32)) -> OwnSession {
        OwnSession {
            shared: Arc::clone(&self.shared),
            side,
            handle: Some(handle),
        }
    }
}

impl Default for OwnStack {
    fn default() -> Self {
        Self::new(OwnConfig::default())
    }
}

/// Cancels the registration of an accept that is dropped.
struct AcceptGuard {
    shared: Arc<Shared>,
    port: u16,
    ticket: Option<u64>,
}

impl Drop for AcceptGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.shared
                .lock()
                .endpoint(Side::Server)
                .cancel_accept(self.port, ticket);
        }
    }
}

impl OwnStack {
    /// Starts listening on `port` at the server host.
    ///
    /// A port may be listened on once.
    pub fn listen(&self, port: u16) -> io::Result<()> {
        self.shared.lock().endpoint(Side::Server).listen(port)
    }

    /// Opens a session from the client host to `port` at the server host and
    /// completes when the handshake finished.
    pub fn connect(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
        self.connect_with(Target::Next, port)
    }

    /// Opens a connection from client host `client_host` to server host
    /// `server_host` (indexes into the two slices) on `port`, over IPv4 when
    /// the pools have IPv4 prefixes and over IPv6 when they have only IPv6
    /// ones. [`OwnStack::connect_addrs`] picks the family.
    pub fn connect_hosts(
        &self,
        client_host: usize,
        server_host: usize,
        port: u16,
    ) -> impl Future<Output = io::Result<OwnSession>> + Send {
        self.connect_with(Target::Hosts(client_host, server_host), port)
    }

    /// Opens a connection from the client host that owns `local` to the server
    /// host that owns `remote` on `port`. The two addresses are of one family,
    /// which is the family of the connection (TRN-30).
    pub fn connect_addrs(
        &self,
        local: IpAddr,
        remote: IpAddr,
        port: u16,
    ) -> impl Future<Output = io::Result<OwnSession>> + Send {
        self.connect_with(Target::Addrs(local, remote), port)
    }

    fn connect_with(
        &self,
        target: Target,
        port: u16,
    ) -> impl Future<Output = io::Result<OwnSession>> + Send {
        let stack = self.clone();
        async move {
            let now = stack.shared.now();
            let handle = {
                let mut inner = stack.shared.lock();
                let pair = match target {
                    Target::Next => inner.next_pair(),
                    Target::Addrs(local, remote) => Some((local, remote)),
                    Target::Hosts(c, s) => u32::try_from(c)
                        .ok()
                        .zip(u32::try_from(s).ok())
                        .and_then(|(c, s)| inner.client_hosts.ip(c).zip(inner.server_hosts.ip(s))),
                };
                let Some((local, remote)) = pair else {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrNotAvailable,
                        "no such client or server host",
                    ));
                };
                let handle = inner
                    .endpoint(Side::Client)
                    .open(local, remote, port, now)?;
                stack.shared.kick(&inner);
                handle
            };
            let session = stack.session(Side::Client, handle);
            std::future::poll_fn(|cx| {
                let mut inner = stack.shared.lock();
                match inner
                    .endpoint(Side::Client)
                    .connected(handle.0, handle.1, cx.waker())
                {
                    Some(result) => Poll::Ready(result),
                    None => Poll::Pending,
                }
            })
            .await?;
            Ok(session)
        }
    }

    /// Completes with the next session that arrived at `port` on the server
    /// host.
    pub fn accept(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
        let stack = self.clone();
        async move {
            let mut guard = AcceptGuard {
                shared: Arc::clone(&stack.shared),
                port,
                ticket: None,
            };
            let handle = std::future::poll_fn(|cx| {
                let mut inner = stack.shared.lock();
                let mut ticket = guard.ticket;
                let result = inner
                    .endpoint(Side::Server)
                    .accept(port, &mut ticket, cx.waker());
                guard.ticket = ticket;
                match result {
                    Ok(Some(handle)) => Poll::Ready(Ok(handle)),
                    Ok(None) => Poll::Pending,
                    Err(error) => Poll::Ready(Err(error)),
                }
            })
            .await?;
            Ok(stack.session(Side::Server, handle))
        }
    }

    /// Moves frames and timers for both hosts until the future is dropped.
    /// The stack makes progress only while a task polls this future.
    pub fn drive(&self) -> impl Future<Output = ()> + Send {
        let stack = self.clone();
        async move {
            loop {
                let (moved, blocked, wait) = {
                    let now = stack.shared.now();
                    let mut inner = stack.shared.lock();
                    stack.shared.pumped.fetch_add(1, Ordering::Relaxed);
                    let moved = inner.pump(now);
                    let wait = inner
                        .next_deadline()
                        .map_or(1_000_000, |d| (d - now).clamp(0, 1_000_000));
                    (moved, inner.blocked(), wait)
                };
                if moved {
                    // Let the tasks that were woken run before the next pass.
                    tokio::task::yield_now().await;
                    continue;
                }
                let sleep = if blocked {
                    Duration::from_micros(50)
                } else {
                    Duration::from_micros(u64::try_from(wait).unwrap_or(0))
                };
                let _ = tokio::time::timeout(sleep, stack.shared.work.notified()).await;
            }
        }
    }

    /// The counters of the pair.
    pub fn stats(&self) -> OwnStats {
        let inner = self.shared.lock();
        OwnStats {
            sockets_open: inner.client.host.sockets() + inner.server.host.sockets(),
            client_hosts: u64::from(inner.client_hosts.hosts()),
            server_hosts: u64::from(inner.server_hosts.hosts()),
            pump_passes: inner.passes,
            frames: inner.client.host.counters.frames_out + inner.server.host.counters.frames_out,
        }
    }
}

/// One session of the own stack, on the client or the server host, which
/// implements [`SessionIo`].
///
/// Closing the write side with `poll_shutdown` queues a FIN behind the data.
/// Dropping the session closes it the way a kernel close does, with a FIN, or
/// with a reset when unread data is left, and [`SessionIo::abort`] resets it.
pub struct OwnSession {
    shared: Arc<Shared>,
    side: Side,
    handle: Option<(u32, u32)>,
}

impl std::fmt::Debug for OwnSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnSession")
            .field("side", &self.side)
            .field("handle", &self.handle)
            .finish()
    }
}

fn not_connected() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "the session was aborted")
}

impl AsyncRead for OwnSession {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let Some((id, gen)) = this.handle else {
            return Poll::Ready(Err(not_connected()));
        };
        let mut inner = this.shared.lock();
        let slot = buf.initialize_unfilled();
        match inner.endpoint(this.side).read(id, gen, slot, cx.waker()) {
            Some(Ok(count)) => {
                buf.advance(count);
                this.shared.kick(&inner);
                Poll::Ready(Ok(()))
            }
            Some(Err(error)) => Poll::Ready(Err(error)),
            None => Poll::Pending,
        }
    }
}

impl AsyncWrite for OwnSession {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let Some((id, gen)) = this.handle else {
            return Poll::Ready(Err(not_connected()));
        };
        let mut inner = this.shared.lock();
        match inner.endpoint(this.side).write(id, gen, data, cx.waker()) {
            Some(result) => {
                this.shared.kick(&inner);
                Poll::Ready(result)
            }
            None => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let Some((id, gen)) = this.handle else {
            return Poll::Ready(Err(not_connected()));
        };
        let mut inner = this.shared.lock();
        let result = inner.endpoint(this.side).shutdown(id, gen);
        this.shared.kick(&inner);
        Poll::Ready(result)
    }
}

impl SessionIo for OwnSession {
    fn abort(&mut self) {
        if let Some((id, gen)) = self.handle.take() {
            let mut inner = self.shared.lock();
            inner.endpoint(self.side).abort(id, gen);
            self.shared.kick(&inner);
        }
    }
}

impl Drop for OwnSession {
    fn drop(&mut self) {
        if let Some((id, gen)) = self.handle.take() {
            let now = self.shared.now();
            let mut inner = self.shared.lock();
            inner.endpoint(self.side).drop_app(id, gen, now);
            self.shared.kick(&inner);
        }
    }
}
