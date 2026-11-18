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

use super::net::{
    NetError, Received, Station, ETH_LEN, MAX_REPLY_LEN, MIN_FRAME_LEN, VLAN_TAG_LEN,
};
use super::recovery::{self, AckView, Action, Recovery};
use super::wire::{self, flags, Header, Options};
use super::OwnConfig;

/// The MSS that a peer is assumed to have when its SYN carries no option.
const DEFAULT_PEER_MSS: u16 = 536;
/// The first port of the ephemeral range of an active open.
const EPHEMERAL_FIRST: u16 = 10_000;
/// The bytes that the timestamp option takes in every segment, which the
/// payload of a segment gives up (RFC 7323 section 2.2).
const TS_OVERHEAD: u16 = wire::TIMESTAMP_OPTION_LEN as u16;

// The option bits of a connection, packed into `Conn::opts` with the two
// window scale shifts (4 bits each) and two flags of the delayed ACK.
/// Window scaling is in use (or offered, before the handshake ends).
const O_WS: u16 = 1;
/// Timestamps are in use.
const O_TS: u16 = 1 << 1;
/// Selective acknowledgment is in use.
const O_SACK: u16 = 1 << 2;
/// The connection timer runs for a delayed ACK and not for a timeout.
const O_DELACK: u16 = 1 << 3;
/// One in order segment was received and its ACK is held back.
const O_UNACKED: u16 = 1 << 4;
/// An entry of the timer heap that was pushed for a delayed ACK may be in the
/// heap, which keeps a second delayed ACK from pushing another. The entry of
/// a timeout cannot serve: it lies at least 200 ms ahead.
const O_DQ: u16 = 1 << 13;
const RCV_SHIFT_AT: u16 = 5;
const SND_SHIFT_AT: u16 = 9;
const SHIFT_MASK: u16 = 0xF;

/// The shift that lets a buffer of `buffer` bytes fit the 16 bit window.
fn shift_for(buffer: usize) -> u8 {
    let mut shift = 0u8;
    while shift < wire::MAX_WSCALE && (buffer >> shift) > 65_535 {
        shift += 1;
    }
    shift
}

/// The timestamp clock: milliseconds of `now` plus a per host offset.
fn ts_clock(base: u32, now: i64) -> u32 {
    ((now.max(0) / 1000) as u32).wrapping_add(base)
}

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
        // Fold the high half into the low half, because the table takes its
        // bucket from the low bits and the multiplication fills the high ones.
        self.0 ^ (self.0 >> 32)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x517C_C1B7_2722_0A95);
        }
    }
    fn write_u32(&mut self, value: u32) {
        self.0 = (self.0.rotate_left(23) ^ u64::from(value)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    fn write_u16(&mut self, value: u16) {
        self.0 = (self.0.rotate_left(23) ^ u64::from(value)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type IntMap<K, V> = HashMap<K, V, BuildHasherDefault<IntHasher>>;

pub(crate) fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

pub(crate) fn seq_le(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) <= 0
}

/// The key of the connection table: the four values that name a connection
/// from this host. It is 12 bytes, so an entry of the table with its `u32`
/// handle takes 16 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tuple {
    /// The local address.
    pub local_ip: u32,
    /// The remote address.
    pub remote_ip: u32,
    /// The local port.
    pub local_port: u16,
    /// The remote port.
    pub remote_port: u16,
}

/// One connection.
struct Conn {
    state: State,
    local_ip: u32,
    remote_ip: u32,
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
    /// The loss recovery state, which exists only while data is in flight,
    /// waits to be sent, or waits behind a hole (see [`recovery`]).
    rec: Option<Box<Recovery>>,
    /// The window that the last segment sent advertised, which decides when
    /// a read of the application is worth a window update.
    adv_wnd: u16,
    /// The negotiated options and the window scale shifts (see the `O_`
    /// bits), 2 bytes for the whole set.
    opts: u16,
    /// The timestamp of the peer that is echoed (RFC 7323 `TS.Recent`).
    ts_recent: u32,
}

impl Conn {
    fn new(state: State, tuple: Tuple, iss: u32, rto_us: i64) -> Self {
        Self {
            state,
            local_ip: tuple.local_ip,
            remote_ip: tuple.remote_ip,
            local_port: tuple.local_port,
            remote_port: tuple.remote_port,
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
            rec: None,
            adv_wnd: u16::MAX,
            opts: 0,
            ts_recent: 0,
        }
    }

    fn has(&self, bit: u16) -> bool {
        self.opts & bit != 0
    }

    fn set(&mut self, bit: u16, on: bool) {
        if on {
            self.opts |= bit;
        } else {
            self.opts &= !bit;
        }
    }

    /// The shift that the receive window of this side is advertised with.
    fn rcv_shift(&self) -> u8 {
        ((self.opts >> RCV_SHIFT_AT) & SHIFT_MASK) as u8
    }

    /// The shift of the window that the peer advertises.
    fn snd_shift(&self) -> u8 {
        ((self.opts >> SND_SHIFT_AT) & SHIFT_MASK) as u8
    }

    fn set_shifts(&mut self, rcv: u8, snd: u8) {
        self.opts &= !((SHIFT_MASK << RCV_SHIFT_AT) | (SHIFT_MASK << SND_SHIFT_AT));
        self.opts |= (u16::from(rcv) << RCV_SHIFT_AT) | (u16::from(snd) << SND_SHIFT_AT);
    }

    /// The window field for `space` free bytes of the receive buffer.
    fn window_field(&self, space: usize) -> u16 {
        u16::try_from((space >> self.rcv_shift()).min(65_535)).unwrap_or(65_535)
    }

    /// An acknowledgment went out (or goes out with a segment): nothing is
    /// owed any more, and a timer that ran for the delayed ACK stops.
    fn acked(&mut self) {
        self.ack_pending = false;
        self.opts &= !O_UNACKED;
        if self.has(O_DELACK) {
            self.opts &= !O_DELACK;
            self.armed = false;
        }
    }

    /// The options of a segment after the handshake: the timestamp, and SACK
    /// blocks while data waits behind a hole.
    fn tx_opts(&self, ts_now: u32) -> Options {
        let mut opts = Options::default();
        if self.has(O_TS) {
            opts.timestamp = Some((ts_now, self.ts_recent));
        }
        if self.has(O_SACK) {
            if let Some(rec) = &self.rec {
                if !rec.ooo.is_empty() {
                    rec.ooo.sack_blocks(self.rcv_nxt, &mut opts);
                }
            }
        }
        opts
    }

    /// The options of a SYN or SYN-ACK: what is offered, or what was agreed.
    fn syn_opts(&self, ts_now: u32) -> Options {
        Options {
            wscale: self.has(O_WS).then(|| self.rcv_shift()),
            sack_permitted: self.has(O_SACK),
            timestamp: self.has(O_TS).then(|| {
                let echo = if self.state == State::SynReceived {
                    self.ts_recent
                } else {
                    0
                };
                (ts_now, echo)
            }),
            ..Options::default()
        }
    }

    /// The highest sequence number sent: `snd_nxt`, or more after a timeout
    /// went back.
    fn snd_max(&self) -> u32 {
        self.rec.as_ref().map_or(self.snd_nxt, |r| {
            if seq_lt(self.snd_nxt, r.snd_max) {
                r.snd_max
            } else {
                self.snd_nxt
            }
        })
    }

    /// Whether the recovery state has nothing left to do, so that the box
    /// can go: no data in flight or waiting, no segment held out of order.
    fn recovery_idle(&self) -> bool {
        self.rec.as_ref().is_some_and(|r| {
            !r.rtx
                && r.ooo.is_empty()
                && r.sb.is_empty()
                && self.sndbuf.is_empty()
                && self.snd_max() == self.snd_una
        })
    }

    /// The recovery state, made on first use.
    fn rec_mut(&mut self, iw: u32) -> &mut Recovery {
        let (mss, snd_nxt) = (self.mss, self.snd_nxt);
        self.rec
            .get_or_insert_with(|| Box::new(Recovery::new(mss, iw, snd_nxt)))
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
    /// Segments sent again after a timeout or a fast retransmit.
    pub retransmits: u64,
    /// Retransmission timeouts that fired with data in flight.
    pub timeouts: u64,
    /// Fast retransmits: the oldest segment sent again on the third duplicate
    /// acknowledgment or on a partial acknowledgment in fast recovery.
    pub fast_retransmits: u64,
    /// Segments that were held out of order and joined the stream later.
    pub ooo_segments: u64,
    /// Resets that the host sent.
    pub resets_sent: u64,
    /// Connections that a timeout or a reset ended abnormally.
    pub aborted: u64,
    /// Frames that the host did not send, because the local or the remote
    /// address has no MAC address in its neighbor tables.
    pub unresolved: u64,
    /// Frames that the host dropped for their Ethernet header: another VLAN,
    /// a tag where the port has none, or a tag type that the stack does not
    /// take.
    pub link_dropped: u64,
    /// ARP replies that the host built for hosts it serves.
    pub arp_replies: u64,
    /// Acknowledgments that the delayed ACK timer sent.
    pub delayed_acks: u64,
    /// Segments dropped by the PAWS test of RFC 7323 (an old timestamp, or no
    /// timestamp on a connection that agreed on them).
    pub paws_drops: u64,
    /// Recoveries that the SACK scoreboard drove (RFC 6675).
    pub sack_recoveries: u64,
    /// Segments that the scoreboard picked to send again, which are also in
    /// `retransmits`.
    pub sack_retransmits: u64,
}

/// The frames that wait for the link.
pub struct Out {
    /// The built frames, oldest first.
    pub queue: VecDeque<Vec<u8>>,
    station: Station,
}

impl Out {
    #[allow(clippy::too_many_arguments)]
    fn send(
        &mut self,
        counters: &mut HostCounters,
        ips: (u32, u32),
        local_port: u16,
        remote_port: u16,
        seq: u32,
        ack: u32,
        flag_bits: u8,
        window: u16,
        mss: Option<u16>,
        opts: Options,
        payload: (&[u8], &[u8]),
    ) {
        let header = Header {
            src_ip: ips.0,
            dst_ip: ips.1,
            src_port: local_port,
            dst_port: remote_port,
            seq,
            ack,
            flags: flag_bits,
            window,
            mss,
            opts,
        };
        let body = wire::IP_LEN
            + wire::TCP_LEN
            + wire::options_len(&header)
            + payload.0.len()
            + payload.1.len();
        // One allocation for the whole frame, with room for a VLAN tag and
        // for the padding of a short frame.
        let mut frame = Vec::with_capacity((ETH_LEN + VLAN_TAG_LEN + body).max(MIN_FRAME_LEN));
        if !self.station.head(&mut frame, ips.0, ips.1) {
            counters.unresolved += 1;
            return;
        }
        wire::encode(&mut frame, &header, payload);
        if frame.len() < MIN_FRAME_LEN {
            frame.resize(MIN_FRAME_LEN, 0);
        }
        self.queue.push_back(frame);
        counters.frames_out += 1;
    }
}

/// One host: its address, connections, listeners, timers, and output queue.
pub struct Host {
    cfg: OwnConfig,
    mss_own: u16,
    conns: Vec<Option<Conn>>,
    gens: Vec<u32>,
    free: Vec<u32>,
    live: u64,
    table: IntMap<Tuple, u32>,
    listeners: IntMap<u16, Listener>,
    dirty: Vec<u32>,
    timers: BinaryHeap<Reverse<(i64, u32, u32)>>,
    next_port: u16,
    /// The offset of the timestamp clock of this host.
    ts_base: u32,
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
    /// Creates the hosts of `station`, which share one connection table,
    /// with an IP MTU of `mtu` bytes.
    pub fn new(station: Station, cfg: OwnConfig, mtu: usize) -> Self {
        let mss_own = u16::try_from(mtu.saturating_sub(wire::IP_LEN + wire::TCP_LEN))
            .unwrap_or(u16::MAX)
            .max(64);
        let cfg_secret = cfg.isn_secret;
        Self {
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
            ts_base: mix(cfg_secret) as u32,
            out: Out {
                queue: VecDeque::new(),
                station,
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
    fn isn(&self, t: Tuple, now: i64) -> u32 {
        let tuple = (u64::from(t.local_ip) << 32)
            ^ (u64::from(t.local_port) << 16)
            ^ u64::from(t.remote_port);
        let hash = mix(self.cfg.isn_secret ^ mix(tuple ^ (u64::from(t.remote_ip) << 17)));
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
        conn.opts &= !O_DELACK;
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
            (h.dst_ip, h.src_ip),
            h.dst_port,
            h.src_port,
            seq,
            ack,
            bits,
            0,
            None,
            Options::default(),
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

    /// Starts an active open from the local host `local_ip` to `remote_port`
    /// of the neighbor `remote_ip` and returns the connection handle.
    ///
    /// The local port is the next free one of the ephemeral range for this
    /// tuple, so two local hosts, or two neighbors, can use the same port.
    pub fn open(
        &mut self,
        local_ip: u32,
        remote_ip: u32,
        remote_port: u16,
        now: i64,
    ) -> io::Result<(u32, u32)> {
        if !self.out.station.owns(local_ip) {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "the local address is not a host of this stack",
            ));
        }
        if self.out.station.resolve(remote_ip).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::HostUnreachable,
                "the remote address has no neighbor entry",
            ));
        }
        let range = u32::from(u16::MAX - EPHEMERAL_FIRST) + 1;
        let mut local_port = None;
        for _ in 0..range {
            let candidate = self.next_port;
            self.next_port = if candidate == u16::MAX {
                EPHEMERAL_FIRST
            } else {
                candidate + 1
            };
            let tuple = Tuple {
                local_ip,
                remote_ip,
                local_port: candidate,
                remote_port,
            };
            if !self.table.contains_key(&tuple) {
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
        let tuple = Tuple {
            local_ip,
            remote_ip,
            local_port,
            remote_port,
        };
        let iss = self.isn(tuple, now);
        let mut conn = Conn::new(State::SynSent, tuple, iss, self.cfg.rto_us);
        conn.mss = self.mss_own;
        conn.set(O_WS, self.cfg.window_scale);
        conn.set(O_TS, self.cfg.timestamps);
        conn.set(O_SACK, self.cfg.sack);
        conn.set_shifts(shift_for(self.cfg.rcv_buffer), 0);
        let (id, gen) = self.alloc(conn);
        self.table.insert(tuple, id);
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
            // Tell the peer when the window opened by a worthwhile amount
            // since the last one it was told: a segment of data or half the
            // buffer, as RFC 1122 section 4.2.3.3 asks. Without this a sender
            // that a congestion window keeps below the receive window would
            // see the window shrink by what is unread at every ACK.
            let space = rcv_cap.saturating_sub(conn.rcvbuf.len());
            let step = usize::from(conn.mss).min(rcv_cap / 2).max(1);
            let shift = conn.rcv_shift();
            let open_now = usize::from(conn.window_field(space)) << shift;
            if open_now.saturating_sub(usize::from(conn.adv_wnd) << shift) >= step
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
        let (ips, lp, rp, seq, ack) = (
            (conn.local_ip, conn.remote_ip),
            conn.local_port,
            conn.remote_port,
            conn.snd_nxt,
            conn.rcv_nxt,
        );
        self.counters.resets_sent += 1;
        self.out.send(
            &mut self.counters,
            ips,
            lp,
            rp,
            seq,
            ack,
            flags::RST | flags::ACK,
            0,
            None,
            Options::default(),
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
        conn.rec = None;
        let key = Tuple {
            local_ip: conn.local_ip,
            remote_ip: conn.remote_ip,
            local_port: conn.local_port,
            remote_port: conn.remote_port,
        };
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
        let mut reply = [0u8; MAX_REPLY_LEN];
        let packet = match self.out.station.receive(frame, &mut reply) {
            Ok(Received::Ip(packet)) => packet,
            Ok(Received::Reply(len)) => {
                self.counters.arp_replies += 1;
                self.counters.frames_out += 1;
                self.out.queue.push_back(reply[..len].to_vec());
                return;
            }
            Ok(Received::Ignored) => return,
            Err(NetError::Short) => {
                self.counters.bad_frames += 1;
                return;
            }
            Err(_) => {
                self.counters.link_dropped += 1;
                return;
            }
        };
        let parsed = match wire::parse(packet) {
            Ok(parsed) => parsed,
            Err(_) => {
                self.counters.bad_frames += 1;
                return;
            }
        };
        let h = parsed.header;
        if !self.out.station.owns(h.dst_ip) {
            return;
        }
        let payload = parsed.payload;
        let seg_len = payload.len() as u32
            + u32::from(h.flags & flags::SYN != 0)
            + u32::from(h.flags & flags::FIN != 0);
        let key = Tuple {
            local_ip: h.dst_ip,
            remote_ip: h.src_ip,
            local_port: h.dst_port,
            remote_port: h.src_port,
        };
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
        if self.out.station.resolve(h.src_ip).is_none() {
            // No SYN-ACK could reach a host that has no neighbor entry.
            self.counters.unresolved += 1;
            return;
        }
        match self.listeners.get(&h.dst_port) {
            None => {
                self.reply_reset(h, 1);
                return;
            }
            Some(listener) if listener.pending >= backlog => return,
            Some(_) => {}
        }
        let tuple = Tuple {
            local_ip: h.dst_ip,
            remote_ip: h.src_ip,
            local_port: h.dst_port,
            remote_port: h.src_port,
        };
        let iss = self.isn(tuple, now);
        let mut conn = Conn::new(State::SynReceived, tuple, iss, self.cfg.rto_us);
        conn.rcv_nxt = h.seq.wrapping_add(1);
        // The options: each one is used when the SYN carries it and this host
        // wants it, and the SYN-ACK answers only those.
        let ws = if self.cfg.window_scale {
            h.opts.wscale
        } else {
            None
        };
        conn.set(O_WS, ws.is_some());
        let rcv_shift = if ws.is_some() {
            shift_for(self.cfg.rcv_buffer)
        } else {
            0
        };
        conn.set_shifts(rcv_shift, ws.unwrap_or(0));
        conn.adv_wnd = 65_535 >> rcv_shift;
        let ts = if self.cfg.timestamps {
            h.opts.timestamp
        } else {
            None
        };
        conn.set(O_TS, ts.is_some());
        if let Some((value, _)) = ts {
            conn.ts_recent = value;
        }
        conn.set(O_SACK, self.cfg.sack && h.opts.sack_permitted);
        conn.snd_wnd = u32::from(h.window);
        conn.mss = self.mss_own.min(h.mss.unwrap_or(DEFAULT_PEER_MSS));
        if ts.is_some() {
            conn.mss = conn.mss.saturating_sub(TS_OVERHEAD).max(1);
        }
        conn.listener = h.dst_port;
        conn.in_backlog = true;
        let (id, _gen) = self.alloc(conn);
        self.table.insert(tuple, id);
        if let Some(listener) = self.listeners.get_mut(&h.dst_port) {
            listener.pending += 1;
        }
        self.mark_dirty(id);
    }

    #[allow(clippy::too_many_lines)]
    fn on_segment(&mut self, id: u32, h: &Header, payload: &[u8], now: i64) {
        let ts_now = ts_clock(self.ts_base, now);
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
                // The SYN-ACK answers the offer: an option that it lacks is
                // off, and its window is never scaled.
                conn.snd_wnd = u32::from(h.window);
                match h.opts.wscale {
                    Some(shift) if conn.has(O_WS) => conn.set_shifts(conn.rcv_shift(), shift),
                    _ => {
                        conn.set(O_WS, false);
                        conn.set_shifts(0, 0);
                    }
                }
                match h.opts.timestamp {
                    Some((value, _)) if conn.has(O_TS) => conn.ts_recent = value,
                    _ => conn.set(O_TS, false),
                }
                if !h.opts.sack_permitted {
                    conn.set(O_SACK, false);
                }
                conn.adv_wnd = 65_535 >> conn.rcv_shift();
                conn.mss = conn.mss.min(h.mss.unwrap_or(DEFAULT_PEER_MSS));
                if conn.has(O_TS) {
                    conn.mss = conn.mss.saturating_sub(TS_OVERHEAD).max(1);
                }
                conn.state = State::Established;
                conn.armed = false;
                conn.retries = 0;
                conn.rto_us = self.cfg.rto_us;
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

        if conn.has(O_TS) && !rst && !syn {
            // PAWS (RFC 7323 section 5): an old timestamp marks a segment of
            // an earlier incarnation or a wrapped sequence number, and a
            // segment without the option breaks the agreement (section 3.2).
            // Either is dropped, and a drop is answered with an ACK.
            match h.opts.timestamp {
                Some((value, _)) if (value.wrapping_sub(conn.ts_recent) as i32) >= 0 => {
                    // TS.Recent follows the segments that start at or before
                    // the next byte expected, and with a delayed ACK pending
                    // it keeps the older one, which the ACK will echo.
                    if seq_le(h.seq, conn.rcv_nxt) && !conn.has(O_UNACKED) {
                        conn.ts_recent = value;
                    }
                }
                _ => {
                    self.counters.paws_drops += 1;
                    conn.ack_pending = true;
                    self.mark_dirty(id);
                    return;
                }
            }
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
                // A gap: hold the data (and a FIN) in the out of order queue
                // if it lies inside the window, repeat the ACK at once, so
                // that the sender counts one duplicate for every segment, and
                // go on with the ACK field of the segment.
                if seg_len > 0 {
                    let queue = matches!(
                        conn.state,
                        State::Established | State::FinWait1 | State::FinWait2
                    ) && (conn.app_alive || conn.state == State::Established);
                    if queue {
                        let rcv_nxt = conn.rcv_nxt;
                        let offset = u64::from(seq.wrapping_sub(rcv_nxt));
                        let rec = conn.rec_mut(self.cfg.initial_cwnd_segments);
                        if rec.ooo.insert(rcv_nxt, seq, data, wnd_now) > 0 {
                            self.counters.ooo_segments += 1;
                        }
                        if fin && offset + data.len() as u64 <= u64::from(wnd_now) {
                            rec.ooo.set_fin(seq.wrapping_add(data.len() as u32));
                        }
                    }
                    let window = conn.window_field(wnd_now as usize);
                    let opts = conn.tx_opts(ts_now);
                    self.out.send(
                        &mut self.counters,
                        (conn.local_ip, conn.remote_ip),
                        conn.local_port,
                        conn.remote_port,
                        conn.snd_nxt,
                        conn.rcv_nxt,
                        flags::ACK,
                        window,
                        None,
                        opts,
                        (&[], &[]),
                    );
                    conn.acked();
                    conn.adv_wnd = window;
                }
                data = &[];
                fin = false;
                seq = conn.rcv_nxt;
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
                conn.rto_us = self.cfg.rto_us;
                conn.snd_wnd = u32::from(h.window) << conn.snd_shift();
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
        let snd_max = conn.snd_max();
        let peer_wnd = u32::from(h.window) << conn.snd_shift();
        let control = h.flags & (flags::SYN | flags::FIN | flags::RST) != 0;
        // The SACK blocks come first: they refine what the acknowledgment
        // number says (RFC 6675 section 5).
        let sack_ok = conn.has(O_SACK);
        let una_now = conn.snd_una;
        let sack_new = sack_ok
            && !h.opts.blocks().is_empty()
            && conn
                .rec
                .as_deref_mut()
                .is_some_and(|r| r.sb.update(una_now, snd_max, h.opts.blocks()));
        let dup = recovery::is_dupack(&AckView {
            ack: h.ack,
            snd_una: conn.snd_una,
            snd_max,
            payload_len: payload.len(),
            control,
            window: peer_wnd,
            last_window: conn.snd_wnd,
        }) || (sack_new
            && h.ack == conn.snd_una
            && conn.snd_una != snd_max
            && payload.is_empty()
            && !control);
        if seq_lt(conn.snd_una, h.ack) && seq_le(h.ack, snd_max) {
            let advanced = h.ack.wrapping_sub(conn.snd_una);
            let fin_in_flight = conn.sent_fin || conn.rec.as_ref().is_some_and(|r| r.fin_sent);
            let fin_acked = fin_in_flight && h.ack == snd_max;
            let data_acked = (advanced - u32::from(fin_acked)) as usize;
            let drop_bytes = data_acked.min(conn.sndbuf.len());
            conn.sndbuf.drain(..drop_bytes);
            if conn.sndbuf.is_empty() {
                conn.sndbuf = VecDeque::new();
            }
            conn.snd_una = h.ack;
            if seq_lt(conn.snd_nxt, h.ack) {
                // The peer holds data that a timeout sent again: skip it.
                conn.snd_nxt = h.ack;
            }
            if fin_acked {
                conn.sent_fin = true;
            }
            // A timestamp echo gives a sample for any acknowledgment that moves
            // `snd_una`, a segment sent again included (RFC 7323 section 4.3),
            // with the resolution of the millisecond clock.
            let ts_sample = match h.opts.timestamp {
                Some((_, echo)) if conn.has(O_TS) && echo != 0 => {
                    let age = ts_now.wrapping_sub(echo) as i32;
                    (age >= 0).then(|| i64::from(age.max(1)) * 1000)
                }
                _ => None,
            };
            if let Some(rec) = conn.rec.as_deref_mut() {
                // Karn's rule: only a segment that was never sent again gives
                // a sample, and the timeout keeps its backoff until one does.
                // The timed segment has the finer clock, so it wins.
                if let Some(sample) = rec.take_sample(h.ack, now).or(ts_sample) {
                    rec.rtt.sample(sample);
                    if let Some(rto) = rec.rtt.rto_us(self.cfg.min_rto_us, self.cfg.max_rto_us) {
                        conn.rto_us = rto;
                    }
                }
                if rec.cc.on_new_ack(h.ack, advanced) == Action::Retransmit {
                    rec.rtx = true;
                    rec.retransmitted();
                }
                rec.sb.advance(h.ack);
                if !rec.cc.in_recovery() {
                    rec.sb.clear_rexmit();
                }
            }
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
        } else if seq_lt(snd_max, h.ack) {
            // An acknowledgment for data that was never sent.
            conn.ack_pending = true;
            self.mark_dirty(id);
            return;
        } else if dup {
            if let Some(rec) = conn.rec.as_deref_mut() {
                let flight = snd_max.wrapping_sub(conn.snd_una);
                if sack_ok {
                    let lost = rec.sb.lost_at_una(conn.snd_una, u32::from(conn.mss));
                    if rec.cc.on_dupack_sack(h.ack, flight, snd_max, lost) == Action::Retransmit {
                        // The scoreboard picks the segments. Without SACK
                        // information the first segment goes out at once
                        // (RFC 6675 section 5, step 4.1).
                        rec.sb.clear_rexmit();
                        rec.rtx = !lost;
                        rec.retransmitted();
                        self.counters.sack_recoveries += 1;
                    }
                } else if rec.cc.on_dupack(h.ack, flight, snd_max) == Action::Retransmit {
                    rec.rtx = true;
                    rec.retransmitted();
                }
            }
        }
        if !seq_lt(h.ack, conn.snd_una) {
            let wnd = peer_wnd;
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
        let mut held_fin = false;
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
                let behind_hole = conn.rec.as_ref().is_some_and(|r| !r.ooo.is_empty());
                if take > 0 {
                    conn.rcvbuf.extend(&data[..take]);
                    conn.rcv_nxt = conn.rcv_nxt.wrapping_add(take as u32);
                    if let Some(rec) = conn.rec.as_deref_mut() {
                        if !rec.ooo.is_empty() {
                            // The data may have filled a hole: what was held
                            // behind it joins the stream.
                            let room = self.cfg.rcv_buffer.saturating_sub(conn.rcvbuf.len());
                            rec.ooo
                                .drain_into(&mut conn.rcv_nxt, &mut conn.rcvbuf, room);
                            if rec.ooo.fin_at(conn.rcv_nxt) {
                                rec.ooo.clear_fin();
                                held_fin = true;
                            }
                        }
                    }
                    if let Some(waker) = conn.read_waker.take() {
                        waker.wake();
                    }
                }
                // A delayed ACK (RFC 1122 section 4.2.3.2, RFC 5681 section
                // 4.2): the first in order segment waits for a second one or
                // for the timer. Everything else is answered at once: data
                // that fills or follows a hole, data that did not fit, a
                // FIN, and any segment while a timeout is armed, which the
                // single timer of the connection could not serve together
                // with the delay.
                if self.cfg.delayed_ack
                    && all_taken
                    && !fin
                    && !behind_hole
                    && !conn.has(O_UNACKED)
                    && !conn.armed
                {
                    let due = now + self.cfg.delayed_ack_us;
                    conn.opts |= O_UNACKED | O_DELACK;
                    conn.deadline = due;
                    conn.armed = true;
                    if !conn.has(O_DQ) {
                        conn.opts |= O_DQ;
                        self.timers.push(Reverse((due, id, gen)));
                    }
                } else {
                    conn.ack_pending = true;
                }
            }
        }

        // FIN.
        let fin = fin || held_fin;
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
            if conn.recovery_idle() {
                conn.rec = None;
            }
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
                conn.rec = None;
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
            conn.opts &= !O_DQ;
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
        if conn.has(O_DELACK) {
            // The timer ran for a delayed ACK: send it.
            conn.set(O_DELACK, false);
            conn.ack_pending = true;
            self.counters.delayed_acks += 1;
            self.mark_dirty(id);
            return;
        }
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
                conn.rto_us = recovery::backoff_us(conn.rto_us, max_rto);
                let snd_max = conn.snd_max();
                if let Some(rec) = conn.rec.as_deref_mut() {
                    // The window falls to one segment and slow start sends
                    // the flight again (RFC 5681 section 3.1, RFC 6298 rule
                    // 5.4); a segment that is sent again gives no sample.
                    rec.cc
                        .on_timeout(snd_max.wrapping_sub(conn.snd_una), snd_max);
                    rec.snd_max = snd_max;
                    rec.rtx = false;
                    rec.retransmitted();
                }
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
                self.counters.timeouts += 1;
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

    #[allow(clippy::too_many_lines)]
    fn output(&mut self, id: u32, now: i64) {
        let gen = self.gens.get(id as usize).copied().unwrap_or(0);
        let window_size = self.cfg.rcv_buffer;
        let mss_own = self.mss_own;
        let ts_now = ts_clock(self.ts_base, now);
        let Some(conn) = self.conns.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        conn.dirty = false;
        let space = window_size.saturating_sub(conn.rcvbuf.len());
        let rcv_wnd = conn.window_field(space);
        let ips = (conn.local_ip, conn.remote_ip);
        let (lp, rp) = (conn.local_port, conn.remote_port);
        let mut sent_anything = false;
        let frames_before = self.counters.frames_out;

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
            // The window of a SYN is never scaled (RFC 7323 section 2.2).
            let syn_wnd = u16::try_from(space.min(65_535)).unwrap_or(65_535);
            let opts = conn.syn_opts(ts_now);
            self.out.send(
                &mut self.counters,
                ips,
                lp,
                rp,
                conn.iss,
                ack,
                bits,
                syn_wnd,
                Some(mss_own),
                opts,
                (&[], &[]),
            );
            conn.snd_nxt = conn.iss.wrapping_add(1);
            conn.acked();
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
            if conn.rec.as_ref().is_some_and(|r| r.rtx) {
                // Fast retransmit or a partial acknowledgment: the oldest
                // unacknowledged segment goes out again, and `snd_nxt` stays.
                let fin_out = conn.sent_fin || conn.rec.as_ref().is_some_and(|r| r.fin_sent);
                let outstanding = conn.snd_max().wrapping_sub(conn.snd_una) as usize;
                let data_out = outstanding - usize::from(fin_out).min(outstanding);
                let n = data_out.min(conn.sndbuf.len()).min(usize::from(conn.mss));
                let opts = conn.tx_opts(ts_now);
                if n > 0 {
                    let (a, b) = conn.sndbuf.as_slices();
                    let payload = slices_range(a, b, 0, n);
                    self.out.send(
                        &mut self.counters,
                        ips,
                        lp,
                        rp,
                        conn.snd_una,
                        conn.rcv_nxt,
                        flags::ACK,
                        rcv_wnd,
                        None,
                        opts,
                        payload,
                    );
                } else if fin_out && outstanding > 0 {
                    self.out.send(
                        &mut self.counters,
                        ips,
                        lp,
                        rp,
                        conn.snd_una,
                        conn.rcv_nxt,
                        flags::FIN | flags::ACK,
                        rcv_wnd,
                        None,
                        opts,
                        (&[], &[]),
                    );
                }
                self.counters.retransmits += 1;
                self.counters.fast_retransmits += 1;
                conn.acked();
                sent_anything = true;
                let una = conn.snd_una;
                if let Some(rec) = conn.rec.as_deref_mut() {
                    rec.rtx = false;
                    rec.retransmitted();
                    if rec.cc.sack_recovery() {
                        rec.sb.mark_rexmit(una, una, n as u32);
                    }
                }
            }
            if conn.rec.as_ref().is_some_and(|r| r.cc.sack_recovery()) {
                // RFC 6675 section 5: while the window leaves room beyond the
                // pipe, send the lowest lost range that was not sent again.
                let mss = u32::from(conn.mss);
                loop {
                    let una = conn.snd_una;
                    let high = conn.snd_max();
                    let Some(rec) = conn.rec.as_deref() else {
                        break;
                    };
                    let pipe = rec.sb.pipe(una, high, mss, true);
                    if rec.cc.cwnd().saturating_sub(pipe) < mss {
                        break;
                    }
                    let Some((start, len)) = rec.sb.next_hole(una, mss, true) else {
                        break;
                    };
                    let off = start.wrapping_sub(una) as usize;
                    let n = (len as usize).min(conn.sndbuf.len().saturating_sub(off));
                    if n == 0 {
                        break;
                    }
                    let opts = conn.tx_opts(ts_now);
                    let (a, b) = conn.sndbuf.as_slices();
                    let payload = slices_range(a, b, off, n);
                    self.out.send(
                        &mut self.counters,
                        ips,
                        lp,
                        rp,
                        start,
                        conn.rcv_nxt,
                        flags::ACK,
                        rcv_wnd,
                        None,
                        opts,
                        payload,
                    );
                    conn.acked();
                    self.counters.retransmits += 1;
                    self.counters.sack_retransmits += 1;
                    sent_anything = true;
                    if let Some(rec) = conn.rec.as_deref_mut() {
                        rec.sb.mark_rexmit(una, start, n as u32);
                        rec.retransmitted();
                    }
                }
            }
            loop {
                let unacked = conn.unacked_data();
                let avail = conn.sndbuf.len().saturating_sub(unacked);
                if avail == 0 {
                    break;
                }
                if conn.rec.is_none() {
                    conn.rec_mut(self.cfg.initial_cwnd_segments);
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
                // The congestion window never cuts a segment short: with less
                // room than the segment, the sender waits for an ACK. It
                // cannot stall, because the window is at least one segment.
                // In a recovery on the scoreboard the pipe is what counts
                // against the window, not the distance to `snd_nxt`.
                let cwnd_room = conn.rec.as_ref().map_or(usize::MAX, |r| {
                    let flight = if r.cc.sack_recovery() {
                        r.sb.pipe(conn.snd_una, conn.snd_max(), u32::from(conn.mss), true)
                    } else {
                        conn.in_flight()
                    };
                    r.cc.send_window().saturating_sub(flight) as usize
                });
                if cwnd_room < n {
                    break;
                }
                let opts = conn.tx_opts(ts_now);
                let (a, b) = conn.sndbuf.as_slices();
                let payload = slices_range(a, b, unacked, n);
                let bits = flags::ACK | if n == avail { flags::PSH } else { 0 };
                self.out.send(
                    &mut self.counters,
                    ips,
                    lp,
                    rp,
                    conn.snd_nxt,
                    conn.rcv_nxt,
                    bits,
                    rcv_wnd,
                    None,
                    opts,
                    payload,
                );
                let is_new = conn.snd_max() == conn.snd_nxt;
                conn.snd_nxt = conn.snd_nxt.wrapping_add(n as u32);
                conn.acked();
                conn.probe = false;
                sent_anything = true;
                let snd_nxt = conn.snd_nxt;
                if let Some(rec) = conn.rec.as_deref_mut() {
                    if is_new {
                        rec.time_segment(snd_nxt, now);
                    } else {
                        rec.retransmitted();
                    }
                    if seq_lt(rec.snd_max, snd_nxt) {
                        rec.snd_max = snd_nxt;
                    }
                }
            }
            let all_sent = conn.sndbuf.len() <= conn.unacked_data();
            if conn.fin_wanted && !conn.sent_fin && all_sent {
                let opts = conn.tx_opts(ts_now);
                self.out.send(
                    &mut self.counters,
                    ips,
                    lp,
                    rp,
                    conn.snd_nxt,
                    conn.rcv_nxt,
                    flags::FIN | flags::ACK,
                    rcv_wnd,
                    None,
                    opts,
                    (&[], &[]),
                );
                conn.sent_fin = true;
                conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
                conn.acked();
                let snd_nxt = conn.snd_nxt;
                if let Some(rec) = conn.rec.as_deref_mut() {
                    rec.fin_sent = true;
                    if seq_lt(rec.snd_max, snd_nxt) {
                        rec.snd_max = snd_nxt;
                    }
                }
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
            conn.acked();
            let opts = conn.tx_opts(ts_now);
            self.out.send(
                &mut self.counters,
                ips,
                lp,
                rp,
                conn.snd_nxt,
                conn.rcv_nxt,
                flags::ACK,
                rcv_wnd,
                None,
                opts,
                (&[], &[]),
            );
        }
        if self.counters.frames_out != frames_before {
            conn.adv_wnd = rcv_wnd;
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
mod recovery_tests;

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

    use crate::pools::Pools;

    /// The stations of a test and the addresses of their hosts.
    struct Built {
        client: Station,
        server: Station,
        client_ips: Vec<u32>,
        server_ips: Vec<u32>,
    }
    use fps_hosts::MacAddr;
    use std::net::Ipv4Addr;

    fn stations(clients: u32, servers: u32) -> Built {
        let pools =
            Pools::sized("10.1.0.0/16", clients, "10.2.0.0/16", servers, None).expect("pools");
        let built = pools.stations().expect("stations");
        Built {
            client: built.client,
            server: built.server,
            client_ips: pools.client.addresses().expect("addresses"),
            server_ips: pools.server.addresses().expect("addresses"),
        }
    }

    fn tuple(local_ip: u32, remote_ip: u32, local_port: u16, remote_port: u16) -> Tuple {
        Tuple {
            local_ip,
            remote_ip,
            local_port,
            remote_port,
        }
    }

    #[test]
    fn the_isn_depends_on_the_tuple_the_secret_and_the_time() {
        let cfg = OwnConfig::default();
        let built = stations(2, 2);
        let host = Host::new(built.client.clone(), cfg.clone(), 1500);
        let t = tuple(1, 2, 40000, 80);
        let a = host.isn(t, 0);
        assert_ne!(a, host.isn(tuple(1, 2, 40001, 80), 0));
        assert_ne!(a, host.isn(tuple(1, 2, 40000, 81), 0));
        assert_ne!(a, host.isn(tuple(3, 2, 40000, 80), 0));
        assert_ne!(a, host.isn(tuple(1, 3, 40000, 80), 0));
        assert_eq!(host.isn(t, 4_000_000).wrapping_sub(a), 1_000_000);
        let other = Host::new(
            built.client,
            OwnConfig {
                isn_secret: 7,
                ..cfg
            },
            1500,
        );
        assert_ne!(a, other.isn(t, 0));
    }

    #[test]
    fn an_entry_of_the_table_takes_16_bytes() {
        assert_eq!(std::mem::size_of::<Tuple>(), 12);
        assert_eq!(std::mem::size_of::<(Tuple, u32)>(), 16);
    }

    #[test]
    fn the_hasher_spreads_tuples_that_differ_in_one_field() {
        use std::hash::{BuildHasher, BuildHasherDefault};
        let build = BuildHasherDefault::<IntHasher>::default();
        let mut seen = std::collections::HashSet::new();
        // 64 buckets of a table: the low 6 bits of the hash.
        for i in 0..64u32 {
            seen.insert(build.hash_one(tuple(i, 7, 5000, 80)) & 63);
            seen.insert(build.hash_one(tuple(7, i, 5000, 80)) & 63);
        }
        assert!(seen.len() > 40, "only {} buckets used", seen.len());
    }

    /// A SYN from client host `src` to server host `dst`, as a frame.
    fn syn(built: &Built, src: u32, dst: u32, sp: u16, dp: u16) -> Vec<u8> {
        let mut frame = Vec::new();
        assert!(built.client.head(&mut frame, src, dst));
        let header = Header {
            src_ip: src,
            dst_ip: dst,
            src_port: sp,
            dst_port: dp,
            seq: 1000,
            ack: 0,
            flags: flags::SYN,
            window: 1000,
            mss: Some(1460),
            opts: Options::default(),
        };
        wire::encode(&mut frame, &header, (&[], &[]));
        frame
    }

    fn server_with(built: &Built) -> Host {
        let mut server = Host::new(built.server.clone(), OwnConfig::default(), 1500);
        server.listen(80).expect("listen");
        server
    }

    #[test]
    fn the_table_keeps_connections_that_differ_only_by_address() {
        let built = stations(2, 2);
        let (c, s) = (&built.client_ips, &built.server_ips);
        let mut server = server_with(&built);
        // The same ports and three different address pairs.
        for (src, dst) in [(c[0], s[0]), (c[1], s[0]), (c[0], s[1])] {
            server.input(&syn(&built, src, dst, 5000, 80), 0);
        }
        assert_eq!(server.sockets(), 4, "the listener and three connections");
        server.flush_dirty(0);
        let replies: Vec<(u32, u32)> = server
            .out
            .queue
            .iter()
            .map(|f| {
                let h = wire::parse_frame(f).expect("frame").header;
                assert_eq!(h.flags, flags::SYN | flags::ACK);
                (h.src_ip, h.dst_ip)
            })
            .collect();
        assert_eq!(replies, vec![(s[0], c[0]), (s[0], c[1]), (s[1], c[0])]);
    }

    #[test]
    fn a_repeated_syn_finds_its_connection_in_the_table() {
        let built = stations(2, 1);
        let (c, s) = (&built.client_ips, &built.server_ips);
        let mut server = server_with(&built);
        let frame = syn(&built, c[1], s[0], 5000, 80);
        server.input(&frame, 0);
        server.input(&frame, 1);
        assert_eq!(server.sockets(), 2);
    }

    #[test]
    fn a_syn_to_an_address_of_no_host_is_ignored() {
        let built = stations(1, 1);
        let mut server = server_with(&built);
        let mut frame = Vec::new();
        assert!(built
            .client
            .head(&mut frame, built.client_ips[0], built.server_ips[0]));
        let header = Header {
            src_ip: built.client_ips[0],
            dst_ip: 0x0A02_0063,
            src_port: 5000,
            dst_port: 80,
            seq: 1,
            ack: 0,
            flags: flags::SYN,
            window: 1,
            mss: None,
            opts: Options::default(),
        };
        wire::encode(&mut frame, &header, (&[], &[]));
        server.input(&frame, 0);
        assert_eq!(server.sockets(), 1);
        assert!(server.out.queue.is_empty());
    }

    #[test]
    fn a_syn_from_an_unknown_neighbor_gets_no_answer() {
        let built = stations(1, 1);
        let mut server = server_with(&built);
        let mut frame = Vec::new();
        // The MAC addresses are right, the source address is in no pool.
        assert!(built
            .client
            .head(&mut frame, built.client_ips[0], built.server_ips[0]));
        let header = Header {
            src_ip: 0x0A09_0001,
            dst_ip: built.server_ips[0],
            src_port: 5000,
            dst_port: 80,
            seq: 1,
            ack: 0,
            flags: flags::SYN,
            window: 1,
            mss: None,
            opts: Options::default(),
        };
        wire::encode(&mut frame, &header, (&[], &[]));
        server.input(&frame, 0);
        assert_eq!(server.sockets(), 1);
        assert_eq!(server.counters.unresolved, 1);
    }

    #[test]
    fn open_checks_both_addresses_and_picks_ports_per_tuple() {
        let built = stations(2, 2);
        let (c, s) = (built.client_ips.clone(), built.server_ips.clone());
        let mut client = Host::new(built.client, OwnConfig::default(), 1500);
        let err = client.open(0x0A01_00FF, s[0], 80, 0).expect_err("local");
        assert_eq!(err.kind(), io::ErrorKind::AddrNotAvailable);
        let err = client.open(c[0], 0x0A02_00FF, 80, 0).expect_err("remote");
        assert_eq!(err.kind(), io::ErrorKind::HostUnreachable);
        for pair in [(c[0], s[0]), (c[1], s[0]), (c[0], s[1]), (c[0], s[0])] {
            client.open(pair.0, pair.1, 80, 0).expect("open");
        }
        assert_eq!(client.sockets(), 4);
        // Four distinct tuples in the table.
        assert_eq!(client.table.len(), 4);
        client.flush_dirty(0);
        let sources: Vec<MacAddr> = client
            .out
            .queue
            .iter()
            .map(|f| crate::net::decode_header(f).expect("eth").src)
            .collect();
        assert_eq!(sources.len(), 4);
        assert_ne!(sources[0], sources[1]);
        assert_eq!(sources[0], sources[3]);
    }

    #[test]
    fn a_host_answers_arp_for_a_host_of_its_station() {
        let built = stations(1, 2);
        let mut server = server_with(&built);
        let asker = MacAddr([0x02, 0xcc, 0, 0, 0, 1]);
        let target = Ipv4Addr::from(built.server_ips[1]);
        let request = fps_hosts::wire::arp_request(asker, Ipv4Addr::new(10, 1, 0, 99), target, &[]);
        server.input(&request, 0);
        assert_eq!(server.counters.arp_replies, 1);
        let reply = server.out.queue.pop_front().expect("reply");
        let arp = fps_hosts::wire::parse_arp(&reply).expect("arp");
        assert_eq!(
            Some(arp.sender_mac),
            built.server.local_mac(built.server_ips[1])
        );
        assert_eq!(arp.target_mac, asker);
        // A request for an address of the client pool is not ours.
        let other = fps_hosts::wire::arp_request(
            asker,
            Ipv4Addr::new(10, 1, 0, 99),
            Ipv4Addr::from(built.client_ips[0]),
            &[],
        );
        server.input(&other, 0);
        assert_eq!(server.counters.arp_replies, 1);
    }
}
