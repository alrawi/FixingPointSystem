// SPDX-License-Identifier: GPL-3.0-only
//! One host of the own stack: the connection table, the TCP state machine,
//! the timers, and the frames that wait for the link.
//!
//! A host does no I/O. [`Host::input`] consumes a frame, [`Host::expire`]
//! fires the timers that are due, [`Host::flush_dirty`] turns the pending work
//! of connections into frames, and the owner moves the frames of
//! [`Host::out`] to the link. The clock is an argument everywhere, so tests
//! fix the time.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::io;
use std::task::Waker;

use super::wire::{self, flags, Header};
use super::OwnConfig;

/// The MSS that a peer is assumed to have when its SYN carries no option.
const DEFAULT_PEER_MSS: u16 = 536;
/// The first port of the ephemeral range of an active open.
const EPHEMERAL_FIRST: u16 = 10_000;

/// The state of a connection, as RFC 9293 names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The SYN is out and the SYN-ACK is awaited.
    SynSent,
    /// The SYN-ACK is out and the final ACK is awaited.
    SynReceived,
    /// Data flows both ways.
    Established,
    /// The own FIN is out and unacknowledged.
    FinWait1,
    /// The own FIN is acknowledged and the FIN of the peer is awaited.
    FinWait2,
    /// The peer closed and the application has not.
    CloseWait,
    /// Both FINs crossed and the own one is unacknowledged.
    Closing,
    /// The application closed after the peer and the FIN is unacknowledged.
    LastAck,
    /// Both sides closed and the tuple is held back for a while.
    TimeWait,
    /// The connection is over.
    Closed,
}

/// A hasher for keys that are small integers, which the table of a host uses
/// in place of SipHash, because the keys are not chosen by an attacker.
#[derive(Default)]
pub struct IntHasher(u64);

impl Hasher for IntHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x517C_C1B7_2722_0A95);
        }
    }
    fn write_u32(&mut self, value: u32) {
        self.0 = u64::from(value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    fn write_u16(&mut self, value: u16) {
        self.0 = u64::from(value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type IntMap<K, V> = HashMap<K, V, BuildHasherDefault<IntHasher>>;

fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

fn seq_le(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) <= 0
}

fn tuple_key(local_port: u16, remote_port: u16) -> u32 {
    (u32::from(local_port) << 16) | u32::from(remote_port)
}

/// One connection.
struct Conn {
    state: State,
    local_port: u16,
    remote_port: u16,
    iss: u32,
    snd_una: u32,
    snd_nxt: u32,
    snd_wnd: u32,
    mss: u16,
    rcv_nxt: u32,
    sndbuf: VecDeque<u8>,
    rcvbuf: VecDeque<u8>,
    fin_wanted: bool,
    sent_fin: bool,
    fin_rcvd: bool,
    app_alive: bool,
    dirty: bool,
    ack_pending: bool,
    syn_pending: bool,
    probe: bool,
    armed: bool,
    in_heap: bool,
    in_backlog: bool,
    retries: u8,
    rto_us: i64,
    deadline: i64,
    /// The listener whose accept queue the connection belongs to, or 0.
    listener: u16,
    error: Option<io::ErrorKind>,
    read_waker: Option<Waker>,
    write_waker: Option<Waker>,
}

impl Conn {
    fn new(state: State, local_port: u16, remote_port: u16, iss: u32, rto_us: i64) -> Self {
        Self {
            state,
            local_port,
            remote_port,
            iss,
            snd_una: iss,
            snd_nxt: iss.wrapping_add(1),
            snd_wnd: 0,
            mss: 0,
            rcv_nxt: 0,
            sndbuf: VecDeque::new(),
            rcvbuf: VecDeque::new(),
            fin_wanted: false,
            sent_fin: false,
            fin_rcvd: false,
            app_alive: true,
            dirty: false,
            ack_pending: false,
            syn_pending: true,
            probe: false,
            armed: false,
            in_heap: false,
            in_backlog: false,
            retries: 0,
            rto_us,
            deadline: 0,
            listener: 0,
            error: None,
            read_waker: None,
            write_waker: None,
        }
    }

    /// The sequence space that was sent and is not acknowledged.
    fn in_flight(&self) -> u32 {
        self.snd_nxt.wrapping_sub(self.snd_una)
    }

    /// The data bytes that were sent and are not acknowledged.
    fn unacked_data(&self) -> usize {
        (self.in_flight() - u32::from(self.sent_fin)) as usize
    }

    fn outstanding(&self) -> bool {
        self.in_flight() > 0
    }
}

/// A listening port.
#[derive(Default)]
struct Listener {
    queue: VecDeque<(u32, u32)>,
    waiters: BTreeMap<u64, Waker>,
    /// The connections that are not accepted yet, handshaking or queued.
    pending: usize,
    next_waiter: u64,
}

/// The counters of one host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostCounters {
    /// Frames that the host queued for the link.
    pub frames_out: u64,
    /// Frames that the host took from the link.
    pub frames_in: u64,
    /// Frames that failed a length or checksum test.
    pub bad_frames: u64,
    /// Segments sent again after a timeout.
    pub retransmits: u64,
    /// Resets that the host sent.
    pub resets_sent: u64,
    /// Connections that a timeout or a reset ended abnormally.
    pub aborted: u64,
}

/// The frames that wait for the link.
pub struct Out {
    /// The built frames, oldest first.
    pub queue: VecDeque<Vec<u8>>,
    ip: u32,
    peer: u32,
}

impl Out {
    #[allow(clippy::too_many_arguments)]
    fn send(
        &mut self,
        counters: &mut HostCounters,
        local_port: u16,
        remote_port: u16,
        seq: u32,
        ack: u32,
        flag_bits: u8,
        window: u16,
        mss: Option<u16>,
        payload: (&[u8], &[u8]),
    ) {
        let header = Header {
            src_ip: self.ip,
            dst_ip: self.peer,
            src_port: local_port,
            dst_port: remote_port,
            seq,
            ack,
            flags: flag_bits,
            window,
            mss,
        };
        let mut frame = Vec::new();
        wire::encode(&mut frame, &header, payload);
        self.queue.push_back(frame);
        counters.frames_out += 1;
    }
}

/// One host: its address, connections, listeners, timers, and output queue.
pub struct Host {
    ip: u32,
    peer: u32,
    cfg: OwnConfig,
    mss_own: u16,
    conns: Vec<Option<Conn>>,
    gens: Vec<u32>,
    free: Vec<u32>,
    live: u64,
    table: IntMap<u32, u32>,
    listeners: IntMap<u16, Listener>,
    dirty: Vec<u32>,
    timers: BinaryHeap<Reverse<(i64, u32, u32)>>,
    next_port: u16,
    /// The frames that wait for the link.
    pub out: Out,
    /// The counters of the host.
    pub counters: HostCounters,
}

/// The mixing function of the initial sequence number, splitmix64.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

impl Host {
    /// Creates a host with the address `ip` that talks to the host `peer`.
    pub fn new(ip: u32, peer: u32, cfg: OwnConfig, mtu: usize) -> Self {
        let mss_own = u16::try_from(mtu.saturating_sub(wire::IP_LEN + wire::TCP_LEN))
            .unwrap_or(u16::MAX)
            .max(64);
        Self {
            ip,
            peer,
            mss_own,
            cfg,
            conns: Vec::new(),
            gens: Vec::new(),
            free: Vec::new(),
            live: 0,
            table: IntMap::default(),
            listeners: IntMap::default(),
            dirty: Vec::new(),
            timers: BinaryHeap::new(),
            next_port: EPHEMERAL_FIRST,
            out: Out {
                queue: VecDeque::new(),
                ip,
                peer,
            },
            counters: HostCounters::default(),
        }
    }

    /// The sockets of the host, connections plus listeners.
    pub fn sockets(&self) -> u64 {
        self.live + self.listeners.len() as u64
    }

    /// The deadline of the earliest timer, which may be stale.
    pub fn next_deadline(&self) -> Option<i64> {
        self.timers
            .peek()
            .map(|Reverse((deadline, _, _))| *deadline)
    }

    /// True when connections have work that [`Host::flush_dirty`] would do.
    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// The initial sequence number of a connection, after RFC 6528: a
    /// counter that ticks every 4 microseconds plus a hash of the tuple and a
    /// secret. The hash is a mixing function and not a keyed cryptographic one.
    fn isn(&self, local_port: u16, remote_port: u16, now: i64) -> u32 {
        let tuple =
            (u64::from(self.ip) << 32) ^ (u64::from(local_port) << 16) ^ u64::from(remote_port);
        let hash = mix(self.cfg.isn_secret ^ mix(tuple ^ (u64::from(self.peer) << 17)));
        ((now / 4) as u32).wrapping_add(hash as u32)
    }

    fn conn(&mut self, id: u32) -> Option<&mut Conn> {
        self.conns.get_mut(id as usize).and_then(Option::as_mut)
    }

    fn alloc(&mut self, conn: Conn) -> (u32, u32) {
        self.live += 1;
        if let Some(id) = self.free.pop() {
            self.conns[id as usize] = Some(conn);
            return (id, self.gens[id as usize]);
        }
        self.conns.push(Some(conn));
        self.gens.push(0);
        ((self.conns.len() - 1) as u32, 0)
    }

    fn release(&mut self, id: u32) {
        if let Some(slot) = self.conns.get_mut(id as usize) {
            if slot.take().is_some() {
                self.gens[id as usize] = self.gens[id as usize].wrapping_add(1);
                self.free.push(id);
                self.live -= 1;
            }
        }
    }

    /// Checks that `(id, gen)` still names a connection.
    pub fn valid(&self, id: u32, gen: u32) -> bool {
        self.gens.get(id as usize) == Some(&gen)
            && self.conns.get(id as usize).is_some_and(Option::is_some)
    }

    fn mark_dirty(&mut self, id: u32) {
        if let Some(conn) = self.conn(id) {
            if !conn.dirty {
                conn.dirty = true;
                self.dirty.push(id);
            }
        }
    }

    fn arm(&mut self, id: u32, gen: u32, deadline: i64) {
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        conn.deadline = deadline;
        conn.armed = true;
        if !conn.in_heap {
            conn.in_heap = true;
            self.timers.push(Reverse((deadline, id, gen)));
        }
    }

    fn reply_reset(&mut self, h: &Header, seg_len: u32) {
        if h.flags & flags::RST != 0 {
            return;
        }
        self.counters.resets_sent += 1;
        let (seq, ack, bits) = if h.flags & flags::ACK != 0 {
            (h.ack, 0, flags::RST)
        } else {
            (0, h.seq.wrapping_add(seg_len), flags::RST | flags::ACK)
        };
        self.out.send(
            &mut self.counters,
            h.dst_port,
            h.src_port,
            seq,
            ack,
            bits,
            0,
            None,
            (&[], &[]),
        );
    }

    // ----------------------------------------------------------------------
    // The application side.

    /// Starts listening on `port`.
    pub fn listen(&mut self, port: u16) -> io::Result<()> {
        if self.listeners.contains_key(&port) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("port {port} is already listened on"),
            ));
        }
        self.listeners.insert(port, Listener::default());
        Ok(())
    }

    /// Starts an active open to `remote_port` of the peer and returns the
    /// connection handle.
    pub fn open(&mut self, remote_port: u16, now: i64) -> io::Result<(u32, u32)> {
        let range = u32::from(u16::MAX - EPHEMERAL_FIRST) + 1;
        let mut local_port = None;
        for _ in 0..range {
            let candidate = self.next_port;
            self.next_port = if candidate == u16::MAX {
                EPHEMERAL_FIRST
            } else {
                candidate + 1
            };
            if !self.table.contains_key(&tuple_key(candidate, remote_port)) {
                local_port = Some(candidate);
                break;
            }
        }
        let Some(local_port) = local_port else {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "every local port is in use for this destination",
            ));
        };
        let iss = self.isn(local_port, remote_port, now);
        let mut conn = Conn::new(
            State::SynSent,
            local_port,
            remote_port,
            iss,
            self.cfg.rto_us,
        );
        conn.mss = self.mss_own;
        let (id, gen) = self.alloc(conn);
        self.table.insert(tuple_key(local_port, remote_port), id);
        self.mark_dirty(id);
        Ok((id, gen))
    }

    /// Takes the next accepted connection of `port`, or registers `waker`
    /// under the ticket `ticket` and returns `None`.
    pub fn accept(
        &mut self,
        port: u16,
        ticket: &mut Option<u64>,
        waker: &Waker,
    ) -> io::Result<Option<(u32, u32)>> {
        let Some(listener) = self.listeners.get_mut(&port) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("port {port} is not listened on"),
            ));
        };
        if let Some(handle) = listener.queue.pop_front() {
            if let Some(t) = ticket.take() {
                listener.waiters.remove(&t);
            }
            listener.pending = listener.pending.saturating_sub(1);
            if let Some(conn) = self.conn(handle.0) {
                conn.in_backlog = false;
            }
            return Ok(Some(handle));
        }
        let t = *ticket.get_or_insert_with(|| {
            listener.next_waiter += 1;
            listener.next_waiter
        });
        listener.waiters.insert(t, waker.clone());
        Ok(None)
    }

    /// Removes the waiter of an accept that was dropped.
    pub fn cancel_accept(&mut self, port: u16, ticket: u64) {
        if let Some(listener) = self.listeners.get_mut(&port) {
            listener.waiters.remove(&ticket);
        }
    }

    /// What a read of a connection finds.
    pub fn read(
        &mut self,
        id: u32,
        gen: u32,
        buf: &mut [u8],
        waker: &Waker,
    ) -> Option<io::Result<usize>> {
        if !self.valid(id, gen) {
            return Some(Err(not_connected()));
        }
        let rcv_cap = self.cfg.rcv_buffer;
        let conn = self.conn(id)?;
        if let Some(kind) = conn.error {
            return Some(Err(io::Error::new(kind, "the connection failed")));
        }
        if !conn.rcvbuf.is_empty() {
            let take = buf.len().min(conn.rcvbuf.len());
            let (a, b) = conn.rcvbuf.as_slices();
            let first = a.len().min(take);
            buf[..first].copy_from_slice(&a[..first]);
            if take > first {
                buf[first..take].copy_from_slice(&b[..take - first]);
            }
            conn.rcvbuf.drain(..take);
            if conn.rcvbuf.is_empty() {
                conn.rcvbuf = VecDeque::new();
            }
            // Tell the peer when the window opened by a worthwhile amount.
            let space = rcv_cap.saturating_sub(conn.rcvbuf.len());
            let before = space.saturating_sub(take);
            let step = usize::from(conn.mss).min(rcv_cap / 2).max(1);
            if before < step
                && space >= step
                && !matches!(conn.state, State::Closed | State::TimeWait)
            {
                conn.ack_pending = true;
                self.mark_dirty(id);
            }
            return Some(Ok(take));
        }
        if conn.fin_rcvd || conn.state == State::Closed {
            return Some(Ok(0));
        }
        conn.read_waker = Some(waker.clone());
        None
    }

    /// What a write of a connection does.
    pub fn write(
        &mut self,
        id: u32,
        gen: u32,
        data: &[u8],
        waker: &Waker,
    ) -> Option<io::Result<usize>> {
        if !self.valid(id, gen) {
            return Some(Err(not_connected()));
        }
        let cap = self.cfg.snd_buffer;
        let conn = self.conn(id)?;
        if let Some(kind) = conn.error {
            return Some(Err(io::Error::new(kind, "the connection failed")));
        }
        if conn.fin_wanted || !matches!(conn.state, State::Established | State::CloseWait) {
            return Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the write side is closed",
            )));
        }
        let space = cap.saturating_sub(conn.sndbuf.len());
        if space == 0 {
            conn.write_waker = Some(waker.clone());
            return None;
        }
        let take = space.min(data.len());
        conn.sndbuf.extend(&data[..take]);
        self.mark_dirty(id);
        Some(Ok(take))
    }

    /// Closes the write side, which sends a FIN after the queued data.
    pub fn shutdown(&mut self, id: u32, gen: u32) -> io::Result<()> {
        if !self.valid(id, gen) {
            return Err(not_connected());
        }
        let Some(conn) = self.conn(id) else {
            return Ok(());
        };
        if let Some(kind) = conn.error {
            return Err(io::Error::new(kind, "the connection failed"));
        }
        if matches!(conn.state, State::Established | State::CloseWait) {
            conn.fin_wanted = true;
            self.mark_dirty(id);
        }
        Ok(())
    }

    /// Whether the connection has left the handshake, and how it ended when it
    /// failed there.
    pub fn connected(&mut self, id: u32, gen: u32, waker: &Waker) -> Option<io::Result<()>> {
        if !self.valid(id, gen) {
            return Some(Err(not_connected()));
        }
        let conn = self.conn(id)?;
        if let Some(kind) = conn.error {
            return Some(Err(io::Error::new(kind, "the connection failed")));
        }
        if conn.state == State::SynSent {
            conn.write_waker = Some(waker.clone());
            return None;
        }
        Some(Ok(()))
    }

    /// Ends the connection with a reset and discards its data.
    pub fn abort(&mut self, id: u32, gen: u32) {
        if !self.valid(id, gen) {
            return;
        }
        let Some(conn) = self.conn(id) else { return };
        conn.app_alive = false;
        let state = conn.state;
        if matches!(state, State::Closed | State::TimeWait | State::SynSent) {
            if state == State::SynSent {
                self.close_conn(id, None);
            }
            self.free_if_orphan(id);
            return;
        }
        self.send_reset_for(id);
        self.close_conn(id, None);
    }

    /// The application dropped its session: close it as a kernel close does.
    pub fn drop_app(&mut self, id: u32, gen: u32, now: i64) {
        if !self.valid(id, gen) {
            return;
        }
        let Some(conn) = self.conn(id) else { return };
        conn.app_alive = false;
        conn.read_waker = None;
        conn.write_waker = None;
        let unread = !conn.rcvbuf.is_empty();
        let state = conn.state;
        if state != State::SynReceived && state != State::Established && state != State::CloseWait {
            conn.rcvbuf = VecDeque::new();
        }
        match state {
            State::SynSent => self.close_conn(id, None),
            State::SynReceived | State::Established | State::CloseWait => {
                if unread {
                    self.send_reset_for(id);
                    self.close_conn(id, None);
                } else {
                    if let Some(conn) = self.conn(id) {
                        conn.fin_wanted = true;
                        conn.rcvbuf = VecDeque::new();
                    }
                    self.mark_dirty(id);
                }
            }
            State::FinWait2 => {
                let gen = self.gens[id as usize];
                let deadline = now + self.cfg.fin_wait2_us;
                self.arm(id, gen, deadline);
            }
            State::FinWait1 | State::Closing | State::LastAck | State::TimeWait | State::Closed => {
            }
        }
        self.free_if_orphan(id);
    }

    fn send_reset_for(&mut self, id: u32) {
        let Some(conn) = self.conns.get(id as usize).and_then(Option::as_ref) else {
            return;
        };
        let (lp, rp, seq, ack) = (
            conn.local_port,
            conn.remote_port,
            conn.snd_nxt,
            conn.rcv_nxt,
        );
        self.counters.resets_sent += 1;
        self.out.send(
            &mut self.counters,
            lp,
            rp,
            seq,
            ack,
            flags::RST | flags::ACK,
            0,
            None,
            (&[], &[]),
        );
    }

    /// Frees a connection that nobody holds and that cannot do anything.
    fn free_if_orphan(&mut self, id: u32) {
        let Some(conn) = self.conns.get(id as usize).and_then(Option::as_ref) else {
            return;
        };
        if !conn.app_alive && conn.state == State::Closed {
            self.release(id);
        }
    }

    /// Ends a connection: removes it from the table and wakes its tasks.
    fn close_conn(&mut self, id: u32, error: Option<io::ErrorKind>) {
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        let previous = conn.state;
        conn.state = State::Closed;
        conn.armed = false;
        conn.syn_pending = false;
        if error.is_some() {
            conn.error = error;
            conn.rcvbuf = VecDeque::new();
            self.counters.aborted += 1;
        }
        conn.sndbuf = VecDeque::new();
        let key = tuple_key(conn.local_port, conn.remote_port);
        let listener = conn.listener;
        let was_pending = previous == State::SynReceived && conn.in_backlog;
        if was_pending {
            conn.in_backlog = false;
        }
        for waker in [conn.read_waker.take(), conn.write_waker.take()]
            .into_iter()
            .flatten()
        {
            waker.wake();
        }
        if self.table.get(&key) == Some(&id) {
            self.table.remove(&key);
        }
        if was_pending {
            if let Some(l) = self.listeners.get_mut(&listener) {
                l.pending = l.pending.saturating_sub(1);
            }
        }
        self.free_if_orphan(id);
    }

    // ----------------------------------------------------------------------
    // The network side.

    /// Processes one frame from the link.
    pub fn input(&mut self, frame: &[u8], now: i64) {
        self.counters.frames_in += 1;
        let parsed = match wire::parse(frame) {
            Ok(parsed) => parsed,
            Err(_) => {
                self.counters.bad_frames += 1;
                return;
            }
        };
        let h = parsed.header;
        if h.dst_ip != self.ip {
            return;
        }
        let payload = parsed.payload;
        let seg_len = payload.len() as u32
            + u32::from(h.flags & flags::SYN != 0)
            + u32::from(h.flags & flags::FIN != 0);
        let key = tuple_key(h.dst_port, h.src_port);
        if let Some(&id) = self.table.get(&key) {
            let syn_only = h.flags & (flags::SYN | flags::ACK | flags::RST) == flags::SYN;
            let reusable = self
                .conns
                .get(id as usize)
                .and_then(Option::as_ref)
                .is_some_and(|c| {
                    c.state == State::TimeWait
                        && syn_only
                        && seq_lt(c.rcv_nxt.wrapping_sub(1), h.seq)
                })
                && self.listeners.contains_key(&h.dst_port);
            if reusable {
                // RFC 9293 allows a new SYN to replace a TIME-WAIT connection
                // when its sequence number is above the old one.
                self.table.remove(&key);
                if let Some(conn) = self.conn(id) {
                    conn.app_alive = false;
                    conn.state = State::Closed;
                    conn.armed = false;
                }
                self.free_if_orphan(id);
                self.passive_open(&h, now);
                return;
            }
            self.on_segment(id, &h, payload, now);
            return;
        }
        if h.flags & flags::RST != 0 {
            return;
        }
        if h.flags & (flags::SYN | flags::ACK) == flags::SYN {
            self.passive_open(&h, now);
        } else {
            self.reply_reset(&h, seg_len);
        }
    }

    fn passive_open(&mut self, h: &Header, now: i64) {
        let backlog = self.cfg.backlog;
        match self.listeners.get(&h.dst_port) {
            None => {
                self.reply_reset(h, 1);
                return;
            }
            Some(listener) if listener.pending >= backlog => return,
            Some(_) => {}
        }
        let iss = self.isn(h.dst_port, h.src_port, now);
        let mut conn = Conn::new(
            State::SynReceived,
            h.dst_port,
            h.src_port,
            iss,
            self.cfg.rto_us,
        );
        conn.rcv_nxt = h.seq.wrapping_add(1);
        conn.snd_wnd = u32::from(h.window);
        conn.mss = self.mss_own.min(h.mss.unwrap_or(DEFAULT_PEER_MSS));
        conn.listener = h.dst_port;
        conn.in_backlog = true;
        let (id, _gen) = self.alloc(conn);
        self.table.insert(tuple_key(h.dst_port, h.src_port), id);
        if let Some(listener) = self.listeners.get_mut(&h.dst_port) {
            listener.pending += 1;
        }
        self.mark_dirty(id);
    }

    #[allow(clippy::too_many_lines)]
    fn on_segment(&mut self, id: u32, h: &Header, payload: &[u8], now: i64) {
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        let gen = self.gens[id as usize];
        let syn = h.flags & flags::SYN != 0;
        let fin = h.flags & flags::FIN != 0;
        let rst = h.flags & flags::RST != 0;
        let has_ack = h.flags & flags::ACK != 0;

        if conn.state == State::SynSent {
            if has_ack && h.ack != conn.snd_nxt {
                if !rst {
                    let seg = payload.len() as u32 + u32::from(syn) + u32::from(fin);
                    self.reply_reset(h, seg);
                }
                return;
            }
            if rst {
                if has_ack {
                    self.close_conn(id, Some(io::ErrorKind::ConnectionRefused));
                }
                return;
            }
            if syn && has_ack {
                conn.rcv_nxt = h.seq.wrapping_add(1);
                conn.snd_una = h.ack;
                conn.snd_wnd = u32::from(h.window);
                conn.mss = conn.mss.min(h.mss.unwrap_or(DEFAULT_PEER_MSS));
                conn.state = State::Established;
                conn.armed = false;
                conn.retries = 0;
                conn.ack_pending = true;
                if let Some(waker) = conn.write_waker.take() {
                    waker.wake();
                }
                self.mark_dirty(id);
            }
            // A SYN without ACK would be a simultaneous open, which the stack
            // does not do, and the segment is dropped.
            return;
        }

        // Closed connections only wait for their session to go away.
        if conn.state == State::Closed {
            return;
        }

        let wnd_now = self.cfg.rcv_buffer.saturating_sub(conn.rcvbuf.len()) as u32;
        let mut seq = h.seq;
        let mut data = payload;
        let mut fin = fin;
        if syn {
            // A repeated SYN of the handshake gets the SYN-ACK again.
            if conn.state == State::SynReceived && h.seq.wrapping_add(1) == conn.rcv_nxt {
                conn.syn_pending = true;
                self.mark_dirty(id);
            } else {
                conn.ack_pending = true;
                self.mark_dirty(id);
            }
            return;
        }
        // Sequence check and trimming of what was received already.
        let seg_len = data.len() as u32 + u32::from(fin);
        if rst {
            let offset = h.seq.wrapping_sub(conn.rcv_nxt);
            if offset >= wnd_now.max(1) && h.seq != conn.rcv_nxt {
                return;
            }
            let error = if conn.state == State::TimeWait {
                None
            } else {
                Some(io::ErrorKind::ConnectionReset)
            };
            self.close_conn(id, error);
            return;
        }
        if seq != conn.rcv_nxt {
            if seq_lt(seq, conn.rcv_nxt) {
                let skip = conn.rcv_nxt.wrapping_sub(seq);
                if skip >= seg_len {
                    // A segment that the peer sent again: acknowledge it, and
                    // let the ACK field below still count.
                    if seg_len > 0 {
                        conn.ack_pending = true;
                        let in_time_wait = conn.state == State::TimeWait;
                        self.mark_dirty(id);
                        if in_time_wait {
                            let deadline = now + self.cfg.time_wait_us;
                            self.arm(id, gen, deadline);
                        }
                    }
                    let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut)
                    else {
                        return;
                    };
                    data = &[];
                    fin = false;
                    seq = conn.rcv_nxt;
                } else {
                    data = &data[(skip as usize).min(data.len())..];
                    seq = conn.rcv_nxt;
                }
            } else {
                // A gap: the stack keeps no out of order data. It drops the
                // segment and repeats its ACK so that the sender goes back.
                if seg_len > 0 {
                    conn.ack_pending = true;
                    self.mark_dirty(id);
                }
                return;
            }
        }
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        let _ = seq;
        if !has_ack {
            return;
        }
        conn.retries = 0;

        // Acknowledgment processing.
        if conn.state == State::SynReceived {
            if h.ack == conn.snd_nxt {
                conn.snd_una = h.ack;
                conn.state = State::Established;
                conn.armed = false;
                conn.snd_wnd = u32::from(h.window);
                let listener = conn.listener;
                let handle = (id, gen);
                if let Some(l) = self.listeners.get_mut(&listener) {
                    l.queue.push_back(handle);
                    if let Some((_, waker)) = l.waiters.pop_first() {
                        waker.wake();
                    }
                }
            } else {
                let seg = payload.len() as u32;
                self.reply_reset(h, seg);
                return;
            }
        }
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        let mut state_changed = false;
        if seq_lt(conn.snd_una, h.ack) && seq_le(h.ack, conn.snd_nxt) {
            let advanced = h.ack.wrapping_sub(conn.snd_una);
            let fin_acked = conn.sent_fin && h.ack == conn.snd_nxt;
            let data_acked = (advanced - u32::from(fin_acked)) as usize;
            let drop_bytes = data_acked.min(conn.sndbuf.len());
            conn.sndbuf.drain(..drop_bytes);
            if conn.sndbuf.is_empty() {
                conn.sndbuf = VecDeque::new();
            }
            conn.snd_una = h.ack;
            conn.rto_us = self.cfg.rto_us;
            if conn.outstanding() {
                conn.deadline = now + conn.rto_us;
            } else {
                conn.armed = false;
            }
            if fin_acked {
                match conn.state {
                    State::FinWait1 => {
                        conn.state = State::FinWait2;
                        state_changed = true;
                    }
                    State::Closing => {
                        conn.state = State::TimeWait;
                        state_changed = true;
                    }
                    State::LastAck => {
                        self.close_conn(id, None);
                        return;
                    }
                    _ => {}
                }
            }
            if let Some(waker) = conn.write_waker.take() {
                waker.wake();
            }
        } else if seq_lt(conn.snd_nxt, h.ack) {
            // An acknowledgment for data that was never sent.
            conn.ack_pending = true;
            self.mark_dirty(id);
            return;
        }
        if !seq_lt(h.ack, conn.snd_una) {
            let wnd = u32::from(h.window);
            if wnd > conn.snd_wnd {
                if let Some(waker) = conn.write_waker.take() {
                    waker.wake();
                }
            }
            if wnd != conn.snd_wnd {
                conn.snd_wnd = wnd;
            }
            self.mark_dirty(id);
        }
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };

        // Data.
        let can_receive = matches!(
            conn.state,
            State::Established | State::FinWait1 | State::FinWait2
        );
        let mut all_taken = true;
        if !data.is_empty() {
            if !can_receive {
                conn.ack_pending = true;
            } else if !conn.app_alive && conn.state != State::Established {
                // An orphan that receives data after its close gets a reset,
                // as the kernel does.
                self.send_reset_for(id);
                self.close_conn(id, Some(io::ErrorKind::ConnectionReset));
                return;
            } else {
                let space = self.cfg.rcv_buffer.saturating_sub(conn.rcvbuf.len());
                let take = space.min(data.len());
                all_taken = take == data.len();
                if take > 0 {
                    conn.rcvbuf.extend(&data[..take]);
                    conn.rcv_nxt = conn.rcv_nxt.wrapping_add(take as u32);
                    if let Some(waker) = conn.read_waker.take() {
                        waker.wake();
                    }
                }
                conn.ack_pending = true;
            }
        }

        // FIN.
        if fin && all_taken && can_receive && !conn.fin_rcvd {
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
            conn.fin_rcvd = true;
            conn.ack_pending = true;
            conn.state = match conn.state {
                State::Established => State::CloseWait,
                State::FinWait1 => State::Closing,
                State::FinWait2 => State::TimeWait,
                other => other,
            };
            state_changed = true;
            if let Some(waker) = conn.read_waker.take() {
                waker.wake();
            }
        }
        if state_changed {
            self.after_state_change(id, now);
        }
        if let Some(conn) = self.conn(id) {
            if conn.ack_pending {
                self.mark_dirty(id);
            }
        }
    }

    /// Arms the timers that a state needs and frees what it no longer holds.
    fn after_state_change(&mut self, id: u32, now: i64) {
        let gen = self.gens[id as usize];
        let Some(conn) = self.conn(id) else { return };
        match conn.state {
            State::TimeWait => {
                // Unread data stays readable until the session reads or
                // drops it.
                conn.sndbuf = VecDeque::new();
                if !conn.app_alive {
                    conn.rcvbuf = VecDeque::new();
                }
                let deadline = now + self.cfg.time_wait_us;
                self.arm(id, gen, deadline);
            }
            State::FinWait2 if !conn.app_alive => {
                let deadline = now + self.cfg.fin_wait2_us;
                self.arm(id, gen, deadline);
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------------------
    // Output and timers.

    /// Fires the timers that are due at `now` and returns how many fired.
    pub fn expire(&mut self, now: i64) -> usize {
        let mut fired = 0;
        while let Some(&Reverse((deadline, id, gen))) = self.timers.peek() {
            if deadline > now {
                break;
            }
            self.timers.pop();
            if self.gens.get(id as usize) != Some(&gen) {
                continue;
            }
            let Some(conn) = self.conn(id) else { continue };
            conn.in_heap = false;
            if !conn.armed {
                continue;
            }
            if conn.deadline > now {
                let later = conn.deadline;
                conn.in_heap = true;
                self.timers.push(Reverse((later, id, gen)));
                continue;
            }
            conn.armed = false;
            fired += 1;
            self.on_timer(id, gen, now);
        }
        fired
    }

    fn on_timer(&mut self, id: u32, gen: u32, now: i64) {
        let max_rto = self.cfg.max_rto_us;
        let max_retries = self.cfg.max_retries;
        let Some(conn) = self.conn(id) else { return };
        match conn.state {
            State::TimeWait | State::FinWait2 => {
                // The hold of TIME-WAIT is over, or an orphan waited long
                // enough for the FIN of its peer.
                if conn.state == State::FinWait2 && conn.app_alive {
                    return;
                }
                self.close_conn(id, None);
                if let Some(conn) = self.conn(id) {
                    conn.app_alive = false;
                }
                self.free_if_orphan(id);
            }
            State::Closed => {}
            _ => {
                if !conn.outstanding() {
                    return;
                }
                conn.retries += 1;
                if conn.retries > max_retries {
                    self.send_reset_for(id);
                    self.close_conn(id, Some(io::ErrorKind::TimedOut));
                    return;
                }
                conn.rto_us = (conn.rto_us * 2).min(max_rto);
                // Go back N: everything after the first unacknowledged byte
                // is sent again, within the window, and a zero window gets a
                // one segment probe.
                conn.snd_nxt = conn.snd_una;
                conn.sent_fin = false;
                conn.probe = true;
                if matches!(conn.state, State::SynSent | State::SynReceived) {
                    conn.syn_pending = true;
                    conn.snd_nxt = conn.snd_una;
                }
                self.counters.retransmits += 1;
                let deadline = now + self.conn(id).map_or(0, |c| c.rto_us);
                self.arm(id, gen, deadline);
                self.mark_dirty(id);
            }
        }
    }

    /// Turns the pending work of every dirty connection into frames.
    pub fn flush_dirty(&mut self, now: i64) -> usize {
        let ids = std::mem::take(&mut self.dirty);
        let count = ids.len();
        for id in ids {
            self.output(id, now);
        }
        count
    }

    fn output(&mut self, id: u32, now: i64) {
        let gen = self.gens.get(id as usize).copied().unwrap_or(0);
        let window_size = self.cfg.rcv_buffer;
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        conn.dirty = false;
        let rcv_wnd = u16::try_from(window_size.saturating_sub(conn.rcvbuf.len()).min(65_535))
            .unwrap_or(65_535);
        let (lp, rp) = (conn.local_port, conn.remote_port);
        let mut sent_anything = false;

        if conn.syn_pending && matches!(conn.state, State::SynSent | State::SynReceived) {
            conn.syn_pending = false;
            let bits = if conn.state == State::SynSent {
                flags::SYN
            } else {
                flags::SYN | flags::ACK
            };
            let ack = if conn.state == State::SynSent {
                0
            } else {
                conn.rcv_nxt
            };
            let mss = Some(conn.mss);
            self.out.send(
                &mut self.counters,
                lp,
                rp,
                conn.iss,
                ack,
                bits,
                rcv_wnd,
                mss,
                (&[], &[]),
            );
            conn.snd_nxt = conn.iss.wrapping_add(1);
            conn.ack_pending = false;
            if !conn.armed {
                let deadline = now + conn.rto_us;
                conn.deadline = deadline;
                conn.armed = true;
                if !conn.in_heap {
                    conn.in_heap = true;
                    self.timers.push(Reverse((deadline, id, gen)));
                }
            }
            return;
        }
        if matches!(
            conn.state,
            State::SynSent | State::SynReceived | State::Closed
        ) {
            return;
        }

        let sendable = matches!(
            conn.state,
            State::Established
                | State::CloseWait
                | State::FinWait1
                | State::Closing
                | State::LastAck
        );
        if sendable {
            loop {
                let unacked = conn.unacked_data();
                let avail = conn.sndbuf.len().saturating_sub(unacked);
                if avail == 0 {
                    break;
                }
                let mut wnd = conn.snd_wnd;
                if conn.probe {
                    wnd = wnd.max(1);
                }
                let room = wnd.saturating_sub(conn.in_flight()) as usize;
                let n = avail.min(room).min(usize::from(conn.mss));
                if n == 0 {
                    break;
                }
                let (a, b) = conn.sndbuf.as_slices();
                let payload = slices_range(a, b, unacked, n);
                let bits = flags::ACK | if n == avail { flags::PSH } else { 0 };
                self.out.send(
                    &mut self.counters,
                    lp,
                    rp,
                    conn.snd_nxt,
                    conn.rcv_nxt,
                    bits,
                    rcv_wnd,
                    None,
                    payload,
                );
                conn.snd_nxt = conn.snd_nxt.wrapping_add(n as u32);
                conn.ack_pending = false;
                conn.probe = false;
                sent_anything = true;
            }
            let all_sent = conn.sndbuf.len() <= conn.unacked_data();
            if conn.fin_wanted && !conn.sent_fin && all_sent {
                self.out.send(
                    &mut self.counters,
                    lp,
                    rp,
                    conn.snd_nxt,
                    conn.rcv_nxt,
                    flags::FIN | flags::ACK,
                    rcv_wnd,
                    None,
                    (&[], &[]),
                );
                conn.sent_fin = true;
                conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
                conn.ack_pending = false;
                conn.state = match conn.state {
                    State::Established => State::FinWait1,
                    State::CloseWait => State::LastAck,
                    other => other,
                };
                sent_anything = true;
            }
        }
        conn.probe = false;
        if conn.ack_pending {
            conn.ack_pending = false;
            self.out.send(
                &mut self.counters,
                lp,
                rp,
                conn.snd_nxt,
                conn.rcv_nxt,
                flags::ACK,
                rcv_wnd,
                None,
                (&[], &[]),
            );
        }
        if sent_anything && conn.outstanding() && !conn.armed {
            let deadline = now + conn.rto_us;
            conn.deadline = deadline;
            conn.armed = true;
            if !conn.in_heap {
                conn.in_heap = true;
                self.timers.push(Reverse((deadline, id, gen)));
            }
        }
    }
}

/// Returns the `len` bytes at `from` of the ring `a` followed by `b`, as two
/// slices.
fn slices_range<'a>(a: &'a [u8], b: &'a [u8], from: usize, len: usize) -> (&'a [u8], &'a [u8]) {
    if from >= a.len() {
        let start = from - a.len();
        (&b[start..start + len], &[])
    } else if from + len <= a.len() {
        (&a[from..from + len], &[])
    } else {
        (&a[from..], &b[..from + len - a.len()])
    }
}

fn not_connected() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "the session was aborted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_ranges_cross_the_wrap() {
        let a = [1u8, 2, 3];
        let b = [4u8, 5, 6];
        assert_eq!(slices_range(&a, &b, 0, 2), (&a[..2], &[][..]));
        assert_eq!(slices_range(&a, &b, 2, 3), (&a[2..], &b[..2]));
        assert_eq!(slices_range(&a, &b, 4, 2), (&b[1..3], &[][..]));
    }

    #[test]
    fn sequence_comparison_wraps() {
        assert!(seq_lt(u32::MAX, 1));
        assert!(!seq_lt(1, u32::MAX));
        assert!(seq_le(5, 5));
    }

    #[test]
    fn the_isn_depends_on_the_tuple_the_secret_and_the_time() {
        let cfg = OwnConfig::default();
        let host = Host::new(1, 2, cfg.clone(), 1500);
        let a = host.isn(40000, 80, 0);
        assert_ne!(a, host.isn(40001, 80, 0));
        assert_ne!(a, host.isn(40000, 81, 0));
        assert_eq!(host.isn(40000, 80, 4_000_000).wrapping_sub(a), 1_000_000);
        let other = Host::new(
            1,
            2,
            OwnConfig {
                isn_secret: 7,
                ..cfg
            },
            1500,
        );
        assert_ne!(a, other.isn(40000, 80, 0));
    }
}
