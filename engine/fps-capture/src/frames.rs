// SPDX-License-Identifier: GPL-3.0-only
//! Frame synthesis for socket level session bytes.
//!
//! The engine sees the bytes that a session reads and writes, and no packets.
//! This module rebuilds Ethernet frames around those bytes with a TCP
//! handshake, sequence and acknowledgment numbers, segmentation at the MSS,
//! and a FIN or RST close, or one UDP datagram per send and receive. The
//! frames are valid for tshark and carry correct IPv4, TCP, and UDP checksums.
//! Everything is a pure function of its inputs, so the same events always give
//! the same bytes.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::sampler::splitmix64;
use crate::CaptureError;

/// The transport of a captured session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// A TCP byte stream.
    Tcp,
    /// UDP datagrams.
    Udp,
}

/// Which end of the session the tap wraps, so which direction the writes take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The tap wraps the client. Its writes go to the server.
    Client,
    /// The tap wraps the server. Its writes go to the client.
    Server,
}

/// The two endpoints of a session. Both are of one address family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowEndpoints {
    client: SocketAddr,
    server: SocketAddr,
}

impl FlowEndpoints {
    /// Pairs a client and a server address, which must share an address family.
    pub fn new(client: SocketAddr, server: SocketAddr) -> Result<Self, CaptureError> {
        if client.is_ipv4() != server.is_ipv4() {
            return Err(CaptureError::InvalidConfig(format!(
                "endpoints {client} and {server} mix IPv4 and IPv6"
            )));
        }
        Ok(Self { client, server })
    }

    /// Returns the client address.
    pub fn client(&self) -> SocketAddr {
        self.client
    }

    /// Returns the server address.
    pub fn server(&self) -> SocketAddr {
        self.server
    }
}

/// What the tap observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventKind {
    /// Bytes or one datagram that the tapped end wrote.
    Out(Vec<u8>),
    /// Bytes or one datagram that the tapped end read.
    In(Vec<u8>),
    /// The tapped end closed its write side, which sends FIN.
    LocalClose,
    /// The tapped end read end of stream, so the peer sent FIN.
    PeerClose,
    /// The tapped end aborted, which sends RST.
    LocalReset,
    /// The tapped end read a connection reset, so the peer sent RST.
    PeerReset,
}

/// One observation with its time in nanoseconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
    pub nanos: u64,
    pub kind: EventKind,
}

/// One synthesized Ethernet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub nanos: u64,
    pub data: Vec<u8>,
}

/// Everything the synthesis needs besides the events.
pub(crate) struct Params {
    pub flow: FlowEndpoints,
    pub side: Side,
    pub mtu: usize,
    pub seed: u64,
    pub session_id: u64,
    pub open_nanos: u64,
    pub end_nanos: u64,
}

const TCP_FIN: u8 = 0x01;
const TCP_SYN: u8 = 0x02;
const TCP_RST: u8 = 0x04;
const TCP_PSH: u8 = 0x08;
const TCP_ACK: u8 = 0x10;
/// Gap between consecutive synthesized frames of one session, at least.
const MIN_GAP_NANOS: u64 = 1_000;
/// Gap between the three handshake frames.
const HANDSHAKE_GAP_NANOS: u64 = 20_000;

fn checksum(parts: &[&[u8]]) -> u16 {
    let mut sum: u32 = 0;
    let mut odd: Option<u8> = None;
    for byte in parts.iter().flat_map(|p| p.iter()) {
        match odd.take() {
            Some(high) => sum += u32::from(u16::from_be_bytes([high, *byte])),
            None => odd = Some(*byte),
        }
    }
    if let Some(high) = odd {
        sum += u32::from(u16::from_be_bytes([high, 0]));
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn mac(ip: IpAddr) -> [u8; 6] {
    // Locally administered, unicast, and stable for one address.
    let tail = match ip {
        IpAddr::V4(a) => a.octets(),
        IpAddr::V6(a) => {
            let o = a.octets();
            [o[12], o[13], o[14], o[15]]
        }
    };
    [0x02, 0x00, tail[0], tail[1], tail[2], tail[3]]
}

/// Builds frames of one session in one direction at a time.
struct Builder<'a> {
    p: &'a Params,
    frames: Vec<Frame>,
    last_nanos: u64,
    ident: u16,
}

impl<'a> Builder<'a> {
    fn new(p: &'a Params) -> Self {
        Self {
            p,
            frames: Vec::new(),
            last_nanos: 0,
            ident: (splitmix64(p.seed ^ p.session_id) & 0xffff) as u16,
        }
    }

    /// Returns a timestamp that is at or after `wanted` and later than the previous frame.
    fn stamp(&mut self, wanted: u64) -> u64 {
        let t = if self.frames.is_empty() {
            wanted
        } else {
            wanted.max(self.last_nanos + MIN_GAP_NANOS)
        };
        self.last_nanos = t;
        t
    }

    fn mss(&self) -> usize {
        let ip = if self.p.flow.client.is_ipv4() { 20 } else { 40 };
        self.p.mtu.saturating_sub(ip + 20).max(64)
    }

    /// Builds and stores one frame. `l4` has its checksum field zeroed.
    fn push(&mut self, wanted: u64, from_client: bool, proto: u8, mut l4: Vec<u8>, df: bool) {
        let (src, dst) = if from_client {
            (self.p.flow.client.ip(), self.p.flow.server.ip())
        } else {
            (self.p.flow.server.ip(), self.p.flow.client.ip())
        };
        let csum_at = if proto == 6 { 16 } else { 6 };
        let l4_len = u32::try_from(l4.len()).unwrap_or(u32::MAX);
        let mut ip = Vec::with_capacity(40 + l4.len());
        let ethertype: [u8; 2];
        match (src, dst) {
            (IpAddr::V4(s), IpAddr::V4(d)) => {
                ethertype = [0x08, 0x00];
                let pseudo = pseudo_v4(s, d, proto, l4_len);
                let mut sum = checksum(&[&pseudo, &l4]);
                if proto == 17 && sum == 0 {
                    sum = 0xffff;
                }
                l4[csum_at..csum_at + 2].copy_from_slice(&sum.to_be_bytes());
                self.ident = self.ident.wrapping_add(1);
                let total = u16::try_from(20 + l4.len()).unwrap_or(u16::MAX);
                ip.extend_from_slice(&[0x45, 0]);
                ip.extend_from_slice(&total.to_be_bytes());
                ip.extend_from_slice(&self.ident.to_be_bytes());
                ip.extend_from_slice(&[if df { 0x40 } else { 0 }, 0, 64, proto, 0, 0]);
                ip.extend_from_slice(&s.octets());
                ip.extend_from_slice(&d.octets());
                let sum = checksum(&[&ip]);
                ip[10..12].copy_from_slice(&sum.to_be_bytes());
            }
            (IpAddr::V6(s), IpAddr::V6(d)) => {
                ethertype = [0x86, 0xdd];
                let pseudo = pseudo_v6(s, d, proto, l4_len);
                let mut sum = checksum(&[&pseudo, &l4]);
                if proto == 17 && sum == 0 {
                    sum = 0xffff;
                }
                l4[csum_at..csum_at + 2].copy_from_slice(&sum.to_be_bytes());
                ip.extend_from_slice(&[0x60, 0, 0, 0]);
                ip.extend_from_slice(&u16::try_from(l4.len()).unwrap_or(u16::MAX).to_be_bytes());
                ip.extend_from_slice(&[proto, 64]);
                ip.extend_from_slice(&s.octets());
                ip.extend_from_slice(&d.octets());
            }
            // FlowEndpoints::new rejects mixed families.
            _ => return,
        }
        let mut data = Vec::with_capacity(14 + ip.len() + l4.len());
        data.extend_from_slice(&mac(dst));
        data.extend_from_slice(&mac(src));
        data.extend_from_slice(&ethertype);
        data.extend_from_slice(&ip);
        data.extend_from_slice(&l4);
        let nanos = self.stamp(wanted);
        self.frames.push(Frame { nanos, data });
    }

    fn tcp(
        &mut self,
        wanted: u64,
        from_client: bool,
        (seq, ack, flags): (u32, u32, u8),
        payload: &[u8],
    ) {
        let (sport, dport) = if from_client {
            (self.p.flow.client.port(), self.p.flow.server.port())
        } else {
            (self.p.flow.server.port(), self.p.flow.client.port())
        };
        let syn = flags & TCP_SYN != 0;
        let mut l4 = Vec::with_capacity(24 + payload.len());
        l4.extend_from_slice(&sport.to_be_bytes());
        l4.extend_from_slice(&dport.to_be_bytes());
        l4.extend_from_slice(&seq.to_be_bytes());
        l4.extend_from_slice(&ack.to_be_bytes());
        l4.extend_from_slice(&[if syn { 0x60 } else { 0x50 }, flags]);
        l4.extend_from_slice(&65_535u16.to_be_bytes());
        l4.extend_from_slice(&[0, 0, 0, 0]);
        if syn {
            // The MSS option, kind 2 and length 4.
            let mss = u16::try_from(self.mss()).unwrap_or(1460);
            l4.extend_from_slice(&[2, 4]);
            l4.extend_from_slice(&mss.to_be_bytes());
        }
        l4.extend_from_slice(payload);
        self.push(wanted, from_client, 6, l4, true);
    }

    fn udp(&mut self, wanted: u64, from_client: bool, payload: &[u8]) {
        let (sport, dport) = if from_client {
            (self.p.flow.client.port(), self.p.flow.server.port())
        } else {
            (self.p.flow.server.port(), self.p.flow.client.port())
        };
        let mut l4 = Vec::with_capacity(8 + payload.len());
        l4.extend_from_slice(&sport.to_be_bytes());
        l4.extend_from_slice(&dport.to_be_bytes());
        l4.extend_from_slice(
            &u16::try_from(8 + payload.len())
                .unwrap_or(u16::MAX)
                .to_be_bytes(),
        );
        l4.extend_from_slice(&[0, 0]);
        l4.extend_from_slice(payload);
        self.push(wanted, from_client, 17, l4, false);
    }
}

fn pseudo_v4(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, len: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    p.extend_from_slice(&[0, proto]);
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p
}

fn pseudo_v6(src: Ipv6Addr, dst: Ipv6Addr, proto: u8, len: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(40);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    p.extend_from_slice(&len.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, proto]);
    p
}

/// True when the event bytes travel from the client to the server.
fn to_server(side: Side, out: bool) -> bool {
    (side == Side::Client) == out
}

/// State of one TCP direction.
struct Dir {
    seq: u32,
    fin: bool,
}

/// Synthesizes the frames of one TCP session.
fn tcp_session(p: &Params, events: &[Event]) -> Vec<Frame> {
    let mut b = Builder::new(p);
    let mss = b.mss();
    // Initial sequence numbers come from the seed and the session id, so a
    // replay reproduces them. The two sides use different streams.
    let base = splitmix64(p.seed ^ splitmix64(p.session_id));
    let mut c = Dir {
        seq: base as u32,
        fin: false,
    };
    let mut s = Dir {
        seq: (base >> 32) as u32 ^ 0x5a5a_5a5a,
        fin: false,
    };

    let t0 = p.open_nanos;
    b.tcp(t0, true, (c.seq, 0, TCP_SYN), &[]);
    c.seq = c.seq.wrapping_add(1);
    b.tcp(
        t0 + HANDSHAKE_GAP_NANOS,
        false,
        (s.seq, c.seq, TCP_SYN | TCP_ACK),
        &[],
    );
    s.seq = s.seq.wrapping_add(1);
    b.tcp(
        t0 + 2 * HANDSHAKE_GAP_NANOS,
        true,
        (c.seq, s.seq, TCP_ACK),
        &[],
    );

    // Sends FIN from one side and the acknowledgment of it from the other.
    fn fin(b: &mut Builder<'_>, at: u64, from_client: bool, c: &mut Dir, s: &mut Dir) {
        let (me, peer) = if from_client {
            (&mut *c, &mut *s)
        } else {
            (&mut *s, &mut *c)
        };
        if me.fin {
            return;
        }
        me.fin = true;
        b.tcp(at, from_client, (me.seq, peer.seq, TCP_FIN | TCP_ACK), &[]);
        me.seq = me.seq.wrapping_add(1);
        b.tcp(at, !from_client, (peer.seq, me.seq, TCP_ACK), &[]);
    }

    let mut reset = false;
    for ev in events {
        match &ev.kind {
            EventKind::Out(bytes) | EventKind::In(bytes) => {
                let from_client = to_server(p.side, matches!(ev.kind, EventKind::Out(_)));
                let (me, peer) = if from_client {
                    (&mut c, &mut s)
                } else {
                    (&mut s, &mut c)
                };
                if me.fin {
                    continue;
                }
                let chunks = bytes.chunks(mss).collect::<Vec<_>>();
                let last = chunks.len().saturating_sub(1);
                for (i, chunk) in chunks.iter().enumerate() {
                    let flags = TCP_ACK | if i == last { TCP_PSH } else { 0 };
                    b.tcp(ev.nanos, from_client, (me.seq, peer.seq, flags), chunk);
                    me.seq = me.seq.wrapping_add(chunk.len() as u32);
                }
            }
            EventKind::LocalClose => fin(&mut b, ev.nanos, p.side == Side::Client, &mut c, &mut s),
            EventKind::PeerClose => fin(&mut b, ev.nanos, p.side != Side::Client, &mut c, &mut s),
            EventKind::LocalReset | EventKind::PeerReset => {
                let from_client =
                    (p.side == Side::Client) == matches!(ev.kind, EventKind::LocalReset);
                let (me, peer) = if from_client { (&c, &s) } else { (&s, &c) };
                b.tcp(
                    ev.nanos,
                    from_client,
                    (me.seq, peer.seq, TCP_RST | TCP_ACK),
                    &[],
                );
                reset = true;
                break;
            }
        }
    }
    if !reset {
        // A session that ended without a close behaves like a close, so both
        // sides finish, the tapped end first.
        let local_is_client = p.side == Side::Client;
        fin(&mut b, p.end_nanos, local_is_client, &mut c, &mut s);
        fin(&mut b, p.end_nanos, !local_is_client, &mut c, &mut s);
    }
    b.frames
}

/// Synthesizes the frames of one UDP session, one frame per datagram.
fn udp_session(p: &Params, events: &[Event]) -> Vec<Frame> {
    let mut b = Builder::new(p);
    for ev in events {
        match &ev.kind {
            EventKind::Out(bytes) => b.udp(ev.nanos, to_server(p.side, true), bytes),
            EventKind::In(bytes) => b.udp(ev.nanos, to_server(p.side, false), bytes),
            _ => {}
        }
    }
    b.frames
}

/// Synthesizes the frames of one session in time order.
pub(crate) fn synthesize(p: &Params, transport: Transport, events: &[Event]) -> Vec<Frame> {
    match transport {
        Transport::Tcp => tcp_session(p, events),
        Transport::Udp => udp_session(p, events),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(v6: bool) -> FlowEndpoints {
        if v6 {
            FlowEndpoints::new(
                "[2001:db8::1]:40000".parse().unwrap(),
                "[2001:db8::2]:443".parse().unwrap(),
            )
            .unwrap()
        } else {
            FlowEndpoints::new(
                "10.0.0.1:40000".parse().unwrap(),
                "10.0.0.2:443".parse().unwrap(),
            )
            .unwrap()
        }
    }

    fn params(v6: bool, side: Side) -> Params {
        Params {
            flow: flow(v6),
            side,
            mtu: 1500,
            seed: 1,
            session_id: 9,
            open_nanos: 1_000_000,
            end_nanos: 9_000_000,
        }
    }

    fn ev(nanos: u64, kind: EventKind) -> Event {
        Event { nanos, kind }
    }

    /// Returns the TCP flags and the payload length of a frame, for either family.
    fn tcp_of(f: &Frame) -> (u8, usize) {
        let ip_len = if f.data[12..14] == [0x08, 0x00] {
            20
        } else {
            40
        };
        let tcp = &f.data[14 + ip_len..];
        let off = usize::from(tcp[12] >> 4) * 4;
        (tcp[13], tcp.len() - off)
    }

    #[test]
    fn mixed_families_are_rejected() {
        assert!(
            FlowEndpoints::new("10.0.0.1:1".parse().unwrap(), "[::1]:2".parse().unwrap()).is_err()
        );
    }

    #[test]
    fn handshake_data_and_close_in_order() {
        for v6 in [false, true] {
            let evs = [
                ev(2_000_000, EventKind::Out(vec![7; 3000])),
                ev(3_000_000, EventKind::In(b"ok".to_vec())),
                ev(4_000_000, EventKind::LocalClose),
                ev(5_000_000, EventKind::PeerClose),
            ];
            let frames = synthesize(&params(v6, Side::Client), Transport::Tcp, &evs);
            let flags: Vec<(u8, usize)> = frames.iter().map(tcp_of).collect();
            assert_eq!(flags, expected_v(v6));
            assert!(frames.windows(2).all(|w| w[0].nanos < w[1].nanos));
        }
    }

    fn expected_v(v6: bool) -> Vec<(u8, usize)> {
        let mss = if v6 { 1440 } else { 1460 };
        vec![
            (TCP_SYN, 0),
            (TCP_SYN | TCP_ACK, 0),
            (TCP_ACK, 0),
            (TCP_ACK, mss),
            (TCP_ACK, mss),
            (TCP_ACK | TCP_PSH, 3000 - 2 * mss),
            (TCP_ACK | TCP_PSH, 2),
            (TCP_FIN | TCP_ACK, 0),
            (TCP_ACK, 0),
            (TCP_FIN | TCP_ACK, 0),
            (TCP_ACK, 0),
        ]
    }

    #[test]
    fn reset_ends_the_stream() {
        let evs = [
            ev(2_000_000, EventKind::Out(vec![1; 10])),
            ev(3_000_000, EventKind::LocalReset),
        ];
        let frames = synthesize(&params(false, Side::Server), Transport::Tcp, &evs);
        let last = tcp_of(frames.last().unwrap());
        assert_eq!(last.0, TCP_RST | TCP_ACK);
        // The server side reset comes from the server port.
        let f = frames.last().unwrap();
        assert_eq!(u16::from_be_bytes([f.data[34], f.data[35]]), 443);
    }

    #[test]
    fn a_session_without_close_events_still_closes() {
        let evs = [ev(2_000_000, EventKind::Out(vec![1; 10]))];
        let frames = synthesize(&params(false, Side::Client), Transport::Tcp, &evs);
        let flags: Vec<u8> = frames.iter().map(|f| tcp_of(f).0).collect();
        assert_eq!(
            &flags[flags.len() - 4..],
            &[TCP_FIN | TCP_ACK, TCP_ACK, TCP_FIN | TCP_ACK, TCP_ACK]
        );
    }

    #[test]
    fn checksums_verify_for_both_families() {
        for v6 in [false, true] {
            let evs = [ev(2_000_000, EventKind::Out(b"odd".to_vec()))];
            for f in synthesize(&params(v6, Side::Client), Transport::Tcp, &evs) {
                let (ip, l4) = if v6 {
                    (&f.data[14..54], &f.data[54..])
                } else {
                    (&f.data[14..34], &f.data[34..])
                };
                let pseudo = if v6 {
                    let mut src = [0u8; 16];
                    let mut dst = [0u8; 16];
                    src.copy_from_slice(&ip[8..24]);
                    dst.copy_from_slice(&ip[24..40]);
                    pseudo_v6(Ipv6Addr::from(src), Ipv6Addr::from(dst), 6, l4.len() as u32)
                } else {
                    let s = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
                    let d = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
                    assert_eq!(checksum(&[ip]), 0, "IPv4 header checksum");
                    pseudo_v4(s, d, 6, l4.len() as u32)
                };
                assert_eq!(checksum(&[&pseudo, l4]), 0, "TCP checksum");
            }
        }
    }

    #[test]
    fn udp_keeps_datagram_boundaries_and_directions() {
        let evs = [
            ev(2_000_000, EventKind::Out(vec![1; 40])),
            ev(3_000_000, EventKind::In(Vec::new())),
            ev(4_000_000, EventKind::LocalClose),
        ];
        let frames = synthesize(&params(false, Side::Client), Transport::Udp, &evs);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data.len(), 14 + 20 + 8 + 40);
        assert_eq!(frames[1].data.len(), 14 + 20 + 8);
        assert_eq!(
            u16::from_be_bytes([frames[0].data[36], frames[0].data[37]]),
            443
        );
        assert_eq!(
            u16::from_be_bytes([frames[1].data[36], frames[1].data[37]]),
            40000
        );
    }

    #[test]
    fn the_same_inputs_give_the_same_bytes() {
        let evs = [ev(2_000_000, EventKind::Out(vec![3; 100]))];
        let a = synthesize(&params(false, Side::Client), Transport::Tcp, &evs);
        let b = synthesize(&params(false, Side::Client), Transport::Tcp, &evs);
        assert_eq!(a, b);
    }
}
