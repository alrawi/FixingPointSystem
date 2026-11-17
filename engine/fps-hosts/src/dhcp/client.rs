// SPDX-License-Identifier: GPL-3.0-only
//! The DHCPv4 client of one host, as a state machine without I/O.
//!
//! [`Client`] follows the states of RFC 2131 section 4.4: it discovers, picks
//! the first offer, requests it, holds the lease, renews it by unicast at T1,
//! rebinds by broadcast at T2, and starts over when the lease runs out or a
//! server answers with a NAK. It reads no clock and no random source of its
//! own. The caller passes the time as a [`Duration`] since any fixed start,
//! calls [`Client::poll`] when [`Client::deadline`] has passed, and hands every
//! reply to [`Client::receive`]. The seed of the client fixes its transaction
//! identifiers and its timer jitter, so a run repeats.
//!
//! Choices that RFC 2131 leaves to the client:
//!
//! * The first message of a run waits a random time below
//!   [`Timing::start_delay_max`], so the hosts of a pool do not discover in
//!   one burst.
//! * A message that gets no answer is sent again after
//!   [`Timing::initial_retransmit`] doubled per try up to
//!   [`Timing::max_retransmit`], randomized by a quarter either way.
//! * The client takes the first OFFER. It sets the broadcast flag before it
//!   has an address, so a server that answers by broadcast needs no ARP entry
//!   for the new address.
//! * T1 and T2 come from options 58 and 59 when `0 < T1 < T2 < lease`, and are
//!   half and seven eighths of the lease time otherwise. The lease runs from
//!   the first REQUEST of the exchange.
//! * A lease time of `u32::MAX` is infinite, so the client never renews.
//! * The client does not probe the address with ARP before it uses it and
//!   never sends a DECLINE.

use std::net::Ipv4Addr;
use std::time::Duration;

use super::msg::{Message, MessageType, Options, BOOTREPLY, FLAG_BROADCAST};
use crate::pool::MacAddr;

/// The timers of a [`Client`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// The longest random wait before the first DISCOVER.
    pub start_delay_max: Duration,
    /// The wait after the first message of an exchange, before the first
    /// retransmission.
    pub initial_retransmit: Duration,
    /// The longest wait between retransmissions of DISCOVER and of the
    /// REQUEST that answers an OFFER.
    pub max_retransmit: Duration,
    /// How many times the REQUEST that answers an OFFER is sent before the
    /// client starts over with a DISCOVER.
    pub request_tries: u32,
    /// The wait before the client starts over after a NAK.
    pub nak_delay: Duration,
    /// The shortest wait between two renewal or rebinding messages. RFC 2131
    /// asks for 60 seconds.
    pub min_renew_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            start_delay_max: Duration::from_secs(1),
            initial_retransmit: Duration::from_secs(4),
            max_retransmit: Duration::from_secs(64),
            request_tries: 4,
            nak_delay: Duration::from_secs(1),
            min_renew_retry: Duration::from_secs(60),
        }
    }
}

/// The state of a [`Client`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No lease and no exchange. The client waits for its timer to send a
    /// DISCOVER.
    Init,
    /// DISCOVER sent, no usable OFFER yet.
    Selecting,
    /// An OFFER was taken, the REQUEST for it is due or sent.
    Requesting,
    /// A lease is held and T1 has not passed.
    Bound,
    /// T1 passed. The client asks its server by unicast to extend the lease.
    Renewing,
    /// T2 passed. The client asks any server by broadcast to extend the lease.
    Rebinding,
}

/// The lease that a server gave a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// The address of the client.
    pub addr: Ipv4Addr,
    /// The subnet mask, if the server sent one.
    pub subnet_mask: Option<Ipv4Addr>,
    /// The routers in order of preference.
    pub routers: Vec<Ipv4Addr>,
    /// The identifier of the server, which is its address.
    pub server: Ipv4Addr,
    /// The Ethernet address that the server's last reply came from.
    pub server_mac: MacAddr,
    /// When the exchange that gave the lease sent its first REQUEST.
    pub granted: Duration,
    /// When the client starts to renew, or `None` for an infinite lease.
    pub renew_at: Option<Duration>,
    /// When the client starts to rebind, or `None` for an infinite lease.
    pub rebind_at: Option<Duration>,
    /// When the lease ends, or `None` for an infinite lease.
    pub expires_at: Option<Duration>,
}

impl Lease {
    /// The length of the prefix that the subnet mask names, or `None` without
    /// a mask or with a mask whose one bits are not contiguous.
    pub fn prefix_len(&self) -> Option<u8> {
        let m = u32::from(self.subnet_mask?);
        let ones = m.leading_ones();
        (m.checked_shl(ones).unwrap_or(0) == 0).then(|| u8::try_from(ones).unwrap_or(32))
    }

    /// The first router, which the host uses as its gateway.
    pub fn router(&self) -> Option<Ipv4Addr> {
        self.routers.first().copied()
    }
}

/// Where a message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dest {
    /// To the Ethernet broadcast address and the IPv4 address 255.255.255.255.
    Broadcast,
    /// To one server.
    Unicast {
        /// The IPv4 address of the server.
        ip: Ipv4Addr,
        /// The Ethernet address of the server.
        mac: MacAddr,
    },
}

/// A message that the client wants sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The message.
    pub msg: Message,
    /// Where it goes.
    pub dest: Dest,
    /// The IPv4 source address: 0.0.0.0 before the client holds a lease, and
    /// its address after.
    pub src_ip: Ipv4Addr,
}

/// What the client learned from a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Nothing that the owner of the address table needs to know.
    None,
    /// The client holds a lease for an address that it did not hold before,
    /// either the first lease or a new address after a renewal.
    Bound(Lease),
    /// The client extended the lease for the same address.
    Renewed(Lease),
    /// The client lost its address, by a NAK or because the lease ended.
    Lost(Ipv4Addr),
}

/// The result of [`Client::poll`] and [`Client::receive`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The message to send now, if any.
    pub send: Option<Outgoing>,
    /// The change of the lease, if any.
    pub event: Event,
}

impl Step {
    fn nothing() -> Self {
        Self {
            send: None,
            event: Event::None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Offer {
    addr: Ipv4Addr,
    server: Ipv4Addr,
}

/// The options that the client asks every server for: mask, router, DNS,
/// lease time, T1, and T2.
const PARAMS: [u8; 6] = [1, 3, 6, 51, 58, 59];

/// The DHCP client of one host.
#[derive(Debug, Clone)]
pub struct Client {
    index: u32,
    mac: MacAddr,
    timing: Timing,
    rng: u64,
    state: State,
    xid: u32,
    exchange_start: Duration,
    request_at: Duration,
    deadline: Option<Duration>,
    tries: u32,
    offer: Option<Offer>,
    lease: Option<Lease>,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Client {
    /// Returns the client of host `index` with the hardware address `mac`.
    ///
    /// `seed` derives the transaction identifiers and the jitter, and `now` is
    /// the time of creation. The first DISCOVER is due after a random wait
    /// below [`Timing::start_delay_max`].
    pub fn new(index: u32, mac: MacAddr, timing: Timing, seed: u64, now: Duration) -> Self {
        // A domain constant keeps the stream apart from other users of the
        // same seed, such as host selection.
        let mut rng = seed ^ 0xd4c9_2d0a_5eed_4001 ^ (u64::from(index) << 24);
        splitmix64(&mut rng);
        let mut c = Self {
            index,
            mac,
            timing,
            rng,
            state: State::Init,
            xid: 0,
            exchange_start: now,
            request_at: now,
            deadline: None,
            tries: 0,
            offer: None,
            lease: None,
        };
        let wait = c.random_below(timing.start_delay_max);
        c.deadline = Some(now + wait);
        c
    }

    /// The host index.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The hardware address.
    pub fn mac(&self) -> MacAddr {
        self.mac
    }

    /// The state.
    pub fn state(&self) -> State {
        self.state
    }

    /// The transaction identifier of the current exchange.
    pub fn xid(&self) -> u32 {
        self.xid
    }

    /// The lease, while the client holds one. A client that renews or rebinds
    /// still holds it.
    pub fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    /// When [`Client::poll`] has work, or `None` when the client waits for a
    /// message only, as an infinite lease does.
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    fn random_below(&mut self, max: Duration) -> Duration {
        let us = u64::try_from(max.as_micros()).unwrap_or(u64::MAX);
        if us == 0 {
            return Duration::ZERO;
        }
        Duration::from_micros(splitmix64(&mut self.rng) % us)
    }

    /// The wait after the `tries`-th message of an exchange: the base doubles
    /// per try up to the maximum, and the wait varies by a quarter either way.
    fn retransmit_delay(&mut self, tries: u32) -> Duration {
        let base = self
            .timing
            .initial_retransmit
            .saturating_mul(1u32 << tries.saturating_sub(1).min(16))
            .min(self.timing.max_retransmit);
        let us = u64::try_from(base.as_micros()).unwrap_or(u64::MAX);
        let quarter = us / 4;
        let r = splitmix64(&mut self.rng) % (2 * quarter + 1);
        Duration::from_micros(us - quarter + r)
    }

    fn new_exchange(&mut self, now: Duration) {
        self.xid = (splitmix64(&mut self.rng) >> 32) as u32;
        self.exchange_start = now;
        self.request_at = now;
        self.tries = 0;
    }

    fn secs(&self, now: Duration) -> u16 {
        u16::try_from(now.saturating_sub(self.exchange_start).as_secs()).unwrap_or(u16::MAX)
    }

    fn message(&self, kind: MessageType, now: Duration) -> Message {
        let mut m = Message::new(kind, self.xid, self.mac);
        m.secs = self.secs(now);
        let mut id = vec![1];
        id.extend_from_slice(&self.mac.0);
        m.options = Options {
            client_id: Some(id),
            param_request: PARAMS.to_vec(),
            ..Options::default()
        };
        m
    }

    fn discover(&self, now: Duration) -> Outgoing {
        let mut m = self.message(MessageType::Discover, now);
        m.flags = FLAG_BROADCAST;
        Outgoing {
            msg: m,
            dest: Dest::Broadcast,
            src_ip: Ipv4Addr::UNSPECIFIED,
        }
    }

    fn select_request(&self, offer: &Offer, now: Duration) -> Outgoing {
        let mut m = self.message(MessageType::Request, now);
        m.flags = FLAG_BROADCAST;
        m.options.requested_ip = Some(offer.addr);
        m.options.server_id = Some(offer.server);
        Outgoing {
            msg: m,
            dest: Dest::Broadcast,
            src_ip: Ipv4Addr::UNSPECIFIED,
        }
    }

    fn extend_request(&self, lease: &Lease, rebinding: bool, now: Duration) -> Outgoing {
        let mut m = self.message(MessageType::Request, now);
        m.ciaddr = lease.addr;
        let dest = if rebinding {
            Dest::Broadcast
        } else {
            Dest::Unicast {
                ip: lease.server,
                mac: lease.server_mac,
            }
        };
        Outgoing {
            msg: m,
            dest,
            src_ip: lease.addr,
        }
    }

    /// Returns the message that gives the lease back, and forgets the lease.
    ///
    /// The client then stops: it has no timer and ignores every message until
    /// the owner makes a new client. It returns `None` for a client without a
    /// lease, which stops all the same.
    pub fn release(&mut self, now: Duration) -> Option<Outgoing> {
        let lease = self.lease.take();
        self.state = State::Init;
        self.deadline = None;
        self.offer = None;
        let lease = lease?;
        self.xid = (splitmix64(&mut self.rng) >> 32) as u32;
        let mut m = self.message(MessageType::Release, now);
        m.ciaddr = lease.addr;
        m.options.server_id = Some(lease.server);
        m.options.param_request.clear();
        Some(Outgoing {
            msg: m,
            dest: Dest::Unicast {
                ip: lease.server,
                mac: lease.server_mac,
            },
            src_ip: lease.addr,
        })
    }

    /// Does what is due at `now`. Call it when `now` has reached
    /// [`Client::deadline`]. Before that it does nothing.
    pub fn poll(&mut self, now: Duration) -> Step {
        let mut lost = None;
        // A rebinding client whose lease ended, or a requester that gave up,
        // starts over in the same call, so the loop runs at most twice.
        for _ in 0..2 {
            match self.deadline {
                Some(d) if now >= d => {}
                _ => break,
            }
            match self.state {
                State::Init => {
                    self.new_exchange(now);
                    self.state = State::Selecting;
                    self.tries = 1;
                    self.deadline = Some(now + self.retransmit_delay(1));
                    return self.step(Some(self.discover(now)), lost);
                }
                State::Selecting => {
                    self.tries = self.tries.saturating_add(1);
                    self.deadline = Some(now + self.retransmit_delay(self.tries));
                    return self.step(Some(self.discover(now)), lost);
                }
                State::Requesting => {
                    let Some(offer) = self.offer else {
                        self.restart(now);
                        continue;
                    };
                    if self.tries >= self.timing.request_tries {
                        self.restart(now);
                        continue;
                    }
                    if self.tries == 0 {
                        self.request_at = now;
                    }
                    self.tries += 1;
                    self.deadline = Some(now + self.retransmit_delay(self.tries));
                    return self.step(Some(self.select_request(&offer, now)), lost);
                }
                State::Bound => {
                    // A renewal is a new exchange, and the lease then runs
                    // from its first REQUEST.
                    self.state = State::Renewing;
                    self.new_exchange(now);
                    return self.extend(now, false, lost);
                }
                State::Renewing => {
                    let rebind = self
                        .lease
                        .as_ref()
                        .and_then(|l| l.rebind_at)
                        .is_some_and(|t| now >= t);
                    if rebind {
                        self.state = State::Rebinding;
                    }
                    return self.extend(now, rebind, lost);
                }
                State::Rebinding => {
                    let expired = self
                        .lease
                        .as_ref()
                        .and_then(|l| l.expires_at)
                        .is_some_and(|t| now >= t);
                    if expired {
                        lost = self.lease.take().map(|l| l.addr);
                        self.restart(now);
                        continue;
                    }
                    return self.extend(now, true, lost);
                }
            }
        }
        Step {
            send: None,
            event: lost.map_or(Event::None, Event::Lost),
        }
    }

    fn step(&self, send: Option<Outgoing>, lost: Option<Ipv4Addr>) -> Step {
        Step {
            send,
            event: lost.map_or(Event::None, Event::Lost),
        }
    }

    /// Sends a renewal or a rebinding message and schedules the next try.
    fn extend(&mut self, now: Duration, rebinding: bool, lost: Option<Ipv4Addr>) -> Step {
        let Some(lease) = self.lease.clone() else {
            self.restart(now);
            return Step::nothing();
        };
        self.tries += 1;
        let limit = if rebinding {
            lease.expires_at
        } else {
            lease.rebind_at
        };
        self.deadline = limit.map(|limit| {
            let left = limit.saturating_sub(now);
            let wait = (left / 2).max(self.timing.min_renew_retry);
            (now + wait).min(limit)
        });
        self.step(Some(self.extend_request(&lease, rebinding, now)), lost)
    }

    /// Drops the offer and the lease and starts a new exchange at `now`.
    fn restart(&mut self, now: Duration) {
        self.state = State::Init;
        self.offer = None;
        self.lease = None;
        self.deadline = Some(now);
    }

    /// Reads one reply that came to the hardware address of the client.
    ///
    /// `eth_src` is the Ethernet source of the frame, which the client keeps
    /// as the address of its server for the unicast renewal. A message for
    /// another client, with another transaction identifier, or of a type that
    /// the state does not expect is ignored.
    pub fn receive(&mut self, now: Duration, msg: &Message, eth_src: MacAddr) -> Step {
        if msg.op != BOOTREPLY || msg.chaddr != self.mac || msg.xid != self.xid {
            return Step::nothing();
        }
        match (self.state, msg.kind) {
            (State::Selecting, MessageType::Offer) => {
                let (Some(server), false) = (msg.options.server_id, msg.yiaddr.is_unspecified())
                else {
                    return Step::nothing();
                };
                self.offer = Some(Offer {
                    addr: msg.yiaddr,
                    server,
                });
                self.state = State::Requesting;
                self.tries = 0;
                self.deadline = Some(now);
                Step::nothing()
            }
            (State::Requesting, MessageType::Ack) => {
                let Some(offer) = self.offer else {
                    return Step::nothing();
                };
                if msg.options.server_id.is_some_and(|s| s != offer.server) {
                    return Step::nothing();
                }
                self.bind(msg, offer.server, eth_src)
            }
            (State::Renewing | State::Rebinding, MessageType::Ack) => {
                let Some(server) = msg
                    .options
                    .server_id
                    .or_else(|| self.lease.as_ref().map(|l| l.server))
                else {
                    return Step::nothing();
                };
                self.bind(msg, server, eth_src)
            }
            (State::Requesting, MessageType::Nak) => {
                self.state = State::Init;
                self.offer = None;
                self.deadline = Some(now + self.timing.nak_delay);
                Step::nothing()
            }
            (State::Renewing | State::Rebinding, MessageType::Nak) => {
                let lost = self.lease.take().map(|l| l.addr);
                self.state = State::Init;
                self.offer = None;
                self.deadline = Some(now + self.timing.nak_delay);
                Step {
                    send: None,
                    event: lost.map_or(Event::None, Event::Lost),
                }
            }
            _ => Step::nothing(),
        }
    }

    fn bind(&mut self, ack: &Message, server: Ipv4Addr, eth_src: MacAddr) -> Step {
        let o = &ack.options;
        let Some(lease_secs) = o.lease_secs.filter(|s| *s > 0) else {
            // An ACK without a lease time breaks RFC 2131, so the client
            // waits for the retransmission.
            return Step::nothing();
        };
        if ack.yiaddr.is_unspecified() {
            return Step::nothing();
        }
        let granted = self.request_at;
        let (renew_at, rebind_at, expires_at) = if lease_secs == u32::MAX {
            (None, None, None)
        } else {
            let l = u64::from(lease_secs);
            let (mut t1, mut t2) = (
                o.renewal_secs.map_or(l / 2, u64::from),
                o.rebinding_secs.map_or(l * 7 / 8, u64::from),
            );
            if !(0 < t1 && t1 < t2 && t2 < l) {
                (t1, t2) = (l / 2, l * 7 / 8);
            }
            // A lease of one second has no room for T1 below T2.
            let at = |s: u64| Some(granted + Duration::from_secs(s));
            (at(t1), at(t2.max(t1)), at(l))
        };
        let lease = Lease {
            addr: ack.yiaddr,
            subnet_mask: o.subnet_mask,
            routers: o.routers.clone(),
            server,
            server_mac: eth_src,
            granted,
            renew_at,
            rebind_at,
            expires_at,
        };
        let previous = self.lease.as_ref().map(|l| l.addr);
        self.state = State::Bound;
        self.offer = None;
        self.tries = 0;
        self.deadline = renew_at;
        let event = if previous == Some(lease.addr) {
            Event::Renewed(lease.clone())
        } else {
            Event::Bound(lease.clone())
        };
        self.lease = Some(lease);
        // An address that changed while the client renewed is a new address
        // for the owner of the table, which replaces the old one by host.
        Step { send: None, event }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: MacAddr = MacAddr([2, 0, 0, 0, 0, 7]);
    const SERVER_MAC: MacAddr = MacAddr([2, 0, 0, 0, 0, 0xfe]);
    const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const ADDR: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 77);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn timing() -> Timing {
        Timing {
            start_delay_max: Duration::ZERO,
            ..Timing::default()
        }
    }

    fn client() -> Client {
        Client::new(7, MAC, timing(), 42, Duration::ZERO)
    }

    fn nothing() -> Step {
        Step {
            send: None,
            event: Event::None,
        }
    }

    fn reply(kind: MessageType, xid: u32, yiaddr: Ipv4Addr, lease: Option<u32>) -> Message {
        let mut m = Message::new(kind, xid, MAC);
        m.yiaddr = yiaddr;
        m.options.server_id = Some(SERVER);
        m.options.lease_secs = lease;
        m.options.subnet_mask = Some(Ipv4Addr::new(255, 255, 255, 0));
        m.options.routers = vec![SERVER];
        m
    }

    /// Runs a client from its start through OFFER and ACK. It returns the time
    /// at which the REQUEST went out, and the lease.
    fn bind(c: &mut Client, lease_secs: u32, tweak: impl Fn(&mut Message)) -> (Duration, Lease) {
        let now = c.deadline().unwrap();
        let d = c.poll(now).send.unwrap();
        assert_eq!(d.msg.kind, MessageType::Discover);
        let mut offer = reply(MessageType::Offer, d.msg.xid, ADDR, Some(lease_secs));
        tweak(&mut offer);
        assert_eq!(c.receive(now, &offer, SERVER_MAC).event, Event::None);
        assert_eq!(c.state(), State::Requesting);
        assert_eq!(c.deadline(), Some(now));
        let r = c.poll(now).send.unwrap();
        assert_eq!(r.msg.kind, MessageType::Request);
        let mut ack = reply(MessageType::Ack, r.msg.xid, ADDR, Some(lease_secs));
        tweak(&mut ack);
        let step = c.receive(now, &ack, SERVER_MAC);
        let Event::Bound(lease) = step.event else {
            panic!("no lease: {step:?}");
        };
        assert_eq!(c.state(), State::Bound);
        (now, lease)
    }

    #[test]
    fn the_first_discover_waits_a_random_time_below_the_maximum() {
        let t = Timing::default();
        let mut starts = Vec::new();
        for index in 0..50 {
            let c = Client::new(index, MAC, t, 1, secs(10));
            let d = c.deadline().unwrap();
            assert!(d >= secs(10) && d < secs(11), "{d:?}");
            starts.push(d);
        }
        starts.sort();
        starts.dedup();
        assert!(starts.len() > 40, "the waits differ between hosts");
        assert_eq!(client().deadline(), Some(Duration::ZERO));
    }

    #[test]
    fn nothing_happens_before_the_deadline() {
        let t = Timing::default();
        let mut c = Client::new(1, MAC, t, 1, Duration::ZERO);
        let d = c.deadline().unwrap();
        assert!(d > Duration::ZERO);
        assert_eq!(c.poll(d - Duration::from_micros(1)), nothing());
        assert_eq!(c.state(), State::Init);
        assert!(c.poll(d).send.is_some());
    }

    #[test]
    fn a_discover_asks_for_a_broadcast_reply_with_the_client_identifier() {
        let mut c = client();
        let out = c.poll(Duration::ZERO).send.unwrap();
        assert_eq!(c.state(), State::Selecting);
        assert_eq!(out.dest, Dest::Broadcast);
        assert_eq!(out.src_ip, Ipv4Addr::UNSPECIFIED);
        let m = &out.msg;
        assert_eq!(m.kind, MessageType::Discover);
        assert_eq!(m.chaddr, MAC);
        assert_eq!(m.xid, c.xid());
        assert!(m.wants_broadcast());
        assert!(m.ciaddr.is_unspecified());
        assert_eq!(m.options.client_id, Some(vec![1, 2, 0, 0, 0, 0, 7]));
        assert_eq!(m.options.param_request, PARAMS);
        assert_eq!(m.op, 1);
    }

    #[test]
    fn a_discover_is_sent_again_with_a_doubling_wait() {
        let mut c = client();
        let mut now = Duration::ZERO;
        let first = c.poll(now).send.unwrap().msg.xid;
        for base in [4u64, 8, 16, 32, 64, 64, 64] {
            let next = c.deadline().unwrap();
            let wait = next - now;
            let (lo, hi) = (ms(base * 750), ms(base * 1250));
            assert!(wait >= lo && wait <= hi, "{wait:?} for base {base}");
            now = next;
            let out = c.poll(now).send.unwrap();
            assert_eq!(out.msg.kind, MessageType::Discover);
            assert_eq!(out.msg.xid, first, "a retransmission keeps the xid");
        }
        assert_eq!(c.state(), State::Selecting);
    }

    #[test]
    fn the_seconds_field_counts_from_the_start_of_the_exchange() {
        let mut c = client();
        c.poll(Duration::ZERO);
        let now = c.deadline().unwrap();
        let out = c.poll(now).send.unwrap();
        assert_eq!(u64::from(out.msg.secs), now.as_secs());
        assert!(out.msg.secs >= 3);
    }

    #[test]
    fn an_offer_leads_to_a_request_for_that_address_from_that_server() {
        let mut c = client();
        let now = c.deadline().unwrap();
        let xid = c.poll(now).send.unwrap().msg.xid;
        let offer = reply(MessageType::Offer, xid, ADDR, Some(3600));
        c.receive(ms(5), &offer, SERVER_MAC);
        let out = c.poll(ms(5)).send.unwrap();
        assert_eq!(out.msg.kind, MessageType::Request);
        assert_eq!(out.msg.xid, xid, "the request continues the exchange");
        assert_eq!(out.msg.options.requested_ip, Some(ADDR));
        assert_eq!(out.msg.options.server_id, Some(SERVER));
        assert!(out.msg.ciaddr.is_unspecified());
        assert!(out.msg.wants_broadcast());
        assert_eq!(out.dest, Dest::Broadcast);
        assert_eq!(out.src_ip, Ipv4Addr::UNSPECIFIED);
    }

    #[test]
    fn an_ack_binds_the_lease_and_sets_t1_and_t2_from_the_lease_time() {
        let mut c = client();
        let (at, lease) = bind(&mut c, 100, |_| {});
        assert_eq!(lease.addr, ADDR);
        assert_eq!(lease.server, SERVER);
        assert_eq!(lease.server_mac, SERVER_MAC);
        assert_eq!(lease.router(), Some(SERVER));
        assert_eq!(lease.prefix_len(), Some(24));
        assert_eq!(lease.granted, at);
        assert_eq!(lease.renew_at, Some(at + secs(50)));
        assert_eq!(lease.rebind_at, Some(at + secs(87)));
        assert_eq!(lease.expires_at, Some(at + secs(100)));
        assert_eq!(c.deadline(), lease.renew_at);
        assert_eq!(c.lease(), Some(&lease));
    }

    #[test]
    fn options_58_and_59_set_t1_and_t2_when_they_are_in_order() {
        let mut c = client();
        let (at, lease) = bind(&mut c, 1000, |m| {
            m.options.renewal_secs = Some(100);
            m.options.rebinding_secs = Some(200);
        });
        assert_eq!(lease.renew_at, Some(at + secs(100)));
        assert_eq!(lease.rebind_at, Some(at + secs(200)));
        for (t1, t2) in [(0, 200), (300, 200), (100, 1000), (100, 2000), (200, 200)] {
            let mut c = client();
            let (at, lease) = bind(&mut c, 1000, |m| {
                m.options.renewal_secs = Some(t1);
                m.options.rebinding_secs = Some(t2);
            });
            assert_eq!(lease.renew_at, Some(at + secs(500)), "T1 {t1} T2 {t2}");
            assert_eq!(lease.rebind_at, Some(at + secs(875)), "T1 {t1} T2 {t2}");
        }
    }

    #[test]
    fn an_infinite_lease_has_no_timer() {
        let mut c = client();
        let (_, lease) = bind(&mut c, u32::MAX, |_| {});
        assert_eq!(lease.renew_at, None);
        assert_eq!(lease.expires_at, None);
        assert_eq!(c.deadline(), None);
        assert_eq!(c.poll(secs(1_000_000)), nothing());
        assert_eq!(c.state(), State::Bound);
    }

    #[test]
    fn a_renewal_is_a_unicast_request_with_the_address_in_ciaddr() {
        let mut c = client();
        let (at, _) = bind(&mut c, 100, |_| {});
        let bound_xid = c.xid();
        assert_eq!(c.poll(at + secs(49)), nothing());
        let t1 = at + secs(50);
        let out = c.poll(t1).send.unwrap();
        assert_eq!(c.state(), State::Renewing);
        assert_eq!(out.msg.kind, MessageType::Request);
        assert_eq!(out.msg.ciaddr, ADDR);
        assert_eq!(out.msg.options.requested_ip, None);
        assert_eq!(out.msg.options.server_id, None);
        assert!(!out.msg.wants_broadcast());
        assert_eq!(
            out.dest,
            Dest::Unicast {
                ip: SERVER,
                mac: SERVER_MAC
            }
        );
        assert_eq!(out.src_ip, ADDR);
        assert_ne!(out.msg.xid, bound_xid, "a renewal is a new exchange");
        // The server answers from another MAC address, which the client keeps.
        let ack = reply(MessageType::Ack, out.msg.xid, ADDR, Some(100));
        let other = MacAddr([2, 0, 0, 0, 0, 0xfd]);
        let Event::Renewed(l) = c.receive(t1, &ack, other).event else {
            panic!("not renewed");
        };
        assert_eq!(l.server_mac, other);
        assert_eq!(c.state(), State::Bound);
        assert_eq!(l.granted, t1, "the lease runs from the renewal request");
        assert_eq!(c.deadline(), Some(t1 + secs(50)));
    }

    #[test]
    fn an_unanswered_renewal_is_retried_then_rebinds_then_the_lease_is_lost() {
        let mut c = client();
        let (at, lease) = bind(&mut c, 1000, |_| {});
        let (t2, end) = (lease.rebind_at.unwrap(), lease.expires_at.unwrap());
        let mut unicasts = 0;
        let mut rebinds = 0;
        let mut now = c.deadline().unwrap();
        let lost = loop {
            let step = c.poll(now);
            if let Event::Lost(a) = step.event {
                break (a, step.send);
            }
            let out = step.send.unwrap();
            assert_eq!(out.msg.kind, MessageType::Request);
            assert_eq!(out.msg.ciaddr, ADDR);
            match out.dest {
                Dest::Unicast { .. } => {
                    assert!(now < t2, "no unicast after T2");
                    assert_eq!(c.state(), State::Renewing);
                    unicasts += 1;
                }
                Dest::Broadcast => {
                    assert!(now >= t2, "no broadcast before T2");
                    assert_eq!(c.state(), State::Rebinding);
                    rebinds += 1;
                }
            }
            let next = c.deadline().unwrap();
            assert!(next > now && next <= end, "{next:?}");
            assert!(next - now >= secs(60) || next == t2 || next == end);
            now = next;
        };
        assert!(unicasts >= 2, "{unicasts}");
        assert!(rebinds >= 2, "{rebinds}");
        assert_eq!(now, at + secs(1000), "the loss is at the end of the lease");
        assert_eq!(lost.0, ADDR);
        let d = lost.1.expect("the client starts over at once");
        assert_eq!(d.msg.kind, MessageType::Discover);
        assert_eq!(d.src_ip, Ipv4Addr::UNSPECIFIED);
        assert_eq!(c.state(), State::Selecting);
        assert!(c.lease().is_none());
    }

    #[test]
    fn a_nak_while_renewing_loses_the_address_and_starts_over_after_the_nak_delay() {
        let mut c = client();
        let (at, _) = bind(&mut c, 100, |_| {});
        let t1 = at + secs(50);
        let xid = c.poll(t1).send.unwrap().msg.xid;
        let nak = Message::new(MessageType::Nak, xid, MAC);
        let step = c.receive(t1, &nak, SERVER_MAC);
        assert_eq!(step.event, Event::Lost(ADDR));
        assert_eq!(c.state(), State::Init);
        assert_eq!(c.deadline(), Some(t1 + secs(1)));
        assert!(c.poll(t1 + ms(999)).send.is_none());
        let d = c.poll(t1 + secs(1)).send.unwrap();
        assert_eq!(d.msg.kind, MessageType::Discover);
    }

    #[test]
    fn a_nak_to_a_request_for_an_offer_starts_over() {
        let mut c = client();
        let now = Duration::ZERO;
        let xid = c.poll(now).send.unwrap().msg.xid;
        let offer = reply(MessageType::Offer, xid, ADDR, Some(60));
        c.receive(now, &offer, SERVER_MAC);
        c.poll(now);
        let nak = Message::new(MessageType::Nak, xid, MAC);
        let step = c.receive(now, &nak, SERVER_MAC);
        assert_eq!(step.event, Event::None, "no lease was held");
        assert_eq!(c.state(), State::Init);
        assert_eq!(c.deadline(), Some(secs(1)));
    }

    #[test]
    fn a_request_without_an_answer_is_sent_request_tries_times_then_the_client_discovers() {
        let mut c = client();
        let mut now = Duration::ZERO;
        let xid = c.poll(now).send.unwrap().msg.xid;
        let offer = reply(MessageType::Offer, xid, ADDR, Some(60));
        c.receive(now, &offer, SERVER_MAC);
        let mut requests = 0;
        let again = loop {
            let out = c.poll(now).send.unwrap();
            match out.msg.kind {
                MessageType::Request => requests += 1,
                MessageType::Discover => break out,
                other => panic!("{other:?}"),
            }
            now = c.deadline().unwrap();
        };
        assert_eq!(requests, 4);
        assert_ne!(again.msg.xid, xid, "a new exchange has a new xid");
        assert_eq!(c.state(), State::Selecting);
    }

    #[test]
    fn a_new_address_in_a_renewal_ack_is_a_new_binding() {
        let mut c = client();
        let (at, _) = bind(&mut c, 100, |_| {});
        let t1 = at + secs(50);
        let xid = c.poll(t1).send.unwrap().msg.xid;
        let new = Ipv4Addr::new(10, 0, 0, 99);
        let ack = reply(MessageType::Ack, xid, new, Some(100));
        let Event::Bound(l) = c.receive(t1, &ack, SERVER_MAC).event else {
            panic!("an address change is not a renewal");
        };
        assert_eq!(l.addr, new);
        assert_eq!(c.lease().unwrap().addr, new);
    }

    #[test]
    fn a_rebinding_ack_from_any_server_binds() {
        let mut c = client();
        let (at, lease) = bind(&mut c, 1000, |_| {});
        let t2 = lease.rebind_at.unwrap();
        let mut now = c.deadline().unwrap();
        let xid = loop {
            let out = c.poll(now).send.unwrap();
            if now >= t2 {
                break out.msg.xid;
            }
            now = c.deadline().unwrap();
        };
        let mut ack = reply(MessageType::Ack, xid, ADDR, Some(1000));
        ack.options.server_id = Some(Ipv4Addr::new(10, 0, 0, 2));
        let Event::Renewed(l) = c.receive(now, &ack, MacAddr([2, 0, 0, 0, 0, 2])).event else {
            panic!("not renewed");
        };
        assert_eq!(l.server, Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(l.granted, at + secs(500), "the exchange began at T1");
        assert_eq!(c.state(), State::Bound);
    }

    #[test]
    fn replies_that_do_not_belong_to_the_exchange_are_ignored() {
        let mut c = client();
        let now = Duration::ZERO;
        let xid = c.poll(now).send.unwrap().msg.xid;
        let good = reply(MessageType::Offer, xid, ADDR, Some(60));
        let mut wrong_xid = good.clone();
        wrong_xid.xid = xid.wrapping_add(1);
        let mut wrong_mac = good.clone();
        wrong_mac.chaddr = MacAddr([2, 0, 0, 0, 0, 8]);
        let mut request_op = good.clone();
        request_op.op = 1;
        let mut no_server = good.clone();
        no_server.options.server_id = None;
        let mut no_address = good.clone();
        no_address.yiaddr = Ipv4Addr::UNSPECIFIED;
        let ack_early = reply(MessageType::Ack, xid, ADDR, Some(60));
        let nak_early = reply(MessageType::Nak, xid, ADDR, None);
        for bad in [
            wrong_xid, wrong_mac, request_op, no_server, no_address, ack_early, nak_early,
        ] {
            assert_eq!(c.receive(now, &bad, SERVER_MAC), nothing(), "{bad:?}");
            assert_eq!(c.state(), State::Selecting);
        }
        c.receive(now, &good, SERVER_MAC);
        assert_eq!(c.state(), State::Requesting);
    }

    #[test]
    fn an_ack_from_another_server_or_without_a_lease_time_is_ignored() {
        let mut c = client();
        let now = Duration::ZERO;
        let xid = c.poll(now).send.unwrap().msg.xid;
        let offer = reply(MessageType::Offer, xid, ADDR, Some(60));
        c.receive(now, &offer, SERVER_MAC);
        c.poll(now);
        let mut other = reply(MessageType::Ack, xid, ADDR, Some(60));
        other.options.server_id = Some(Ipv4Addr::new(10, 0, 0, 9));
        let no_time = reply(MessageType::Ack, xid, ADDR, None);
        let zero = reply(MessageType::Ack, xid, ADDR, Some(0));
        let mut no_addr = reply(MessageType::Ack, xid, ADDR, Some(60));
        no_addr.yiaddr = Ipv4Addr::UNSPECIFIED;
        for bad in [other, no_time, zero, no_addr] {
            assert_eq!(c.receive(now, &bad, SERVER_MAC), nothing(), "{bad:?}");
            assert_eq!(c.state(), State::Requesting);
        }
        let good = reply(MessageType::Ack, xid, ADDR, Some(60));
        assert!(matches!(
            c.receive(now, &good, SERVER_MAC).event,
            Event::Bound(_)
        ));
    }

    #[test]
    fn a_late_ack_after_binding_changes_nothing() {
        let mut c = client();
        let (at, _) = bind(&mut c, 100, |_| {});
        let xid = c.xid();
        let ack = reply(MessageType::Ack, xid, Ipv4Addr::new(10, 0, 0, 5), Some(100));
        assert_eq!(c.receive(at, &ack, SERVER_MAC), nothing());
        assert_eq!(c.lease().unwrap().addr, ADDR);
    }

    #[test]
    fn release_gives_the_lease_back_and_stops_the_client() {
        let mut c = client();
        let (at, _) = bind(&mut c, 100, |_| {});
        let out = c.release(at + secs(5)).unwrap();
        assert_eq!(out.msg.kind, MessageType::Release);
        assert_eq!(out.msg.ciaddr, ADDR);
        assert_eq!(out.msg.options.server_id, Some(SERVER));
        assert_eq!(
            out.dest,
            Dest::Unicast {
                ip: SERVER,
                mac: SERVER_MAC
            }
        );
        assert_eq!(out.src_ip, ADDR);
        assert_eq!(c.deadline(), None);
        assert!(c.lease().is_none());
        assert_eq!(c.poll(secs(10_000)), nothing());
        assert!(c.release(secs(1)).is_none());
    }

    #[test]
    fn the_same_seed_and_index_repeat_and_other_indexes_differ() {
        let run = |index: u32, seed: u64| {
            let mut c = Client::new(index, MAC, Timing::default(), seed, Duration::ZERO);
            let start = c.deadline().unwrap();
            let xid = c.poll(start).send.unwrap().msg.xid;
            (start, xid, c.deadline().unwrap())
        };
        assert_eq!(run(3, 9), run(3, 9));
        assert_ne!(run(3, 9).1, run(4, 9).1);
        assert_ne!(run(3, 9).1, run(3, 10).1);
    }

    #[test]
    fn the_prefix_length_of_a_mask_must_be_contiguous() {
        let mut l = Lease {
            addr: ADDR,
            subnet_mask: None,
            routers: Vec::new(),
            server: SERVER,
            server_mac: SERVER_MAC,
            granted: Duration::ZERO,
            renew_at: None,
            rebind_at: None,
            expires_at: None,
        };
        assert_eq!(l.prefix_len(), None);
        for (mask, len) in [
            ([255, 255, 255, 255], Some(32)),
            ([255, 255, 255, 0], Some(24)),
            ([255, 255, 254, 0], Some(23)),
            ([128, 0, 0, 0], Some(1)),
            ([0, 0, 0, 0], Some(0)),
            ([255, 0, 255, 0], None),
            ([0, 255, 255, 255], None),
        ] {
            l.subnet_mask = Some(Ipv4Addr::from(mask));
            assert_eq!(l.prefix_len(), len, "{mask:?}");
        }
        assert_eq!(l.router(), None);
    }
}
