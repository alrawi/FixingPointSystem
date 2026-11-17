// SPDX-License-Identifier: GPL-3.0-only
//! The DHCP clients of the hosts of one slice, on one queue of a user space
//! backend.
//!
//! [`DhcpHosts`] owns one [`Client`] per host of the slice, with the MAC
//! address `mac_base + i` that [`crate::MacAllocator`] gave the host. It turns
//! the messages of the clients into Ethernet frames, reads the DHCP replies
//! out of the received frames, keeps a timer heap so that a pool of 20,000
//! hosts costs one heap pop per due message, and fills a [`LeaseTable`] with
//! the addresses that the servers gave.
//!
//! One value belongs to one queue and one thread, as [`FrameIo`] asks. A
//! typical loop receives a batch with [`crate::NeighborService`], calls
//! [`DhcpHosts::handle_frame`] for each frame that the service passes, and then
//! calls [`DhcpHosts::send_due`]. When [`LeaseTable::generation`] has changed
//! since the last turn, the loop makes a new [`crate::NeighborTable`] from
//! [`crate::NeighborTable::add_leased`] and a new [`crate::Responder`], so ARP
//! requests for the new address get an answer. [`DhcpHosts::poll`] does the
//! receive and the send in one call for a loop that has no neighbor service.
//!
//! `send_due` sends at most [`DhcpConfig::burst`] new messages per call, so a
//! start of 20,000 hosts does not flood the server, and it keeps the frames
//! that the transmit queue had no room for until the next call.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::io;
use std::time::Duration;

use fps_backend_afxdp::FrameIo;

use super::client::{Client, Dest, Event, Outgoing, Timing};
use super::frame::{parse_udp4, udp4_frame};
use super::leases::{HostLease, LeaseTable};
use super::msg::{Message, MessageType, CLIENT_PORT, SERVER_PORT};
use crate::mac::MacBlock;
use crate::pool::{HostPool, MacAddr};
use crate::wire::{BROADCAST_MAC, MAX_VLAN_TAGS};

/// How often a send that makes no progress is retried after a flush before the
/// frames wait for the next call.
const SEND_RETRIES: u32 = 8;

/// Errors that making [`DhcpHosts`] reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DhcpError {
    /// The pool has static addresses.
    #[error("host pool {pool}: the hosts lease no addresses, the pool has no dhcp4")]
    NotDhcp {
        /// The pool name.
        pool: String,
    },
    /// The MAC block belongs to another pool than the one given.
    #[error("host pool {pool}: the MAC block is for pool {block_pool}")]
    BlockMismatch {
        /// The pool name.
        pool: String,
        /// The pool of the block.
        block_pool: String,
    },
    /// The slice holds no host or lies outside the pool.
    #[error("host pool {pool}: slice {offset}+{count} is empty or outside the pool of {pool_count} hosts")]
    BadSlice {
        /// The pool name.
        pool: String,
        /// The first host index of the slice.
        offset: u32,
        /// The number of hosts of the slice.
        count: u32,
        /// The number of hosts of the pool.
        pool_count: u32,
    },
    /// More VLAN tags than the frame code looks through.
    #[error("host pool {pool}: {tags} VLAN tags, at most {MAX_VLAN_TAGS} are supported")]
    TooManyVlans {
        /// The pool name.
        pool: String,
        /// The number of tags asked for.
        tags: usize,
    },
}

/// The settings of [`DhcpHosts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhcpConfig {
    /// The timers of every client.
    pub timing: Timing,
    /// The VLAN ids that every frame of the clients carries, outermost first,
    /// at most two.
    pub vlans: Vec<u16>,
    /// The most new messages that one call of [`DhcpHosts::send_due`] makes.
    /// A value of 0 becomes 1.
    pub burst: usize,
    /// The run seed that the transaction identifiers and the jitter derive
    /// from.
    pub seed: u64,
}

impl Default for DhcpConfig {
    fn default() -> Self {
        Self {
            timing: Timing::default(),
            vlans: Vec::new(),
            burst: 256,
            seed: 0,
        }
    }
}

/// Counters of one [`DhcpHosts`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DhcpStats {
    /// DISCOVER messages sent, retransmissions included.
    pub discovers: u64,
    /// REQUEST messages sent that answer an OFFER, retransmissions included.
    pub requests: u64,
    /// REQUEST messages sent by unicast to renew a lease at T1.
    pub renews: u64,
    /// REQUEST messages sent by broadcast to rebind a lease at T2.
    pub rebinds: u64,
    /// RELEASE messages sent.
    pub releases: u64,
    /// Replies that reached a client.
    pub replies: u64,
    /// Hosts that got an address that they did not hold, counting a changed
    /// address after a renewal again.
    pub bound: u64,
    /// Leases extended for the same address.
    pub renewed: u64,
    /// Addresses lost to a NAK or to the end of the lease.
    pub lost: u64,
    /// DHCP frames that no host of the slice owns.
    pub foreign: u64,
    /// DHCP frames that are not valid DHCP messages.
    pub malformed: u64,
    /// Frames longer than the queue carries, which were dropped.
    pub oversize: u64,
    /// Frames handed to the queue.
    pub tx_frames: u64,
}

/// The DHCP clients of the hosts of one slice.
#[derive(Debug)]
pub struct DhcpHosts {
    macs: MacBlock,
    vlans: Vec<u16>,
    burst: usize,
    clients: Vec<Client>,
    leases: LeaseTable,
    timers: BinaryHeap<Reverse<(Duration, usize)>>,
    pending: VecDeque<Vec<u8>>,
    stats: DhcpStats,
}

impl DhcpHosts {
    /// Returns the clients of the hosts that `macs` covers in `pool`, which
    /// must be the block that [`crate::MacAllocator::claim`] returned for the
    /// slice. The first DISCOVER of each host is due after a random wait that
    /// starts at `now`.
    pub fn new(
        pool: &HostPool,
        macs: &MacBlock,
        config: &DhcpConfig,
        now: Duration,
    ) -> Result<Self, DhcpError> {
        let name = pool.name().to_owned();
        if !pool.is_dhcp() {
            return Err(DhcpError::NotDhcp { pool: name });
        }
        if macs.pool() != pool.name() {
            return Err(DhcpError::BlockMismatch {
                pool: name,
                block_pool: macs.pool().to_owned(),
            });
        }
        let range = macs.range();
        if range.is_empty() || range.end() > u64::from(pool.count()) {
            return Err(DhcpError::BadSlice {
                pool: name,
                offset: range.offset,
                count: range.count,
                pool_count: pool.count(),
            });
        }
        if config.vlans.len() > MAX_VLAN_TAGS {
            return Err(DhcpError::TooManyVlans {
                pool: name,
                tags: config.vlans.len(),
            });
        }
        let mut clients = Vec::with_capacity(range.count as usize);
        let mut timers = Vec::with_capacity(range.count as usize);
        for index in range.indexes() {
            // The block covers the range, so every index has an address.
            let mac = macs.mac_of(index).unwrap_or(MacAddr([0; 6]));
            let c = Client::new(index, mac, config.timing, config.seed, now);
            if let Some(d) = c.deadline() {
                timers.push(Reverse((d, clients.len())));
            }
            clients.push(c);
        }
        Ok(Self {
            macs: macs.clone(),
            vlans: config.vlans.clone(),
            burst: config.burst.max(1),
            clients,
            leases: LeaseTable::new(range),
            timers: BinaryHeap::from(timers),
            pending: VecDeque::new(),
            stats: DhcpStats::default(),
        })
    }

    /// The leases so far.
    pub fn leases(&self) -> &LeaseTable {
        &self.leases
    }

    /// The counters since the hosts were made.
    pub fn stats(&self) -> DhcpStats {
        self.stats
    }

    /// The clients, in host order.
    pub fn clients(&self) -> &[Client] {
        &self.clients
    }

    /// A time at which [`DhcpHosts::send_due`] may have work, or `None` when no
    /// client has a timer. It is never later than the next real deadline, and
    /// may be earlier, so a loop that sleeps until it wakes at most once too
    /// often.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.timers.peek().map(|Reverse((d, _))| *d)
    }

    fn schedule(&mut self, slot: usize) {
        if let Some(d) = self.clients.get(slot).and_then(Client::deadline) {
            self.timers.push(Reverse((d, slot)));
        }
    }

    fn apply(&mut self, index: u32, event: Event) {
        match event {
            Event::None => {}
            Event::Bound(l) => {
                let changed = self.leases.set(
                    index,
                    HostLease {
                        addr: l.addr,
                        prefix_len: l.prefix_len(),
                        router: l.router(),
                    },
                );
                if changed {
                    self.stats.bound += 1;
                } else {
                    self.stats.renewed += 1;
                }
            }
            Event::Renewed(l) => {
                self.leases.set(
                    index,
                    HostLease {
                        addr: l.addr,
                        prefix_len: l.prefix_len(),
                        router: l.router(),
                    },
                );
                self.stats.renewed += 1;
            }
            Event::Lost(_) => {
                if self.leases.clear(index) {
                    self.stats.lost += 1;
                }
            }
        }
    }

    /// Reads one received frame. It returns `true` when the frame is a UDP
    /// datagram to the DHCP client port, whether or not a host of the slice
    /// owns it, because no other consumer wants such a frame. Every other
    /// frame returns `false` and is the caller's.
    ///
    /// A reply goes to the host whose MAC address is the client hardware
    /// address of the message, when the frame was sent to that address or to
    /// the broadcast address.
    pub fn handle_frame(&mut self, frame: &[u8], now: Duration) -> bool {
        let Some(udp) = parse_udp4(frame) else {
            return false;
        };
        if udp.dst_port != CLIENT_PORT {
            return false;
        }
        if udp.src_port != SERVER_PORT {
            self.stats.foreign += 1;
            return true;
        }
        let Ok(msg) = Message::parse(udp.payload) else {
            self.stats.malformed += 1;
            return true;
        };
        let owner = self
            .macs
            .host_of(msg.chaddr)
            .filter(|_| udp.eth_dst == msg.chaddr || udp.eth_dst == BROADCAST_MAC);
        let Some(index) = owner else {
            self.stats.foreign += 1;
            return true;
        };
        let slot = (index - self.macs.range().offset) as usize;
        let Some(client) = self.clients.get_mut(slot) else {
            self.stats.foreign += 1;
            return true;
        };
        let before = client.deadline();
        let step = client.receive(now, &msg, udp.eth_src);
        let after = client.deadline();
        self.stats.replies += 1;
        self.apply(index, step.event);
        if after != before {
            self.schedule(slot);
        }
        true
    }

    fn count(&mut self, out: &Outgoing) {
        let s = &mut self.stats;
        match (out.msg.kind, out.dest) {
            (MessageType::Discover, _) => s.discovers += 1,
            (MessageType::Release, _) => s.releases += 1,
            (MessageType::Request, Dest::Broadcast) if out.msg.ciaddr.is_unspecified() => {
                s.requests += 1;
            }
            (MessageType::Request, Dest::Broadcast) => s.rebinds += 1,
            (MessageType::Request, Dest::Unicast { .. }) => s.renews += 1,
            _ => {}
        }
    }

    fn render(&self, out: &Outgoing, mac: MacAddr) -> Vec<u8> {
        let (eth_dst, ip_dst) = match out.dest {
            Dest::Broadcast => (BROADCAST_MAC, std::net::Ipv4Addr::BROADCAST),
            Dest::Unicast { ip, mac } => (mac, ip),
        };
        udp4_frame(
            eth_dst,
            mac,
            &self.vlans,
            out.src_ip,
            ip_dst,
            CLIENT_PORT,
            SERVER_PORT,
            &out.msg.encode(),
        )
    }

    /// Sends what is due at `now`: first the frames that an earlier call could
    /// not queue, then the messages of the clients whose timers have passed,
    /// at most [`DhcpConfig::burst`] of them. It returns the number of new
    /// messages. A frame that waits for room is sent by the next call.
    ///
    /// # Errors
    ///
    /// Returns the error of the queue, and returns
    /// [`io::ErrorKind::InvalidInput`] when the queue is too short for any
    /// frame of the clients.
    pub fn send_due<F: FrameIo + ?Sized>(
        &mut self,
        io: &mut F,
        now: Duration,
    ) -> io::Result<usize> {
        let max_len = io.max_frame_len();
        let mut made = 0;
        while made < self.burst && self.pending.len() < self.burst {
            let Some(Reverse((d, slot))) = self.timers.peek().copied() else {
                break;
            };
            if d > now {
                break;
            }
            self.timers.pop();
            let Some(client) = self.clients.get_mut(slot) else {
                continue;
            };
            // A timer that the client moved on is stale.
            if client.deadline() != Some(d) {
                continue;
            }
            let (index, mac) = (client.index(), client.mac());
            let step = client.poll(now);
            self.apply(index, step.event);
            self.schedule(slot);
            if let Some(out) = step.send {
                let frame = self.render(&out, mac);
                if frame.len() > max_len {
                    self.stats.oversize += 1;
                    continue;
                }
                self.count(&out);
                self.pending.push_back(frame);
                made += 1;
            }
        }
        self.flush(io)?;
        Ok(made)
    }

    /// Gives every lease back with a RELEASE message and forgets the
    /// addresses. The clients stop, so call it when the run ends. It returns
    /// the number of messages made.
    ///
    /// # Errors
    ///
    /// Returns the error of the queue.
    pub fn release_all<F: FrameIo + ?Sized>(
        &mut self,
        io: &mut F,
        now: Duration,
    ) -> io::Result<usize> {
        let max_len = io.max_frame_len();
        let mut made = 0;
        for slot in 0..self.clients.len() {
            let Some(client) = self.clients.get_mut(slot) else {
                continue;
            };
            let (index, mac) = (client.index(), client.mac());
            let out = client.release(now);
            self.leases.clear(index);
            if let Some(out) = out {
                let frame = self.render(&out, mac);
                if frame.len() <= max_len {
                    self.count(&out);
                    self.pending.push_back(frame);
                    made += 1;
                }
            }
        }
        self.timers.clear();
        self.flush(io)?;
        Ok(made)
    }

    /// The number of frames that wait for room in the transmit queue.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    fn flush<F: FrameIo + ?Sized>(&mut self, io: &mut F) -> io::Result<()> {
        let mut stalled = 0;
        while !self.pending.is_empty() {
            let total = self.pending.len();
            let queued = {
                let frames = &self.pending;
                io.send_batch_with(total, &mut |i, buf| match frames.get(i) {
                    Some(f) => match buf.get_mut(..f.len()) {
                        Some(dst) => {
                            dst.copy_from_slice(f);
                            f.len()
                        }
                        None => 0,
                    },
                    None => 0,
                })?
            };
            for _ in 0..queued.min(total) {
                self.pending.pop_front();
            }
            self.stats.tx_frames += queued as u64;
            if queued == 0 {
                stalled += 1;
                io.flush()?;
                if stalled > SEND_RETRIES {
                    break;
                }
            } else {
                stalled = 0;
            }
        }
        io.flush()
    }

    /// Receives one batch of at most `batch` frames, reads the DHCP replies,
    /// calls `pass` for every other frame, and sends what is due. It returns
    /// the number of frames received and never blocks.
    ///
    /// A loop that also answers ARP uses [`crate::NeighborService::poll`] and
    /// [`DhcpHosts::handle_frame`] instead, because both read the same queue.
    ///
    /// # Errors
    ///
    /// Returns the error of the queue.
    pub fn poll<F: FrameIo + ?Sized>(
        &mut self,
        io: &mut F,
        batch: usize,
        now: Duration,
        pass: &mut dyn FnMut(&[u8]),
    ) -> io::Result<usize> {
        let received = io.recv_batch(batch.max(1), &mut |frame| {
            if !self.handle_frame(frame, now) {
                pass(frame);
            }
        })?;
        self.send_due(io, now)?;
        Ok(received)
    }
}
