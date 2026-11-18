// SPDX-License-Identifier: GPL-3.0-only
//! The UDP layer of one queue: the session table, the listeners, the ephemeral
//! ports, the frames that wait for the queue, and the counters.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use fps_tcp_own::net::{decode_header, encode_header, Station, ETHERTYPE_IPV4, MIN_FRAME_LEN};

use crate::lock;
use crate::session::{AcceptQueue, Push, SessionQueue, UdpSession};
use crate::wire::{self, Outgoing, RxError, TooLong};

/// The smallest send queue. A datagram of 65,507 bytes is 45 fragments at an
/// MTU of 1,500 and 8,000 at the smallest MTU, so the floor keeps the common
/// case moving, and a datagram that needs more frames than the queue holds
/// waits for an empty queue.
const MIN_SEND_QUEUE: usize = 128;

/// Settings of a [`UdpArm`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UdpConfig {
    /// The first and the last ephemeral port of a client session, both
    /// included. Each host of the arm walks the range on its own.
    pub ephemeral_ports: (u16, u16),
    /// The largest IPv4 packet that the layer sends in one frame. A larger
    /// datagram leaves as fragments. The datapath lowers it to what the queue
    /// carries.
    pub mtu: usize,
    /// The time to live of the packets that the layer sends.
    pub ttl: u8,
    /// The datagrams that a session holds for its reader. A datagram that
    /// arrives at a full queue is dropped and counted.
    pub recv_queue: usize,
    /// The sessions that a listener holds for `accept`.
    pub accept_queue: usize,
    /// The frames that wait for the queue. A send that does not fit waits
    /// until the loop has taken frames.
    pub send_queue: usize,
}

impl Default for UdpConfig {
    /// Ephemeral ports 32768 to 60999 (the range of Linux), an MTU of 1,500,
    /// a TTL of 64, 64 datagrams per session (as `fps-backend-kernel`), 1,024
    /// sessions per listener, and 1,024 frames in the send queue.
    fn default() -> Self {
        Self {
            ephemeral_ports: (32768, 60999),
            mtu: 1500,
            ttl: 64,
            recv_queue: 64,
            accept_queue: 1024,
            send_queue: 1024,
        }
    }
}

/// What a [`UdpArm`] did, since it was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UdpStats {
    /// Frames that [`UdpArm::receive`] took.
    pub rx_frames: u64,
    /// Datagrams queued at a session.
    pub rx_delivered: u64,
    /// Payload bytes of those datagrams.
    pub rx_delivered_bytes: u64,
    /// Sessions that a listener opened for a new peer.
    pub rx_accepted: u64,
    /// Datagrams for a port that nothing listens on and no session owns.
    pub rx_no_listener: u64,
    /// Datagrams dropped because the queue of their session was full or the
    /// session had just closed.
    pub rx_queue_full: u64,
    /// Datagrams dropped because the accept queue of the listener was full.
    pub rx_accept_full: u64,
    /// Frames with a length that contradicts their headers.
    pub rx_malformed: u64,
    /// Frames with a wrong IPv4 header checksum.
    pub rx_bad_ip_checksum: u64,
    /// Frames with a wrong UDP checksum.
    pub rx_bad_udp_checksum: u64,
    /// IPv4 fragments, which the layer does not reassemble.
    pub rx_fragments: u64,
    /// Frames for an address that is not a host of the arm, and frames that
    /// are not IPv4 UDP.
    pub rx_not_ours: u64,
    /// Datagrams that sessions sent.
    pub tx_datagrams: u64,
    /// Frames that those datagrams made.
    pub tx_frames: u64,
    /// Payload bytes of the sent datagrams.
    pub tx_bytes: u64,
    /// Datagrams that left as more than one fragment.
    pub tx_fragmented: u64,
    /// Sends that had to wait for room in the send queue.
    pub tx_blocked: u64,
    /// Sessions that are open now.
    pub sessions_open: u64,
    /// Frames that wait for the queue now.
    pub tx_queued: u64,
}

#[derive(Default)]
struct Counters {
    rx_frames: AtomicU64,
    rx_delivered: AtomicU64,
    rx_delivered_bytes: AtomicU64,
    rx_accepted: AtomicU64,
    rx_no_listener: AtomicU64,
    rx_queue_full: AtomicU64,
    rx_accept_full: AtomicU64,
    rx_malformed: AtomicU64,
    rx_bad_ip_checksum: AtomicU64,
    rx_bad_udp_checksum: AtomicU64,
    rx_fragments: AtomicU64,
    rx_not_ours: AtomicU64,
    tx_datagrams: AtomicU64,
    tx_frames: AtomicU64,
    tx_bytes: AtomicU64,
    tx_fragmented: AtomicU64,
    tx_blocked: AtomicU64,
}

fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

/// The identity of a session: the local and the peer address and port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) local: u32,
    pub(crate) local_port: u16,
    pub(crate) peer: u32,
    pub(crate) peer_port: u16,
}

impl Key {
    pub(crate) fn local_addr(self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::from(self.local),
            self.local_port,
        ))
    }

    pub(crate) fn peer_addr(self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(self.peer), self.peer_port))
    }
}

struct Tables {
    station: Station,
    sessions: HashMap<Key, Arc<SessionQueue>>,
    listeners: HashMap<u16, Arc<AcceptQueue>>,
    /// The next ephemeral port to try, per local host.
    cursors: HashMap<u32, u16>,
}

#[derive(Default)]
struct Outbox {
    frames: VecDeque<Vec<u8>>,
    wakers: Vec<Waker>,
}

/// The state that sessions and the datapath loop share.
pub(crate) struct Shared {
    config: UdpConfig,
    tables: Mutex<Tables>,
    outbox: Mutex<Outbox>,
    ip_id: AtomicU16,
    counters: Counters,
}

/// The UDP layer of one queue. Cloning gives another handle on the same layer.
#[derive(Clone)]
pub struct UdpArm {
    pub(crate) shared: Arc<Shared>,
}

impl std::fmt::Debug for UdpArm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpArm")
            .field("config", &self.shared.config)
            .field("stats", &self.stats())
            .finish()
    }
}

impl UdpArm {
    /// A layer for the hosts of `station`, which also says which neighbors
    /// the hosts talk to and which VLAN the port has.
    #[must_use]
    pub fn new(station: Station, mut config: UdpConfig) -> Self {
        config.send_queue = config.send_queue.max(MIN_SEND_QUEUE);
        config.recv_queue = config.recv_queue.max(1);
        config.accept_queue = config.accept_queue.max(1);
        config.mtu = config.mtu.max(wire::MIN_MTU);
        if config.ephemeral_ports.0 > config.ephemeral_ports.1 {
            config.ephemeral_ports = (config.ephemeral_ports.1, config.ephemeral_ports.0);
        }
        config.ephemeral_ports.0 = config.ephemeral_ports.0.max(1);
        Self {
            shared: Arc::new(Shared {
                config,
                tables: Mutex::new(Tables {
                    station,
                    sessions: HashMap::new(),
                    listeners: HashMap::new(),
                    cursors: HashMap::new(),
                }),
                outbox: Mutex::new(Outbox::default()),
                ip_id: AtomicU16::new(0),
                counters: Counters::default(),
            }),
        }
    }

    /// The settings after the layer raised values that are too small.
    #[must_use]
    pub fn config(&self) -> &UdpConfig {
        &self.shared.config
    }

    /// Takes one Ethernet frame that carries IPv4 UDP, verifies it, and
    /// queues its datagram at the session that owns it or at a new session of
    /// a listener. A frame that fails a check is dropped and counted. The
    /// call never blocks on a task: it only takes short locks.
    pub fn receive(&self, frame: &[u8]) {
        let shared = &*self.shared;
        let c = &shared.counters;
        bump(&c.rx_frames);
        let Ok(eth) = decode_header(frame) else {
            bump(&c.rx_malformed);
            return;
        };
        let Some(ip) = frame.get(eth.payload_at..) else {
            bump(&c.rx_malformed);
            return;
        };
        let parsed = match wire::parse(ip) {
            Ok(p) => p,
            Err(error) => {
                bump(match error {
                    RxError::Malformed => &c.rx_malformed,
                    RxError::IpChecksum => &c.rx_bad_ip_checksum,
                    RxError::UdpChecksum => &c.rx_bad_udp_checksum,
                    RxError::Fragment => &c.rx_fragments,
                    RxError::NotIpv4 | RxError::NotUdp => &c.rx_not_ours,
                });
                return;
            }
        };
        let key = Key {
            local: u32::from(parsed.dst_ip),
            local_port: parsed.dst_port,
            peer: u32::from(parsed.src_ip),
            peer_port: parsed.src_port,
        };
        let route = {
            let t = lock(&shared.tables);
            if let Some(queue) = t.sessions.get(&key) {
                Route::Session(Arc::clone(queue))
            } else if let Some(src_mac) = t.station.local_mac(key.local) {
                match t.listeners.get(&key.local_port) {
                    Some(listener) => {
                        // The reply goes to the neighbor of the table, or to
                        // the sender of this frame when the table lacks it.
                        let dst_mac = t.station.resolve(key.peer).unwrap_or(eth.src);
                        let mut head = Vec::with_capacity(18);
                        encode_header(
                            &mut head,
                            dst_mac,
                            src_mac,
                            t.station.vlan(),
                            ETHERTYPE_IPV4,
                        );
                        Route::Listener(Arc::clone(listener), head)
                    }
                    None => Route::None(&c.rx_no_listener),
                }
            } else {
                Route::None(&c.rx_not_ours)
            }
        };
        match route {
            Route::None(counter) => bump(counter),
            Route::Session(queue) => self.deliver(&queue, parsed.payload),
            Route::Listener(accept, head) => self.open(&accept, key, head, parsed.payload),
        }
    }

    fn deliver(&self, queue: &SessionQueue, payload: &[u8]) {
        let c = &self.shared.counters;
        match queue.push(payload, self.shared.config.recv_queue) {
            Push::Queued => {
                bump(&c.rx_delivered);
                c.rx_delivered_bytes
                    .fetch_add(payload.len() as u64, Ordering::Relaxed);
            }
            Push::Full | Push::Closed => bump(&c.rx_queue_full),
        }
    }

    /// Opens a server session for the first datagram of a new peer and hands
    /// it to the listener.
    fn open(&self, accept: &Arc<AcceptQueue>, key: Key, head: Vec<u8>, payload: &[u8]) {
        let c = &self.shared.counters;
        let queue = Arc::new(SessionQueue::new());
        let raced = {
            let mut t = lock(&self.shared.tables);
            match t.sessions.entry(key) {
                Entry::Occupied(e) => Some(Arc::clone(e.get())),
                Entry::Vacant(v) => {
                    v.insert(Arc::clone(&queue));
                    None
                }
            }
        };
        if let Some(existing) = raced {
            // A session for this peer opened since the lookup.
            self.deliver(&existing, payload);
            return;
        }
        // The queue is empty and open, so the push cannot fail.
        let _ = queue.push(payload, self.shared.config.recv_queue);
        let session = UdpSession::new(Arc::clone(&self.shared), queue, key, head);
        match accept.push(session, key.peer_addr()) {
            Ok(()) => {
                bump(&c.rx_accepted);
                bump(&c.rx_delivered);
                c.rx_delivered_bytes
                    .fetch_add(payload.len() as u64, Ordering::Relaxed);
            }
            // Dropping the session removes it from the table, with no lock
            // held here.
            Err(session) => {
                bump(&c.rx_accept_full);
                drop(session);
            }
        }
    }

    /// Hands up to `max` waiting frames to `sink`, oldest first, and returns
    /// how many it gave. A send that waited for room wakes when the queue is
    /// half empty.
    pub fn drain_tx(&self, max: usize, sink: &mut dyn FnMut(Vec<u8>)) -> usize {
        let (given, wake) = {
            let mut outbox = lock(&self.shared.outbox);
            let given = max.min(outbox.frames.len());
            for frame in outbox.frames.drain(..given) {
                sink(frame);
            }
            let wake = if outbox.frames.len() <= self.shared.config.send_queue / 2 {
                std::mem::take(&mut outbox.wakers)
            } else {
                Vec::new()
            };
            (given, wake)
        };
        for waker in wake {
            waker.wake();
        }
        given
    }

    /// Whether frames wait for the queue.
    #[must_use]
    pub fn has_tx(&self) -> bool {
        !lock(&self.shared.outbox).frames.is_empty()
    }

    /// Opens a session from the local host `source` to `destination` with an
    /// ephemeral port of that host. Nothing is sent: UDP has no handshake.
    ///
    /// # Errors
    ///
    /// * `InvalidInput` for a destination port of 0.
    /// * `Unsupported` for an IPv6 destination (TRN-30).
    /// * `AddrNotAvailable` when `source` is not a host of the arm.
    /// * `HostUnreachable` when the neighbor table has no MAC address for the
    ///   destination.
    /// * `AddrInUse` when every ephemeral port of the host is taken toward
    ///   this destination.
    pub fn connect(&self, source: Ipv4Addr, destination: SocketAddr) -> io::Result<UdpSession> {
        let SocketAddr::V4(dst) = destination else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "IPv6 UDP is not served by the user space layer yet (TRN-30)",
            ));
        };
        if dst.port() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the destination port is 0",
            ));
        }
        let (local, peer) = (u32::from(source), u32::from(*dst.ip()));
        let (first, last) = self.shared.config.ephemeral_ports;
        let mut t = lock(&self.shared.tables);
        if !t.station.owns(local) {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("{source} is not a host of this port"),
            ));
        }
        let mut head = Vec::with_capacity(18);
        if !t.station.head(&mut head, local, peer) {
            return Err(io::Error::new(
                io::ErrorKind::HostUnreachable,
                format!("no neighbor entry for {}", dst.ip()),
            ));
        }
        let mut port = t.cursors.get(&local).copied().unwrap_or(first);
        if !(first..=last).contains(&port) {
            port = first;
        }
        let mut chosen = None;
        for _ in 0..=u32::from(last - first) {
            let key = Key {
                local,
                local_port: port,
                peer,
                peer_port: dst.port(),
            };
            let next = if port >= last { first } else { port + 1 };
            if !t.sessions.contains_key(&key) && !t.listeners.contains_key(&port) {
                chosen = Some((key, next));
                break;
            }
            port = next;
        }
        let Some((key, next)) = chosen else {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("every ephemeral port of {source} is in use toward {destination}"),
            ));
        };
        t.cursors.insert(local, next);
        let queue = Arc::new(SessionQueue::new());
        t.sessions.insert(key, Arc::clone(&queue));
        drop(t);
        Ok(UdpSession::new(Arc::clone(&self.shared), queue, key, head))
    }

    /// The counters of the layer.
    #[must_use]
    pub fn stats(&self) -> UdpStats {
        let c = &self.shared.counters;
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let sessions_open = lock(&self.shared.tables).sessions.len() as u64;
        let tx_queued = lock(&self.shared.outbox).frames.len() as u64;
        UdpStats {
            rx_frames: get(&c.rx_frames),
            rx_delivered: get(&c.rx_delivered),
            rx_delivered_bytes: get(&c.rx_delivered_bytes),
            rx_accepted: get(&c.rx_accepted),
            rx_no_listener: get(&c.rx_no_listener),
            rx_queue_full: get(&c.rx_queue_full),
            rx_accept_full: get(&c.rx_accept_full),
            rx_malformed: get(&c.rx_malformed),
            rx_bad_ip_checksum: get(&c.rx_bad_ip_checksum),
            rx_bad_udp_checksum: get(&c.rx_bad_udp_checksum),
            rx_fragments: get(&c.rx_fragments),
            rx_not_ours: get(&c.rx_not_ours),
            tx_datagrams: get(&c.tx_datagrams),
            tx_frames: get(&c.tx_frames),
            tx_bytes: get(&c.tx_bytes),
            tx_fragmented: get(&c.tx_fragmented),
            tx_blocked: get(&c.tx_blocked),
            sessions_open,
            tx_queued,
        }
    }
}

/// Where a received datagram goes.
enum Route<'a> {
    None(&'a AtomicU64),
    Session(Arc<SessionQueue>),
    Listener(Arc<AcceptQueue>, Vec<u8>),
}

impl Shared {
    /// Queues the frames of one datagram, or registers the waker of a send
    /// that has to wait for room.
    pub(crate) fn send(
        &self,
        cx: &mut Context<'_>,
        head: &[u8],
        key: Key,
        payload: &[u8],
    ) -> Poll<io::Result<()>> {
        let needed = wire::frames_for(payload.len(), self.config.mtu);
        {
            let mut outbox = lock(&self.outbox);
            let queued = outbox.frames.len();
            if queued > 0 && queued + needed > self.config.send_queue {
                if !outbox.wakers.iter().any(|w| w.will_wake(cx.waker())) {
                    outbox.wakers.push(cx.waker().clone());
                }
                bump(&self.counters.tx_blocked);
                return Poll::Pending;
            }
        }
        let datagram = Outgoing {
            src_ip: Ipv4Addr::from(key.local),
            dst_ip: Ipv4Addr::from(key.peer),
            src_port: key.local_port,
            dst_port: key.peer_port,
            ttl: self.config.ttl,
            id: self.ip_id.fetch_add(1, Ordering::Relaxed),
            payload,
        };
        let mut frames = Vec::with_capacity(needed);
        if let Err(TooLong) =
            wire::build(head, &datagram, self.config.mtu, MIN_FRAME_LEN, &mut frames)
        {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("a datagram of {} bytes does not fit IPv4", payload.len()),
            )));
        }
        let c = &self.counters;
        bump(&c.tx_datagrams);
        c.tx_frames
            .fetch_add(frames.len() as u64, Ordering::Relaxed);
        c.tx_bytes
            .fetch_add(payload.len() as u64, Ordering::Relaxed);
        if frames.len() > 1 {
            bump(&c.tx_fragmented);
        }
        lock(&self.outbox).frames.extend(frames);
        Poll::Ready(Ok(()))
    }

    /// Removes a session from the table, if `queue` is the one registered.
    pub(crate) fn remove(&self, key: &Key, queue: &Arc<SessionQueue>) {
        let removed = {
            let mut t = lock(&self.tables);
            match t.sessions.get(key) {
                Some(registered) if Arc::ptr_eq(registered, queue) => t.sessions.remove(key),
                _ => None,
            }
        };
        drop(removed);
    }

    /// Registers `queue` as the listener of `port`.
    pub(crate) fn listen(&self, port: u16, queue: &Arc<AcceptQueue>) -> io::Result<()> {
        if port == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the listen port is 0",
            ));
        }
        match lock(&self.tables).listeners.entry(port) {
            Entry::Occupied(_) => Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("UDP port {port} already has a listener"),
            )),
            Entry::Vacant(v) => {
                v.insert(Arc::clone(queue));
                Ok(())
            }
        }
    }

    /// Removes the listener of `port` if it is `queue`.
    pub(crate) fn unlisten(&self, port: u16, queue: &Arc<AcceptQueue>) {
        let removed = {
            let mut t = lock(&self.tables);
            match t.listeners.get(&port) {
                Some(registered) if Arc::ptr_eq(registered, queue) => t.listeners.remove(&port),
                _ => None,
            }
        };
        drop(removed);
    }

    pub(crate) fn accept_queue(&self) -> usize {
        self.config.accept_queue
    }
}
