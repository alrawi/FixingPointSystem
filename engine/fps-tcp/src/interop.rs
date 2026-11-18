// SPDX-License-Identifier: GPL-3.0-only
//! The own stack against another stack, over an impaired link in memory
//! (TRN-26).
//!
//! An [`InteropStack`] joins one end of the own stack to one smoltcp host.
//! The own stack lives behind Ethernet frames and smoltcp behind IP packets
//! (its medium is `ip`), so the link between them holds Ethernet frames and
//! the smoltcp side adds the header on send and strips it on receive. The
//! station tables of the own stack give both MAC addresses, so no ARP runs.
//! The link is the impairment model of [`crate::impair`] with the settings of
//! the loss workload, and a [`SharedTap`] reads every frame that either stack
//! offers, before the link drops or delays it.
//!
//! The stack implements [`TcpStack`], so the loss workload of
//! [`crate::workload`] runs on it unchanged: with [`Pairing::OwnClient`] the
//! own stack opens and smoltcp listens, and with [`Pairing::OwnServer`] it is
//! the other way round.

use std::future::Future;
use std::io;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use fps_backend_afxdp::memory::frame_pair;
use fps_backend_afxdp::{FrameIo, QueueStats};
use fps_io::SessionIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::impair::{Framing, ImpairConfig, ImpairedLink, LinkStats};
use crate::own::{OwnConfig, OwnSession, OwnStack, PoolError, Pools};
use crate::smol::{IpBridge, Role, SmolConfig, SmolSession, SmolStack};
use crate::stack::{StackStats, TcpStack};
use crate::wiretap::{SharedTap, WireSummary};

const ETH_HEADER: usize = 14;
const ETHERTYPE_IPV4: [u8; 2] = [0x08, 0x00];
/// The room of the frame link in frames per direction.
const DEPTH: usize = 65_536;

/// The addresses of one client host and one server host of the own stack's
/// pools, with the MAC addresses that its station tables give them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairAddresses {
    /// The IPv4 address of the client host.
    pub client_ip: Ipv4Addr,
    /// The IPv4 address of the server host.
    pub server_ip: Ipv4Addr,
    /// The MAC address of the client host.
    pub client_mac: [u8; 6],
    /// The MAC address of the server host.
    pub server_mac: [u8; 6],
}

impl PairAddresses {
    /// Reads the first client host and the first server host of `pools`.
    ///
    /// # Errors
    ///
    /// Returns an error when the pools cannot be built or are empty.
    pub fn of(pools: &Pools) -> Result<Self, PoolError> {
        let built = pools.stations()?;
        let first = |slice: &fps_tcp_own::HostSlice| {
            slice
                .address(0)
                .ok_or_else(|| PoolError::NoIpv4("an empty pool".into()))
        };
        let (client, server) = (first(&pools.client)?, first(&pools.server)?);
        let mac = |station: &crate::own::net::Station, ip: u32| {
            station
                .local_mac(ip)
                .map(|m| m.0)
                .ok_or_else(|| PoolError::NoIpv4("a host without a MAC address".into()))
        };
        Ok(Self {
            client_ip: Ipv4Addr::from(client),
            server_ip: Ipv4Addr::from(server),
            client_mac: mac(&built.client, client)?,
            server_mac: mac(&built.server, server)?,
        })
    }
}

/// Which end of the connection the own stack plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pairing {
    /// The own stack opens the connection and smoltcp accepts it.
    OwnClient,
    /// smoltcp opens the connection and the own stack accepts it.
    OwnServer,
}

impl Pairing {
    /// The name that the reports use.
    pub fn name(self) -> &'static str {
        match self {
            Self::OwnClient => "own client to smoltcp server",
            Self::OwnServer => "smoltcp client to own server",
        }
    }
}

/// The settings of an [`InteropStack`].
#[derive(Clone, Debug)]
pub struct InteropConfig {
    /// Which end the own stack plays.
    pub pairing: Pairing,
    /// The own stack, with its options.
    pub own: OwnConfig,
    /// The smoltcp host. `impair` is ignored; `link` below applies.
    pub smol: SmolConfig,
    /// The settings of one direction of the link, for the direction from the
    /// client to the server. The other direction has [`ImpairConfig::reverse`].
    pub link: ImpairConfig,
}

impl InteropConfig {
    /// Defaults for both stacks over a link with `link`.
    pub fn new(pairing: Pairing, link: ImpairConfig) -> Self {
        Self {
            pairing,
            own: OwnConfig::default(),
            smol: SmolConfig {
                idle_poll_micros: 1000,
                ..SmolConfig::default()
            },
            link,
        }
    }
}

struct Wire {
    link: Mutex<ImpairedLink>,
    clock: Arc<dyn Clock>,
    wake: Notify,
    tap: SharedTap,
}

impl Wire {
    fn lock(&self) -> MutexGuard<'_, ImpairedLink> {
        self.link.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The end of the own stack: it sends into `dirs[0]` and reads `dirs[1]`.
struct OwnEnd {
    wire: Arc<Wire>,
    max_len: usize,
    scratch: Vec<u8>,
    stats: QueueStats,
}

impl FrameIo for OwnEnd {
    fn queue_id(&self) -> u32 {
        0
    }

    fn max_frame_len(&self) -> usize {
        self.max_len
    }

    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        let now = self.wire.clock.now_micros();
        let mut taken = Vec::new();
        {
            let mut link = self.wire.lock();
            while taken.len() < max {
                match link.dirs[1].pop_due(now) {
                    Some(f) => taken.push(f),
                    None => break,
                }
            }
        }
        for f in &taken {
            frame(f);
            self.stats.rx_frames += 1;
            self.stats.rx_bytes += f.len() as u64;
        }
        Ok(taken.len())
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        let now = self.wire.clock.now_micros();
        let mut queued = 0;
        {
            let mut link = self.wire.lock();
            let room = DEPTH.saturating_sub(link.dirs[0].held());
            if room < count {
                self.stats.tx_full += 1;
            }
            for i in 0..count.min(room) {
                let len = fill(i, &mut self.scratch);
                let Some(bytes) = self.scratch.get(..len).filter(|b| !b.is_empty()) else {
                    break;
                };
                self.wire.tap.record_frame(bytes);
                link.dirs[0].offer(now, bytes.to_vec());
                self.stats.tx_frames += 1;
                self.stats.tx_bytes += len as u64;
                queued += 1;
            }
        }
        if queued > 0 {
            self.wire.wake.notify_one();
        }
        Ok(queued)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        let start = self.wire.clock.now_micros();
        let limit = start.saturating_add(i64::try_from(timeout.as_micros()).unwrap_or(i64::MAX));
        loop {
            let now = self.wire.clock.now_micros();
            let next = self.wire.lock().dirs[1].next_release();
            match next {
                Some(at) if at <= now => return Ok(true),
                _ if now >= limit => return Ok(false),
                _ => std::thread::sleep(Duration::from_micros(100)),
            }
        }
    }

    fn stats(&self) -> QueueStats {
        self.stats
    }
}

/// The end of smoltcp: it sends into `dirs[1]` and reads `dirs[0]`, and it
/// wraps the IP packets of its host in Ethernet frames between two MAC
/// addresses.
struct SmolEnd {
    wire: Arc<Wire>,
    own_mac: [u8; 6],
    smol_mac: [u8; 6],
}

impl IpBridge for SmolEnd {
    fn send(&mut self, now_micros: i64, packet: Vec<u8>) {
        let mut frame = Vec::with_capacity(ETH_HEADER + packet.len());
        frame.extend_from_slice(&self.own_mac);
        frame.extend_from_slice(&self.smol_mac);
        frame.extend_from_slice(&ETHERTYPE_IPV4);
        frame.extend_from_slice(&packet);
        self.wire.tap.record_frame(&frame);
        {
            let mut link = self.wire.lock();
            if link.dirs[1].held() < DEPTH {
                link.dirs[1].offer(now_micros, frame);
            }
        }
        self.wire.wake.notify_one();
    }

    fn recv(&mut self, now_micros: i64) -> Option<Vec<u8>> {
        let mut link = self.wire.lock();
        loop {
            let frame = link.dirs[0].pop_due(now_micros)?;
            if frame.get(12..14) == Some(&ETHERTYPE_IPV4[..]) {
                return frame.get(ETH_HEADER..).map(<[u8]>::to_vec);
            }
        }
    }

    fn next_release(&self) -> Option<i64> {
        self.wire.lock().dirs[0].next_release()
    }

    fn stats(&self) -> LinkStats {
        self.wire.lock().stats()
    }
}

/// One session of either stack.
pub enum InteropSession {
    /// A session of the own stack.
    Own(OwnSession),
    /// A session of smoltcp.
    Smol(SmolSession),
}

impl AsyncRead for InteropSession {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Own(s) => Pin::new(s).poll_read(cx, buf),
            Self::Smol(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for InteropSession {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Own(s) => Pin::new(s).poll_write(cx, data),
            Self::Smol(s) => Pin::new(s).poll_write(cx, data),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Own(s) => Pin::new(s).poll_flush(cx),
            Self::Smol(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Own(s) => Pin::new(s).poll_shutdown(cx),
            Self::Smol(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

impl SessionIo for InteropSession {
    fn abort(&mut self) {
        match self {
            Self::Own(s) => s.abort(),
            Self::Smol(s) => s.abort(),
        }
    }
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The own stack and a smoltcp host, joined by one impaired link in memory.
#[derive(Clone)]
pub struct InteropStack {
    own: OwnStack,
    smol: SmolStack,
    wire: Arc<Wire>,
    pairing: Pairing,
    describe: String,
}

impl InteropStack {
    /// Builds the pair.
    ///
    /// # Errors
    ///
    /// Returns an error when the link settings are invalid or the pools of
    /// the own stack cannot be built.
    pub fn new(config: InteropConfig, clock: Arc<dyn Clock>) -> Result<Self, String> {
        config.link.validate()?;
        let pools = Pools::pair();
        let addresses = PairAddresses::of(&pools).map_err(|e| e.to_string())?;
        let frame_len = config.own.mtu + ETH_HEADER + 4;
        let wire = Arc::new(Wire {
            link: Mutex::new(ImpairedLink::new(config.link, Framing::Ethernet)),
            clock: Arc::clone(&clock),
            wake: Notify::new(),
            tap: SharedTap::new(),
        });
        let end = OwnEnd {
            wire: Arc::clone(&wire),
            max_len: frame_len,
            scratch: vec![0; frame_len],
            stats: QueueStats::default(),
        };
        let (idle, _gone) = frame_pair(1, frame_len);
        let (client, server, role, own_mac, smol_mac): (
            Box<dyn FrameIo>,
            Box<dyn FrameIo>,
            _,
            _,
            _,
        ) = match config.pairing {
            Pairing::OwnClient => (
                Box::new(end),
                Box::new(idle),
                Role::Server,
                addresses.client_mac,
                addresses.server_mac,
            ),
            Pairing::OwnServer => (
                Box::new(idle),
                Box::new(end),
                Role::Client,
                addresses.server_mac,
                addresses.client_mac,
            ),
        };
        let describe = format!(
            "{}: the own stack and smoltcp over the in memory impaired link of TRN-22 ({}), with Ethernet framing between them",
            config.pairing.name(),
            config.link.describe()
        );
        let own =
            OwnStack::with_links_and_pools(client, server, config.own, pools, Arc::clone(&clock))
                .map_err(|e| e.to_string())?;
        let bridge = SmolEnd {
            wire: Arc::clone(&wire),
            own_mac,
            smol_mac,
        };
        let smol = SmolStack::bridged(
            config.smol,
            clock,
            role,
            (addresses.client_ip, addresses.server_ip),
            Box::new(bridge),
        );
        Ok(Self {
            own,
            smol,
            wire,
            pairing: config.pairing,
            describe,
        })
    }

    /// What the tap read on the link so far.
    pub fn wire_summary(&self) -> WireSummary {
        self.wire.tap.summary()
    }

    /// The counters of the link.
    pub fn link_stats(&self) -> LinkStats {
        self.wire.lock().stats()
    }

    /// The own stack, for its counters.
    pub fn own(&self) -> &OwnStack {
        &self.own
    }

    async fn tick(self) {
        loop {
            let now = self.wire.clock.now_micros();
            let next = self.wire.lock().next_release();
            let wait = next.map_or(50_000, |at| (at - now).clamp(0, 50_000));
            if wait == 0 {
                self.own.pump();
                tokio::task::yield_now().await;
            } else {
                let sleep = Duration::from_micros(u64::try_from(wait).unwrap_or(0));
                let _ = tokio::time::timeout(sleep, self.wire.wake.notified()).await;
            }
        }
    }
}

impl TcpStack for InteropStack {
    type Session = InteropSession;

    fn name(&self) -> String {
        format!("own stack with smoltcp ({})", self.pairing.name())
    }

    fn notes(&self) -> Vec<String> {
        vec![self.describe.clone()]
    }

    fn listen(&self, port: u16) -> io::Result<()> {
        match self.pairing {
            Pairing::OwnClient => self.smol.listen(port),
            Pairing::OwnServer => self.own.listen(port),
        }
    }

    fn connect(&self, port: u16) -> impl Future<Output = io::Result<InteropSession>> + Send {
        let (own, smol, pairing) = (self.own.clone(), self.smol.clone(), self.pairing);
        async move {
            match pairing {
                Pairing::OwnClient => own.connect(port).await.map(InteropSession::Own),
                Pairing::OwnServer => smol.connect(port).await.map(InteropSession::Smol),
            }
        }
    }

    fn accept(&self, port: u16) -> impl Future<Output = io::Result<InteropSession>> + Send {
        let (own, smol, pairing) = (self.own.clone(), self.smol.clone(), self.pairing);
        async move {
            match pairing {
                Pairing::OwnClient => smol.accept(port).await.map(InteropSession::Smol),
                Pairing::OwnServer => own.accept(port).await.map(InteropSession::Own),
            }
        }
    }

    fn drive(&self) -> impl Future<Output = ()> + Send {
        let me = self.clone();
        async move {
            let smol = me.smol.clone();
            let _smol_driver = AbortOnDrop(tokio::spawn(async move { smol.drive().await }));
            let _ticker = AbortOnDrop(tokio::spawn(me.clone().tick()));
            me.own.drive().await;
        }
    }

    fn poll_once(&self) {
        self.own.pump();
        self.smol.poll_once();
    }

    fn stats(&self) -> StackStats {
        let own = TcpStack::stats(&self.own);
        let smol = self.smol.stats();
        let link = self.link_stats();
        StackStats {
            sockets_open: own.sockets_open + smol.sockets_open,
            pump_passes: own.pump_passes + smol.pump_passes,
            frames: own.frames + smol.frames,
            retransmits: link.retransmits,
            link_lost: link.lost,
            link_queue_dropped: link.queue_dropped,
        }
    }
}
