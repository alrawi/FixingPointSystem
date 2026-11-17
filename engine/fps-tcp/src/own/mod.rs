// SPDX-License-Identifier: GPL-3.0-only
//! A minimal TCP stack of our own, the second measured candidate of TRN-10.
//!
//! The stack is two [`host::Host`]s, a client at `10.0.0.1` and a server at
//! `10.0.0.2`, that exchange IPv4 frames over a pair of
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
//! * IPv4 and TCP checksums, computed and verified on every frame.
//!
//! # What the stack leaves out, on purpose
//!
//! These are the limits that decision record 0014 has to weigh, because every
//! one of them is work that a production stack would still have to do.
//!
//! * Retransmission is minimal. One timer per connection with a fixed initial
//!   timeout of 200 ms that doubles up to 2 s, go back N from the first
//!   unacknowledged byte, and a reset after 8 timeouts without any segment
//!   from the peer. There is no round trip estimate, no fast retransmit, no
//!   selective acknowledgment, and no persist timer beyond the same probe.
//! * No congestion control. The sender is limited by the window of the peer
//!   only, which is right for a link of two hosts and wrong for a network.
//! * No window scaling, so a window is at most 65,535 bytes. No timestamps, no
//!   delayed acknowledgments, no Nagle (the stack sends what it has).
//! * Out of order segments are dropped, and the receiver repeats its ACK.
//! * No simultaneous open, no urgent data, no IP fragmentation or options, no
//!   keepalive, and no IPv6.
//! * One peer per host, so the table is keyed by the two ports.
//!
//! The timers sit in a heap with at most one entry per connection, so a poll
//! with idle sockets costs the same for ten sockets and for a million, which
//! is the reason `idle_poll_us` of this stack is flat where a stack that walks
//! its socket set grows with the socket count.

pub mod host;
pub mod wire;

use std::future::Future;
use std::io;
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
use crate::stack::{StackStats, TcpStack};

use host::{Host, HostCounters};

/// The name that the reports use for this stack.
pub const OWN_STACK_NAME: &str = "own minimal TCP (fps-tcp)";

/// The address of the client host, 10.0.0.1.
const CLIENT_IP: u32 = 0x0A00_0001;
/// The address of the server host, 10.0.0.2.
const SERVER_IP: u32 = 0x0A00_0002;
/// The frames that one call of the link moves at most.
const BATCH: usize = 64;

/// Settings of an [`OwnStack`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnConfig {
    /// The largest frame, which sets the MSS to this minus 40 bytes. A link
    /// with a smaller frame limit lowers it.
    pub mtu: usize,
    /// The receive buffer of a connection in bytes, which is also the
    /// advertised window up to 65,535.
    pub rcv_buffer: usize,
    /// The send buffer of a connection in bytes.
    pub snd_buffer: usize,
    /// The retransmission timeout at the start, in microseconds.
    pub rto_us: i64,
    /// The largest retransmission timeout, in microseconds.
    pub max_rto_us: i64,
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
}

impl Default for OwnConfig {
    fn default() -> Self {
        Self {
            mtu: 1500,
            rcv_buffer: 65_535,
            snd_buffer: 128 * 1024,
            rto_us: 200_000,
            max_rto_us: 2_000_000,
            max_retries: 8,
            time_wait_us: 1_000_000,
            fin_wait2_us: 10_000_000,
            backlog: 4096,
            isn_secret: 0x005E_ED0F_F1C5_0001,
            link_depth: 8192,
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
/// progress while a task polls [`TcpStack::drive`], or when a test calls
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

    /// Creates a stack over an in memory link that reads `clock`.
    pub fn with_clock(config: OwnConfig, clock: Arc<dyn Clock>) -> Self {
        let (client, server) = frame_pair(config.link_depth.max(1), config.mtu);
        Self::with_links(Box::new(client), Box::new(server), config, clock)
    }

    /// Creates a stack over two ends of a link, which tests use to lose or
    /// delay frames and TRN-12 uses for AF_XDP queues.
    pub fn with_links(
        client: Box<dyn FrameIo>,
        server: Box<dyn FrameIo>,
        config: OwnConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let mtu = config
            .mtu
            .min(client.max_frame_len())
            .min(server.max_frame_len());
        Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    client: Endpoint {
                        host: Host::new(CLIENT_IP, SERVER_IP, config.clone(), mtu),
                        link: client,
                    },
                    server: Endpoint {
                        host: Host::new(SERVER_IP, CLIENT_IP, config, mtu),
                        link: server,
                    },
                    passes: 0,
                }),
                work: Notify::new(),
                clock,
                pumped: AtomicU64::new(0),
            }),
        }
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

impl TcpStack for OwnStack {
    type Session = OwnSession;

    fn name(&self) -> String {
        OWN_STACK_NAME.to_string()
    }

    fn notes(&self) -> Vec<String> {
        let inner = self.shared.lock();
        let _ = &inner;
        vec![
            "the client host and the server host are two instances of the own TCP state machine in one process, joined by an in memory pair of frame queues of fps-backend-afxdp, so there is no kernel socket and no descriptor".to_string(),
            "frames are IPv4 with TCP, with an IP header checksum and a TCP checksum that are computed on send and verified on receive".to_string(),
            "retransmission is minimal: one timer per connection, a fixed initial timeout that doubles, go back N, no round trip estimate and no fast retransmit, and the link loses nothing, so the measurements never retransmit".to_string(),
            "there is no congestion control, no window scaling (the window is at most 65,535 bytes), no delayed acknowledgment, and no out of order queue".to_string(),
            "timers live in a heap with one entry per connection, so a poll with idle sockets does not grow with the socket count, where a stack that walks its sockets does".to_string(),
            "a closed tuple is held for one second in TIME-WAIT instead of minutes, so a rate run does not run out of ports".to_string(),
            "bytes_per_socket is the growth of the resident set of the process and includes the connection table, the buffers that the held sockets own (none while idle), and the frame link".to_string(),
        ]
    }

    fn listen(&self, port: u16) -> io::Result<()> {
        self.shared.lock().endpoint(Side::Server).listen(port)
    }

    fn connect(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
        let stack = self.clone();
        async move {
            let now = stack.shared.now();
            let handle = {
                let mut inner = stack.shared.lock();
                let handle = inner.endpoint(Side::Client).open(port, now)?;
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

    fn accept(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
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

    fn drive(&self) -> impl Future<Output = ()> + Send {
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

    fn poll_once(&self) {
        self.pump();
    }

    fn stats(&self) -> StackStats {
        let inner = self.shared.lock();
        StackStats {
            sockets_open: inner.client.host.sockets() + inner.server.host.sockets(),
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
