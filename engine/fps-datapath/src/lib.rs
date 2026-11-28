// SPDX-License-Identifier: GPL-3.0-only
#![deny(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The user space datapath of the engine (TRN-27).
//!
//! The datapath runs the own TCP stack of `fps-tcp-own` on the queues of a
//! port. It is a crate of its own because `fps-hosts` and the stacks depend on
//! `fps-backend-afxdp` for [`FrameIo`], so the datapath cannot live there.
//!
//! # Shape
//!
//! A [`Lane`] is the unit of one thread: the queue of the client arm, the
//! queue of the server arm, and the host pools (a slice of the pools of the
//! test configuration) that the two arms serve. The own stack keeps one host
//! per arm, so a lane has exactly the two links that the stack wants. A lane
//! over a veth pair, or over the two arms of a router under test, looks the
//! same to the code above. [`split_pools`] cuts the pools into one slice per
//! lane.
//!
//! ```text
//!   wire --> queue --> Splitter --+--> IPv4 TCP --> OwnStack (hosts, sessions)
//!                                 +--> ARP ------> fps-hosts Responder
//!                                 +--> IPv6 TCP --> OwnStack (same hosts)
//!                                 +--> NS -------> fps-hosts Responder (ND)
//!                                 +--> UDP -------> UdpArm (fps-udp-own, IPv4 and IPv6)
//!                                 +--> other ND, DHCP --> FrameHook (unsupported)
//!   wire <-- queue <---------------- frames of the stack, replies, datagrams
//! ```
//!
//! One thread per lane polls the stack ([`OwnStack::pump`]), which takes
//! frames from both queues, runs timers, and sends. The stack never waits for
//! a frame by itself, so a thread that finds nothing to do spins for a few
//! passes and then sleeps for [`DatapathConfig::idle_sleep`].
//!
//! # What a caller sees
//!
//! [`Datapath::connector`] returns a [`Connector`] with the shape of
//! `fps_backend_kernel::Connector`: `connect(source, destination)` opens a
//! session from a client host address to a server host address. A session is
//! an [`DatapathSession`], which implements `fps_io::SessionIo`.
//! [`Datapath::listen`] returns a [`Listener`] whose `accept` gives the
//! sessions that arrived at any server host of the datapath, each with its
//! peer address, as `KernelListener::accept` does. Dropping the listener
//! frees the port.
//!
//! # UDP
//!
//! Every arm of a lane has a [`UdpArm`] of `fps-udp-own`, which sits behind
//! the UDP hook of the arm's splitter, so the UDP modules run on the datapath
//! the way they run on the kernel backend. [`Connector::connect_udp`] opens a
//! [`DatapathDatagram`] (a `fps_io::DatagramIo`) from a client host to a
//! server host with an ephemeral port of the client host, and
//! [`Datapath::listen_udp`] returns a [`UdpListener`] whose `accept` gives a
//! session and the peer address for every peer that sends to the port at a
//! server host. The addresses are the host pools, as for TCP, and the MAC
//! addresses come from the same neighbor tables. The layer checks and
//! computes both checksums, fragments datagrams larger than the MTU on send,
//! and does not reassemble received fragments (it drops and counts them, see
//! the crate documentation of `fps-udp-own`). UDP ports 67 and 68 are DHCP and
//! never reach the layer. [`DatapathConfig::udp`] sets the layer or, with
//! `None`, turns it off and leaves the UDP hook of
//! [`DatapathConfig::hooks`] in place.
//!
//! # What is not served yet
//!
//! DHCP answers [`DatapathError::Unsupported`] (the calls) or
//! [`HookOutcome::Unsupported`] (the frames) and is counted in
//! [`FrameCounters`]; a [`FrameHook`] is the place for it. Neighbor
//! discovery messages that are not solicitations for a host of the pools
//! (router advertisements, say) go to the ND hook the same way.
//!
//! # Next hop resolution
//!
//! By default the stack reads the MAC address of every peer from the host
//! pools, so a peer is a host of the other pool on the same link. A peer
//! behind a router has no entry there. With [`DatapathConfig::neighbors`] set
//! (TRN-35) the stack leaves the MAC address of its peers to the splitter,
//! and the splitter keeps a neighbor cache per queue ([`NeighborConfig`]): it
//! sends an ARP request (IPv4) or a neighbor solicitation (IPv6) for the next
//! hop of a frame, holds the frame while it waits (bounded), asks again, gives
//! up after a few requests, and ages its entries. The next hop is the
//! destination on the prefix of the local host pool, and the gateway of the
//! pool otherwise ([`Route`]); a pool without a gateway resolves the
//! destination itself. The gateway comes from the pool of the plan
//! (`HostPool::gateway`). A pool that leases its addresses through DHCP has no
//! prefix and no gateway until the lease hook is wired (HST-04 keeps the
//! router of each lease in `LeaseTable::router_of`), so DHCP hosts resolve
//! the destination itself. [`FrameCounters`] counts requests, resolutions,
//! failures, held, released, and dropped frames.
//!
//! The datapath does not set `TRANSPORT_AVAILABLE` and does not change the
//! backend selection. TRN-31 flips the flag.

mod classify;
mod dhcp;
mod neighbor;
mod split;
mod udp;

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use fps_backend_afxdp::FrameIo;
use fps_hosts::{DhcpError, DhcpHosts, Responder, SliceError};
use fps_tcp_own::clock::{Clock, MonotonicClock};
use fps_tcp_own::net::Station;
use fps_tcp_own::{
    HostSlice, LeaseBoard, Leases, OwnConfig, OwnSession, OwnStack, OwnStats, PoolError, Pools,
};
use fps_udp_own::UdpArm;
pub use fps_udp_own::{UdpConfig, UdpListener, UdpSession, UdpStats};
use tokio::sync::mpsc;
use tokio::task::JoinHandle as TaskHandle;

pub use classify::{classify, Class, Frame, Tagging};
pub use dhcp::DhcpSettings;
pub use neighbor::{NeighborConfig, Route};
pub use split::{
    FrameCounters, FrameHook, HookOutcome, Hooks, Replies, SharedCounters, Splitter, Unsupported,
};

/// A session of the datapath: the session of the own stack, which implements
/// `fps_io::SessionIo` and `AsyncRead` and `AsyncWrite`.
pub type DatapathSession = OwnSession;

/// A UDP session of the datapath: the session of the UDP layer, which
/// implements `fps_io::DatagramIo`.
pub type DatapathDatagram = UdpSession;

/// A feature that the datapath does not serve yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// UDP sessions, when the layer is turned off with
    /// [`DatapathConfig::udp`] set to `None`.
    Udp,
    /// DHCP for hosts (the `DhcpHosts` of HST-04 plugs in as a hook).
    Dhcp,
}

impl Feature {
    /// The package that adds the feature.
    #[must_use]
    pub fn tracked_by(self) -> &'static str {
        match self {
            Self::Udp => "TRN-29",
            Self::Dhcp => "HST-04 on the datapath",
        }
    }
}

impl std::fmt::Display for Feature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Udp => "UDP",
            Self::Dhcp => "DHCP",
        };
        write!(
            f,
            "{name} is not supported by the user space datapath yet ({})",
            self.tracked_by()
        )
    }
}

/// Why a datapath call failed.
#[derive(Debug, thiserror::Error)]
pub enum DatapathError {
    /// The feature is not served yet.
    #[error("{0}")]
    Unsupported(Feature),
    /// A pool setup is not valid.
    #[error("host pools: {0}")]
    Pools(#[from] PoolError),
    /// The pools cannot be cut into the requested number of lanes.
    #[error("splitting the pools: {0}")]
    Split(#[from] SliceError),
    /// A side has fewer hosts than there are lanes, so a lane would serve none.
    #[error("{parts} lanes need at least {parts} hosts on each side, and a side has {hosts}")]
    TooManyLanes {
        /// The lanes asked for.
        parts: usize,
        /// The hosts of the smaller side.
        hosts: u32,
    },
    /// A datapath needs at least one lane.
    #[error("a datapath needs at least one lane")]
    NoLanes,
    /// A pool leases its addresses and the datapath has no DHCP settings.
    #[error("host pool {pool} leases its addresses (dhcp4) and DatapathConfig::dhcp is not set")]
    DhcpOff {
        /// The pool.
        pool: String,
    },
    /// The DHCP clients of a pool could not be made.
    #[error("dhcp4: {0}")]
    DhcpSetup(#[from] DhcpError),
    /// The lease phase ended before every lease was bound.
    #[error("dhcp4: {bound} of {hosts} leases bound after {timeout_ms} ms ({detail})")]
    LeaseTimeout {
        /// The leases bound when the phase ended.
        bound: usize,
        /// The hosts that lease.
        hosts: usize,
        /// The limit that passed.
        timeout_ms: u64,
        /// The bound count of each pool of each lane.
        detail: String,
    },
    /// The source address is not a client host of any lane.
    #[error("source {0} is not a client host of the datapath")]
    UnknownSource(IpAddr),
    /// The destination is not a server host in the lane of the source.
    #[error("destination {0} is not a server host in the lane of the source")]
    UnknownDestination(IpAddr),
    /// The handshake did not finish in time.
    #[error("connect from {source_addr} to {destination} timed out after {timeout_ms} ms")]
    ConnectTimeout {
        /// The source address.
        source_addr: IpAddr,
        /// The destination.
        destination: SocketAddr,
        /// The limit that passed.
        timeout_ms: u64,
    },
    /// The stack refused a call, for example a port that is already served or
    /// an exhausted connection table.
    #[error("{op}: {source}")]
    Stack {
        /// The call that failed.
        op: &'static str,
        /// The error of the stack.
        source: io::Error,
    },
    /// The thread of a lane could not start.
    #[error("starting the thread of lane {lane}: {source}")]
    Thread {
        /// The lane.
        lane: usize,
        /// The error of the system.
        source: io::Error,
    },
}

impl DatapathError {
    /// The kind of the underlying I/O error, when there is one, so a caller
    /// can tell an exhausted table from a refused connection as it does for
    /// the kernel backend.
    #[must_use]
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Stack { source, .. } | Self::Thread { source, .. } => Some(source.kind()),
            Self::ConnectTimeout { .. } => Some(io::ErrorKind::TimedOut),
            _ => None,
        }
    }
}

/// The two arms of a lane and the hosts that they serve.
pub struct Lane {
    /// The queue of the client arm. The client hosts of `pools` send from it.
    pub client: Box<dyn FrameIo>,
    /// The queue of the server arm. The server hosts of `pools` answer on it.
    pub server: Box<dyn FrameIo>,
    /// The hosts of the lane and the VLAN of its ports.
    pub pools: Pools,
}

/// The side of a lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The arm of the client hosts.
    Client,
    /// The arm of the server hosts.
    Server,
}

/// Builds the hooks of one side of one lane.
pub type HookFactory = Arc<dyn Fn(usize, Side) -> Hooks + Send + Sync>;

/// Settings of a [`Datapath`].
#[derive(Clone)]
pub struct DatapathConfig {
    /// The settings of the own stack of every lane.
    pub own: OwnConfig,
    /// How long a connect may take before it fails.
    pub connect_timeout: Duration,
    /// Empty passes that a lane thread spins before it sleeps.
    pub spin_passes: u32,
    /// How long a lane thread sleeps after the spin. This bounds the added
    /// latency of an idle lane and the CPU that it burns.
    pub idle_sleep: Duration,
    /// The prefix of the thread names, with the lane number appended.
    pub name: String,
    /// Creates the hooks of each side, or `None` for hooks that answer
    /// unsupported. The `udp` hook of the factory is used only when
    /// [`DatapathConfig::udp`] is `None`.
    pub hooks: Option<HookFactory>,
    /// The settings of the UDP layer of each arm, or `None` to run no UDP
    /// layer, which makes UDP frames go to the UDP hook of `hooks` and the
    /// UDP calls answer [`Feature::Udp`]. The datapath lowers the MTU of an
    /// arm to what its queue carries.
    pub udp: Option<UdpConfig>,
    /// Resolve the next hop of the frames with ARP and neighbor solicitation
    /// (TRN-35), or `None` to read the MAC address of every peer from the host
    /// pools. With the setting a peer need not be a host of the pools: a
    /// destination behind a router reaches the gateway of the pool of the
    /// arm. The datapath sets `own.resolve_next_hop` and defers the peers of
    /// the UDP layers itself.
    pub neighbors: Option<NeighborConfig>,
    /// The DHCP clients of the arms whose pool leases its addresses
    /// (TRN-37), or `None` when no pool leases. A pool that leases and no
    /// settings fail [`Datapath::start`]. The caller waits for the leases
    /// with [`Datapath::wait_leases`] before it opens a session.
    pub dhcp: Option<DhcpSettings>,
}

impl Default for DatapathConfig {
    /// The default stack, a connect timeout of 10 seconds, 256 spin passes,
    /// a sleep of 30 microseconds, the name `fps-dp`, no hooks, the default
    /// UDP layer, and no next hop resolution.
    fn default() -> Self {
        Self {
            own: OwnConfig::default(),
            connect_timeout: Duration::from_secs(10),
            spin_passes: 256,
            idle_sleep: Duration::from_micros(30),
            name: "fps-dp".to_owned(),
            hooks: None,
            udp: Some(UdpConfig::default()),
            neighbors: None,
            dhcp: None,
        }
    }
}

impl std::fmt::Debug for DatapathConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatapathConfig")
            .field("connect_timeout", &self.connect_timeout)
            .field("spin_passes", &self.spin_passes)
            .field("idle_sleep", &self.idle_sleep)
            .field("name", &self.name)
            .field("hooks", &self.hooks.is_some())
            .field("udp", &self.udp)
            .field("neighbors", &self.neighbors)
            .field("dhcp", &self.dhcp)
            .finish_non_exhaustive()
    }
}

/// The counters of one lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneStats {
    /// Sockets, frames, and passes of the stack.
    pub stack: OwnStats,
    /// What the splitter of the client arm did.
    pub client: FrameCounters,
    /// What the splitter of the server arm did.
    pub server: FrameCounters,
    /// What the UDP layer of the client arm did (all zero when it is off).
    pub udp_client: UdpStats,
    /// What the UDP layer of the server arm did (all zero when it is off).
    pub udp_server: UdpStats,
    /// Passes of the lane thread, empty ones included.
    pub loop_passes: u64,
}

/// What a lane reports when it stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneReport {
    /// The counters at the end.
    pub stats: LaneStats,
    /// Whether the lane thread panicked.
    pub panicked: bool,
}

/// One lane at run time.
struct Running {
    stack: OwnStack,
    client_hosts: HostSlice,
    server_hosts: HostSlice,
    leases: Leases,
    client_counters: Arc<SharedCounters>,
    server_counters: Arc<SharedCounters>,
    udp_client: Option<UdpArm>,
    udp_server: Option<UdpArm>,
    passes: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Running {
    fn stats(&self) -> LaneStats {
        LaneStats {
            stack: self.stack.stats(),
            client: self.client_counters.snapshot(),
            server: self.server_counters.snapshot(),
            udp_client: self
                .udp_client
                .as_ref()
                .map(UdpArm::stats)
                .unwrap_or_default(),
            udp_server: self
                .udp_server
                .as_ref()
                .map(UdpArm::stats)
                .unwrap_or_default(),
            loop_passes: self.passes.load(Ordering::Relaxed),
        }
    }
}

/// The index of `ip` in `slice`, or `None` when the slice does not serve it.
/// A slice that leases serves the addresses that its hosts hold now.
fn index_in(slice: &HostSlice, board: Option<&Arc<LeaseBoard>>, ip: IpAddr) -> Option<usize> {
    let index = match board {
        Some(board) => {
            let IpAddr::V4(a) = ip else { return None };
            let relative = board.index_of(a)?.checked_sub(slice.range.offset)?;
            (relative < slice.range.count).then_some(relative)?
        }
        None => slice.index_of(ip)?,
    };
    usize::try_from(index).ok()
}

/// Cuts the pools into `parts` lanes. Client hosts and server hosts are
/// divided evenly and in order, so lane `k` serves the `k`-th slice of each,
/// and a client host reaches only the server hosts of its own lane.
///
/// # Errors
///
/// Returns [`DatapathError::TooManyLanes`] when a side has fewer hosts than
/// `parts`, and [`DatapathError::Split`] when `parts` is 0.
pub fn split_pools(pools: &Pools, parts: usize) -> Result<Vec<Pools>, DatapathError> {
    let hosts = pools.client.hosts().min(pools.server.hosts());
    if parts > usize::try_from(hosts).unwrap_or(usize::MAX) {
        return Err(DatapathError::TooManyLanes { parts, hosts });
    }
    let clients = pools.client.range.split_even(parts)?;
    let servers = pools.server.range.split_even(parts)?;
    Ok(clients
        .into_iter()
        .zip(servers)
        .map(|(client, server)| Pools {
            client: HostSlice {
                pool: pools.client.pool.clone(),
                range: client,
            },
            server: HostSlice {
                pool: pools.server.pool.clone(),
                range: server,
            },
            vlan: pools.vlan,
        })
        .collect())
}

/// The ARP and neighbor discovery responder of each arm, from the neighbor
/// tables of [`Pools::tables`], which also give the stations of the stack and
/// of the UDP layer their MAC addresses, so a reply and a frame of the stack
/// carry the same MAC address by construction.
fn responders(pools: &Pools) -> Result<(Responder, Responder), PoolError> {
    let tables = pools.tables()?;
    Ok((Responder::new(tables.client), Responder::new(tables.server)))
}

/// The settings of the UDP layer of one arm: the MTU that the queue carries
/// (the frame length without the Ethernet header and the VLAN tag) bounds the
/// MTU of the layer.
fn udp_settings(settings: &UdpConfig, queue: &dyn FrameIo, vlan: Option<u16>) -> UdpConfig {
    let link = 14 + if vlan.is_some() { 4 } else { 0 };
    let mut settings = settings.clone();
    settings.mtu = settings.mtu.min(queue.max_frame_len().saturating_sub(link));
    settings
}

/// The loop of one lane: poll the stack until told to stop.
fn run_lane(stack: &OwnStack, stop: &AtomicBool, passes: &AtomicU64, spin: u32, sleep: Duration) {
    let mut empty = 0u32;
    while !stop.load(Ordering::Acquire) {
        passes.fetch_add(1, Ordering::Relaxed);
        if stack.pump() {
            empty = 0;
        } else if empty < spin {
            empty += 1;
            std::hint::spin_loop();
        } else {
            std::thread::sleep(sleep);
        }
    }
}

/// The datapath: the lanes, their threads, and the calls that open sessions.
pub struct Datapath {
    lanes: Arc<Vec<Running>>,
    threads: Vec<JoinHandle<()>>,
    config: DatapathConfig,
}

impl std::fmt::Debug for Datapath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Datapath")
            .field("lanes", &self.lanes.len())
            .field("config", &self.config)
            .finish()
    }
}

impl Datapath {
    /// Starts one thread per lane.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::NoLanes`] for an empty list,
    /// [`DatapathError::Pools`] when the pools of a lane are not valid, and
    /// [`DatapathError::Thread`] when a thread does not start. Lanes that
    /// started before the failure are stopped again.
    pub fn start(lanes: Vec<Lane>, config: DatapathConfig) -> Result<Self, DatapathError> {
        if lanes.is_empty() {
            return Err(DatapathError::NoLanes);
        }
        let mut own = config.own.clone();
        own.resolve_next_hop = config.neighbors.is_some();
        // One clock for the stack and the neighbor caches of every lane.
        let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::new());
        let mut running = Vec::with_capacity(lanes.len());
        for (n, lane) in lanes.into_iter().enumerate() {
            let (client_responder, server_responder) = responders(&lane.pools)?;
            let leases = lane.pools.leases()?;
            for (board, slice) in [
                (&leases.client, &lane.pools.client),
                (&leases.server, &lane.pools.server),
            ] {
                if board.is_some() && config.dhcp.is_none() {
                    return Err(DatapathError::DhcpOff {
                        pool: slice.pool.name().to_owned(),
                    });
                }
            }
            let (udp_client, udp_server) = match &config.udp {
                Some(settings) => {
                    let built = lane.pools.stations_with(&leases)?;
                    let defer = config.neighbors.is_some();
                    let arm = |station: Station, queue: &dyn FrameIo| {
                        let station = if defer {
                            station.defer_peers()
                        } else {
                            station
                        };
                        UdpArm::new(station, udp_settings(settings, queue, lane.pools.vlan))
                    };
                    (
                        Some(arm(built.client, &*lane.client)),
                        Some(arm(built.server, &*lane.server)),
                    )
                }
                None => (None, None),
            };
            let hooks = |side, arm: &Option<UdpArm>, queue: &dyn FrameIo| {
                let mut hooks = config
                    .hooks
                    .as_ref()
                    .map_or_else(Hooks::default, |f| f(n, side));
                if let Some(arm) = arm {
                    hooks.udp = Box::new(udp::UdpHook(arm.clone()));
                }
                let (board, slice) = match side {
                    Side::Client => (&leases.client, &lane.pools.client),
                    Side::Server => (&leases.server, &lane.pools.server),
                };
                if let (Some(board), Some(settings)) = (board, &config.dhcp) {
                    let mut dhcp = settings.config.clone();
                    dhcp.vlans = lane.pools.vlan.into_iter().collect();
                    dhcp.seed = dhcp
                        .seed
                        .wrapping_add((n as u64).wrapping_mul(2) + u64::from(side == Side::Server));
                    let now = Duration::from_micros(u64::try_from(clock.now_micros()).unwrap_or(0));
                    let hosts = DhcpHosts::new(&slice.pool, board.block(), &dhcp, now)?;
                    hooks.dhcp = Box::new(dhcp::DhcpHook::new(
                        hosts,
                        Arc::clone(board),
                        Arc::clone(&clock),
                        queue.max_frame_len(),
                    ));
                }
                Ok::<_, DatapathError>(hooks)
            };
            let client_hooks = hooks(Side::Client, &udp_client, &*lane.client)?;
            let server_hooks = hooks(Side::Server, &udp_server, &*lane.server)?;
            let (client, client_counters) = Splitter::new(
                lane.client,
                client_responder,
                lane.pools.vlan,
                client_hooks,
            );
            let (server, server_counters) = Splitter::new(
                lane.server,
                server_responder,
                lane.pools.vlan,
                server_hooks,
            );
            let (client, server) = (
                match &leases.client {
                    Some(b) => client.with_leases(Arc::clone(b)),
                    None => client,
                },
                match &leases.server {
                    Some(b) => server.with_leases(Arc::clone(b)),
                    None => server,
                },
            );
            // Each arm resolves through the gateway of its own pool.
            let resolving = |splitter: Splitter, pool: &HostSlice| match &config.neighbors {
                Some(settings) => splitter.with_next_hop(
                    settings.clone(),
                    Route::of(&pool.pool),
                    Arc::clone(&clock),
                ),
                None => splitter,
            };
            let client = resolving(client, &lane.pools.client);
            let server = resolving(server, &lane.pools.server);
            let stack = OwnStack::with_links_pools_and_leases(
                Box::new(client),
                Box::new(server),
                own.clone(),
                lane.pools.clone(),
                leases.clone(),
                Arc::clone(&clock),
            )?;
            running.push(Running {
                stack,
                client_hosts: lane.pools.client,
                server_hosts: lane.pools.server,
                leases,
                client_counters,
                server_counters,
                udp_client,
                udp_server,
                passes: Arc::new(AtomicU64::new(0)),
                stop: Arc::new(AtomicBool::new(false)),
            });
        }
        let lanes = Arc::new(running);
        let mut datapath = Self {
            lanes: Arc::clone(&lanes),
            threads: Vec::new(),
            config,
        };
        for n in 0..lanes.len() {
            let shared = Arc::clone(&lanes);
            let (spin, sleep) = (datapath.config.spin_passes, datapath.config.idle_sleep);
            let spawned = std::thread::Builder::new()
                .name(format!("{}{n}", datapath.config.name))
                .spawn(move || {
                    if let Some(lane) = shared.get(n) {
                        run_lane(&lane.stack, &lane.stop, &lane.passes, spin, sleep);
                    }
                });
            match spawned {
                Ok(handle) => datapath.threads.push(handle),
                // Dropping `datapath` stops the lanes that started.
                Err(source) => return Err(DatapathError::Thread { lane: n, source }),
            }
        }
        Ok(datapath)
    }

    /// The number of lanes.
    #[must_use]
    pub fn lanes(&self) -> usize {
        self.lanes.len()
    }

    /// A connector that opens sessions on this datapath. It may outlive the
    /// calls that made it and be cloned for every task.
    #[must_use]
    pub fn connector(&self) -> Connector {
        Connector {
            lanes: Arc::clone(&self.lanes),
            timeout: self.config.connect_timeout,
            resolving: self.config.neighbors.is_some(),
        }
    }

    /// Starts listening on `port` at every server host of every lane.
    ///
    /// Dropping the listener stops the listening on every lane and frees the
    /// port, as closing a listening socket does: connections that arrived and
    /// were not accepted are reset, and the port can be listened on again
    /// (TRN-33). Sessions that were accepted keep running. The call must run
    /// inside a Tokio runtime, which carries one task per lane that moves
    /// accepted sessions to [`Listener::accept`].
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::Stack`] when a lane already listens on the
    /// port. The lanes that started before the failure stop listening again.
    pub fn listen(&self, port: u16) -> Result<Listener, DatapathError> {
        let (tx, rx) = mpsc::channel(1024);
        // Built first, so that an error on a later lane drops it and undoes
        // the lanes that already listen.
        let mut listener = Listener {
            port,
            rx,
            tasks: Vec::with_capacity(self.lanes.len()),
            stacks: Vec::with_capacity(self.lanes.len()),
        };
        for lane in self.lanes.iter() {
            lane.stack
                .listen(port)
                .map_err(|source| DatapathError::Stack {
                    op: "listen",
                    source,
                })?;
            listener.stacks.push(lane.stack.clone());
            let (stack, tx) = (lane.stack.clone(), tx.clone());
            listener.tasks.push(tokio::spawn(async move {
                loop {
                    let accepted = stack.accept(port).await;
                    let failed = accepted.is_err();
                    if tx.send(accepted).await.is_err() || failed {
                        break;
                    }
                }
            }));
        }
        Ok(listener)
    }

    /// Starts listening for UDP on `port` at every server host of every
    /// lane. The listener gives a session for each peer that sends to the
    /// port, whatever the lane and the host, as [`Datapath::listen`] does
    /// for TCP. It needs no Tokio runtime. Dropping the listener frees the
    /// port.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::Unsupported`] with [`Feature::Udp`] when the
    /// UDP layer is off, and [`DatapathError::Stack`] when the port is 0 or
    /// a lane already listens on it.
    pub fn listen_udp(&self, port: u16) -> Result<UdpListener, DatapathError> {
        let mut listener = UdpListener::new(
            port,
            self.config
                .udp
                .as_ref()
                .ok_or(DatapathError::Unsupported(Feature::Udp))?
                .accept_queue,
        );
        for lane in self.lanes.iter() {
            let arm = lane
                .udp_server
                .as_ref()
                .ok_or(DatapathError::Unsupported(Feature::Udp))?;
            listener
                .attach(arm)
                .map_err(|source| DatapathError::Stack {
                    op: "listen_udp",
                    source,
                })?;
        }
        Ok(listener)
    }

    /// A handle on the lease phase (TRN-37).
    #[must_use]
    pub fn leases(&self) -> LeaseMonitor {
        LeaseMonitor {
            lanes: Arc::clone(&self.lanes),
            timeout: self.config.dhcp.as_ref().map(|d| d.timeout),
        }
    }

    /// Waits until every host of every leasing pool holds an address, for at
    /// most [`DhcpSettings::timeout`]. It returns at once when no pool
    /// leases. Call it before the first session; the clients keep renewing
    /// after it returns. It blocks the calling thread, so a task calls
    /// [`LeaseMonitor::wait_async`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::LeaseTimeout`], which names DHCP and the
    /// bound count, when the limit passed first.
    pub fn wait_leases(&self) -> Result<LeaseReport, DatapathError> {
        self.leases().wait()
    }

    /// A handle that reads the counters from any thread or task.
    #[must_use]
    pub fn monitor(&self) -> Monitor {
        Monitor {
            lanes: Arc::clone(&self.lanes),
        }
    }

    /// The counters of every lane.
    #[must_use]
    pub fn stats(&self) -> Vec<LaneStats> {
        self.monitor().stats()
    }

    /// The sockets that the stacks hold now, both arms of all lanes.
    #[must_use]
    pub fn sockets_open(&self) -> u64 {
        self.monitor().sockets_open()
    }

    /// Stops the lane threads and returns what each lane saw.
    #[must_use]
    pub fn stop(mut self) -> Vec<LaneReport> {
        let panicked = self.join();
        self.lanes
            .iter()
            .zip(panicked)
            .map(|(lane, panicked)| LaneReport {
                stats: lane.stats(),
                panicked,
            })
            .collect()
    }

    fn join(&mut self) -> Vec<bool> {
        for lane in self.lanes.iter() {
            lane.stop.store(true, Ordering::Release);
        }
        let mut panicked: Vec<bool> = self.threads.drain(..).map(|t| t.join().is_err()).collect();
        panicked.resize(self.lanes.len(), false);
        panicked
    }
}

impl Drop for Datapath {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

/// The leases of one arm of one lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmLeases {
    /// The lane.
    pub lane: usize,
    /// The side of the arm.
    pub side: Side,
    /// The name of the pool.
    pub pool: String,
    /// The hosts that hold an address now.
    pub bound: usize,
    /// The hosts of the slice of the lane.
    pub hosts: usize,
}

/// The leases of every leasing arm, at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeaseReport {
    /// One entry per leasing arm of each lane. Empty when no pool leases.
    pub arms: Vec<ArmLeases>,
}

impl LeaseReport {
    /// The hosts that hold an address, over all arms.
    #[must_use]
    pub fn bound(&self) -> usize {
        self.arms.iter().map(|a| a.bound).sum()
    }

    /// The hosts that lease, over all arms.
    #[must_use]
    pub fn hosts(&self) -> usize {
        self.arms.iter().map(|a| a.hosts).sum()
    }

    /// Whether every host holds an address. True when no pool leases.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.arms.iter().all(|a| a.bound == a.hosts)
    }

    /// The bound count of each arm, for an error message.
    #[must_use]
    pub fn describe(&self) -> String {
        self.arms
            .iter()
            .map(|a| {
                format!(
                    "lane {} {:?} pool {}: {} of {}",
                    a.lane, a.side, a.pool, a.bound, a.hosts
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A cloneable handle on the lease phase of a [`Datapath`] (TRN-37).
#[derive(Clone)]
pub struct LeaseMonitor {
    lanes: Arc<Vec<Running>>,
    timeout: Option<Duration>,
}

impl LeaseMonitor {
    /// How many hosts of each leasing arm hold an address now.
    #[must_use]
    pub fn report(&self) -> LeaseReport {
        let mut arms = Vec::new();
        for (lane, running) in self.lanes.iter().enumerate() {
            for (side, board) in [
                (Side::Client, &running.leases.client),
                (Side::Server, &running.leases.server),
            ] {
                if let Some(board) = board {
                    arms.push(ArmLeases {
                        lane,
                        side,
                        pool: board.pool().name().to_owned(),
                        bound: board.bound(),
                        hosts: board.count(),
                    });
                }
            }
        }
        LeaseReport { arms }
    }

    fn timed_out(&self, report: &LeaseReport, limit: Duration) -> DatapathError {
        DatapathError::LeaseTimeout {
            bound: report.bound(),
            hosts: report.hosts(),
            timeout_ms: u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
            detail: report.describe(),
        }
    }

    /// Waits for every lease, as [`Datapath::wait_leases`] does.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::LeaseTimeout`] when the limit passed.
    pub fn wait(&self) -> Result<LeaseReport, DatapathError> {
        let limit = self.timeout.unwrap_or_default();
        let deadline = std::time::Instant::now() + limit;
        loop {
            let report = self.report();
            if report.is_complete() {
                return Ok(report);
            }
            if std::time::Instant::now() >= deadline {
                return Err(self.timed_out(&report, limit));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Waits for every lease without blocking the thread, inside a Tokio
    /// runtime with time enabled.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::LeaseTimeout`] when the limit passed.
    pub async fn wait_async(&self) -> Result<LeaseReport, DatapathError> {
        let limit = self.timeout.unwrap_or_default();
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let report = self.report();
            if report.is_complete() {
                return Ok(report);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(self.timed_out(&report, limit));
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}

/// A cloneable handle on the counters of a [`Datapath`].
#[derive(Clone)]
pub struct Monitor {
    lanes: Arc<Vec<Running>>,
}

impl Monitor {
    /// The counters of every lane.
    #[must_use]
    pub fn stats(&self) -> Vec<LaneStats> {
        self.lanes.iter().map(Running::stats).collect()
    }

    /// The sockets that the stacks hold now, both arms of all lanes.
    #[must_use]
    pub fn sockets_open(&self) -> u64 {
        self.lanes
            .iter()
            .map(|l| l.stack.stats().sockets_open)
            .sum()
    }
}

/// Opens sessions from client hosts to server hosts, with the shape of
/// `fps_backend_kernel::Connector`.
#[derive(Clone)]
pub struct Connector {
    lanes: Arc<Vec<Running>>,
    timeout: Duration,
    /// Whether the datapath resolves the next hop, in which case a
    /// destination need not be a server host of the lane.
    resolving: bool,
}

impl std::fmt::Debug for Connector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connector")
            .field("lanes", &self.lanes.len())
            .field("timeout", &self.timeout)
            .field("resolving", &self.resolving)
            .finish()
    }
}

impl Connector {
    /// The lane whose client hosts include `source` and whose server hosts
    /// include `destination`. The two addresses are of one family, which is
    /// IPv4 or IPv6. A datapath that resolves the next hop takes a destination
    /// of the family of `source` that no pool serves, because the peer is
    /// behind a router.
    fn lane_of(&self, source: IpAddr, destination: IpAddr) -> Result<&Running, DatapathError> {
        let lane = self
            .lanes
            .iter()
            .find(|l| index_in(&l.client_hosts, l.leases.client.as_ref(), source).is_some())
            .ok_or(DatapathError::UnknownSource(source))?;
        let known =
            index_in(&lane.server_hosts, lane.leases.server.as_ref(), destination).is_some();
        let routed = self.resolving && source.is_ipv4() == destination.is_ipv4();
        if !known && !routed {
            return Err(DatapathError::UnknownDestination(destination));
        }
        Ok(lane)
    }

    /// Opens a session from the client host `source` to `destination`.
    ///
    /// `source` must be a client host of a lane and the address of
    /// `destination` a server host of the same lane (see [`split_pools`]). The
    /// stack picks the source port. The call must run inside a Tokio runtime
    /// with time enabled.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::Unsupported`] for IPv6,
    /// [`DatapathError::UnknownSource`] and [`DatapathError::UnknownDestination`]
    /// for addresses that no lane serves, [`DatapathError::ConnectTimeout`]
    /// when the handshake takes longer than the limit, and
    /// [`DatapathError::Stack`] for a refused or reset connection and for an
    /// exhausted table.
    pub async fn connect(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> Result<DatapathSession, DatapathError> {
        let lane = self.lane_of(source, destination.ip())?;
        let attempt = lane
            .stack
            .connect_addrs(source, destination.ip(), destination.port());
        match tokio::time::timeout(self.timeout, attempt).await {
            Ok(Ok(session)) => Ok(session),
            Ok(Err(source)) => Err(DatapathError::Stack {
                op: "connect",
                source,
            }),
            Err(_) => Err(DatapathError::ConnectTimeout {
                source_addr: source,
                destination,
                timeout_ms: u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX),
            }),
        }
    }

    /// Opens a UDP session from the client host `source` to `destination`,
    /// with the shape of `fps_backend_kernel::Connector::connect_datagram`.
    ///
    /// `source` must be a client host of a lane and the address of
    /// `destination` a server host of the same lane (see [`split_pools`]).
    /// The layer picks an ephemeral port of the source host. Nothing is sent
    /// and the call does not wait, because UDP has no handshake, and it
    /// needs no Tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::Unsupported`] for IPv6 and when the UDP layer
    /// is off, [`DatapathError::UnknownSource`] and
    /// [`DatapathError::UnknownDestination`] for addresses that no lane
    /// serves, and [`DatapathError::Stack`] when the layer refuses: a
    /// destination port of 0, or no free ephemeral port of the host.
    pub fn connect_udp(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> Result<DatapathDatagram, DatapathError> {
        let lane = self.lane_of(source, destination.ip())?;
        let arm = lane
            .udp_client
            .as_ref()
            .ok_or(DatapathError::Unsupported(Feature::Udp))?;
        arm.connect(source, destination)
            .map_err(|source| DatapathError::Stack {
                op: "connect_udp",
                source,
            })
    }
}

/// The accept side of [`Datapath::listen`].
pub struct Listener {
    port: u16,
    rx: mpsc::Receiver<io::Result<OwnSession>>,
    tasks: Vec<TaskHandle<()>>,
    /// The stacks that listen on the port, which the drop stops.
    stacks: Vec<OwnStack>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("port", &self.port)
            .field("lanes", &self.stacks.len())
            .finish()
    }
}

impl Listener {
    /// The port that the listener serves.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Completes with the next session that arrived at a server host, and
    /// the address of its peer (a client host and its port), as
    /// `KernelListener::accept` does.
    ///
    /// # Errors
    ///
    /// Returns [`DatapathError::Stack`] when a lane failed to accept, and the
    /// same error with the kind [`io::ErrorKind::BrokenPipe`] when the
    /// accepting tasks ended.
    pub async fn accept(&mut self) -> Result<(DatapathSession, SocketAddr), DatapathError> {
        let fail = |source| DatapathError::Stack {
            op: "accept",
            source,
        };
        match self.rx.recv().await {
            Some(Ok(session)) => {
                let peer = session.peer_addr().map_err(fail)?;
                Ok((session, peer))
            }
            Some(Err(source)) => Err(fail(source)),
            None => Err(fail(io::ErrorKind::BrokenPipe.into())),
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        for stack in &self.stacks {
            stack.unlisten(self.port);
        }
    }
}
