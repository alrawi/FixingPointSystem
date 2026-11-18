// SPDX-License-Identifier: GPL-3.0-only
//! Ethernet framing, 802.1Q VLAN tags, and neighbor lookup for the hosts of
//! one side of the stack.
//!
//! A [`Station`] is the link layer of one [`crate::host::Host`]. It knows
//!
//! * the hosts that the side serves, as a [`NeighborTable`] of `fps-hosts`
//!   (host `i` of a pool has the MAC address `mac_base + i`, plan rule 5),
//! * the neighbors that the side talks to, as a second table, and
//! * the VLAN of the port, if any.
//!
//! The stack does not send ARP requests of its own, because the neighbors of a
//! test configuration are the hosts of another pool whose addresses the
//! configuration fixes, so the lookup is a table read. The station does
//! answer ARP requests for the hosts that it serves, through the
//! [`Responder`] of `fps-hosts`, so a switch or a real peer can resolve them.
//! IPv6 neighbor solicitations for the hosts that the station serves are
//! answered the same way, and the stack sends no solicitations of its own.

use std::cell::Cell;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub use fps_hosts::wire::{ETHERTYPE_ARP, ETHERTYPE_IPV4, ETHERTYPE_IPV6, MAX_REPLY_LEN};
use fps_hosts::{MacAddr, NeighborTable, Responder, Verdict};

/// The length of an Ethernet header without a tag.
pub const ETH_LEN: usize = 14;
/// The length of an 802.1Q tag.
pub const VLAN_TAG_LEN: usize = 4;
/// The Ethernet type of a customer VLAN tag.
pub const TPID_8021Q: u16 = 0x8100;
/// The shortest frame on the wire without the frame check sequence. The
/// station pads shorter frames with zeros, as a NIC would.
pub const MIN_FRAME_LEN: usize = 60;
/// The largest VLAN identifier of a tagged frame. 0 and 4095 are reserved.
pub const MAX_VLAN_ID: u16 = 4094;

/// An address that a [`Station`] looks up: an IPv4 address as the `u32` that
/// `Ipv4Addr::from` reads (the form that the stack used before IPv6), an
/// `Ipv4Addr`, an `Ipv6Addr`, or an `IpAddr`.
pub trait IntoIp: Copy {
    /// The address as an `IpAddr`.
    fn into_ip(self) -> IpAddr;
}

impl IntoIp for IpAddr {
    fn into_ip(self) -> IpAddr {
        self
    }
}

impl IntoIp for u32 {
    fn into_ip(self) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(self))
    }
}

impl IntoIp for Ipv4Addr {
    fn into_ip(self) -> IpAddr {
        IpAddr::V4(self)
    }
}

impl IntoIp for Ipv6Addr {
    fn into_ip(self) -> IpAddr {
        IpAddr::V6(self)
    }
}

/// The offset of the next header field in an IPv6 header.
const IPV6_NEXT_HEADER_AT: usize = 6;
/// The next header value of ICMPv6, which carries neighbor discovery.
const NEXT_HEADER_ICMP6: u8 = 58;

/// Why a frame or a station setting was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// The frame is shorter than its Ethernet header and tag claim.
    #[error("the frame is too short for its Ethernet header")]
    Short,
    /// The frame carries a tag type other than 802.1Q, or more than one tag.
    #[error("the frame has a VLAN tag that the stack does not take")]
    UnsupportedTag,
    /// The VLAN of the frame is not the VLAN of the port.
    #[error("the frame is for another VLAN")]
    OtherVlan,
    /// The frame has no tag where the port has a VLAN, or a tag where it has
    /// none.
    #[error("the VLAN tag of the frame does not match the port")]
    TagMismatch,
    /// The VLAN identifier is outside 1 to 4094.
    #[error("the VLAN identifier {0} is outside 1 to 4094")]
    BadVlan(u16),
}

/// The fields of a decoded Ethernet header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EthHeader {
    /// The destination MAC address.
    pub dst: MacAddr,
    /// The source MAC address.
    pub src: MacAddr,
    /// The VLAN identifier of the 802.1Q tag, if the frame has one.
    pub vlan: Option<u16>,
    /// The priority code point of the tag, 0 for an untagged frame.
    pub pcp: u8,
    /// The Ethernet type of the payload.
    pub ethertype: u16,
    /// The offset of the payload in the frame.
    pub payload_at: usize,
}

/// Writes an Ethernet header, with an 802.1Q tag when `vlan` is `Some`, to the
/// end of `out`.
pub fn encode_header(
    out: &mut Vec<u8>,
    dst: MacAddr,
    src: MacAddr,
    vlan: Option<u16>,
    ethertype: u16,
) {
    out.extend_from_slice(&dst.0);
    out.extend_from_slice(&src.0);
    if let Some(id) = vlan {
        out.extend_from_slice(&TPID_8021Q.to_be_bytes());
        out.extend_from_slice(&(id & 0x0FFF).to_be_bytes());
    }
    out.extend_from_slice(&ethertype.to_be_bytes());
}

/// Decodes the Ethernet header of `frame`, with at most one 802.1Q tag.
pub fn decode_header(frame: &[u8]) -> Result<EthHeader, NetError> {
    let head = frame.get(..ETH_LEN).ok_or(NetError::Short)?;
    let mac = |at: usize| {
        let mut b = [0u8; 6];
        b.copy_from_slice(&head[at..at + 6]);
        MacAddr(b)
    };
    let (dst, src) = (mac(0), mac(6));
    let first = u16::from_be_bytes([head[12], head[13]]);
    if first != TPID_8021Q {
        if fps_hosts::wire::VLAN_TPIDS.contains(&first) {
            return Err(NetError::UnsupportedTag);
        }
        return Ok(EthHeader {
            dst,
            src,
            vlan: None,
            pcp: 0,
            ethertype: first,
            payload_at: ETH_LEN,
        });
    }
    let tag = frame
        .get(ETH_LEN..ETH_LEN + VLAN_TAG_LEN)
        .ok_or(NetError::Short)?;
    let tci = u16::from_be_bytes([tag[0], tag[1]]);
    let ethertype = u16::from_be_bytes([tag[2], tag[3]]);
    if fps_hosts::wire::VLAN_TPIDS.contains(&ethertype) {
        return Err(NetError::UnsupportedTag);
    }
    Ok(EthHeader {
        dst,
        src,
        vlan: Some(tci & 0x0FFF),
        pcp: (tci >> 13) as u8,
        ethertype,
        payload_at: ETH_LEN + VLAN_TAG_LEN,
    })
}

/// The result of [`Station::receive`].
#[derive(Debug, PartialEq, Eq)]
pub enum Received<'a> {
    /// An IPv4 or IPv6 packet. The slice starts at the IP header, and the
    /// caller checks the destination address.
    Ip(&'a [u8]),
    /// An ARP request or neighbor solicitation for a host of the station. The
    /// reply of this length is in the buffer that the caller passed.
    Reply(usize),
    /// A frame that the station ignores, for example ARP for another host.
    Ignored,
}

/// The link layer of one side of the stack.
#[derive(Debug, Clone)]
pub struct Station {
    vlan: Option<u16>,
    local: Responder,
    peers: NeighborTable,
    /// The last local host and the last neighbor that were found. The frames
    /// of one connection repeat the same two addresses, so the table, which
    /// walks its slices, is read about once per connection and not once per
    /// frame.
    last_local: Cell<Option<(IpAddr, MacAddr)>>,
    last_peer: Cell<Option<(IpAddr, MacAddr)>>,
}

/// The MAC address that `table` holds for `ip`.
fn lookup(table: &NeighborTable, ip: IpAddr) -> Option<MacAddr> {
    match ip {
        IpAddr::V4(a) => table.lookup_v4(a),
        IpAddr::V6(a) => table.lookup_v6(a),
    }
}

impl Station {
    /// Returns a station that serves the hosts of `local`, resolves the
    /// neighbors in `peers`, and tags its frames with `vlan`.
    pub fn new(
        local: NeighborTable,
        peers: NeighborTable,
        vlan: Option<u16>,
    ) -> Result<Self, NetError> {
        if let Some(id) = vlan {
            if id == 0 || id > MAX_VLAN_ID {
                return Err(NetError::BadVlan(id));
            }
        }
        Ok(Self {
            vlan,
            local: Responder::new(local),
            peers,
            last_local: Cell::new(None),
            last_peer: Cell::new(None),
        })
    }

    /// The VLAN of the port.
    pub fn vlan(&self) -> Option<u16> {
        self.vlan
    }

    /// The number of hosts that the station serves.
    pub fn hosts(&self) -> u64 {
        self.local.table().hosts()
    }

    /// Reports whether `ip` belongs to a host of the station.
    pub fn owns(&self, ip: impl IntoIp) -> bool {
        self.local_mac(ip).is_some()
    }

    /// The MAC address of the local host `ip`.
    pub fn local_mac(&self, ip: impl IntoIp) -> Option<MacAddr> {
        let ip = ip.into_ip();
        if let Some((cached, mac)) = self.last_local.get() {
            if cached == ip {
                return Some(mac);
            }
        }
        let mac = lookup(self.local.table(), ip)?;
        self.last_local.set(Some((ip, mac)));
        Some(mac)
    }

    /// The MAC address of the neighbor `ip`.
    pub fn resolve(&self, ip: impl IntoIp) -> Option<MacAddr> {
        let ip = ip.into_ip();
        if let Some((cached, mac)) = self.last_peer.get() {
            if cached == ip {
                return Some(mac);
            }
        }
        let mac = lookup(&self.peers, ip)?;
        self.last_peer.set(Some((ip, mac)));
        Some(mac)
    }

    /// Writes the Ethernet header for a packet from the local host `src_ip`
    /// to the neighbor `dst_ip` and returns `true`, or returns `false` and
    /// writes nothing when either address has no MAC address. The Ethernet
    /// type follows the family of the addresses, and a pair of mixed families
    /// has no header.
    pub fn head(&self, out: &mut Vec<u8>, src_ip: impl IntoIp, dst_ip: impl IntoIp) -> bool {
        let (src_ip, dst_ip) = (src_ip.into_ip(), dst_ip.into_ip());
        if src_ip.is_ipv4() != dst_ip.is_ipv4() {
            return false;
        }
        let (Some(src), Some(dst)) = (self.local_mac(src_ip), self.resolve(dst_ip)) else {
            return false;
        };
        let ethertype = if src_ip.is_ipv4() {
            ETHERTYPE_IPV4
        } else {
            ETHERTYPE_IPV6
        };
        encode_header(out, dst, src, self.vlan, ethertype);
        true
    }

    /// Takes one frame from the link. IPv4 and IPv6 packets come back as
    /// [`Received::Ip`], ARP requests and neighbor solicitations for a host
    /// are answered into `reply`, and everything else is an error or
    /// [`Received::Ignored`].
    pub fn receive<'a>(&self, frame: &'a [u8], reply: &mut [u8]) -> Result<Received<'a>, NetError> {
        let head = decode_header(frame)?;
        match (self.vlan, head.vlan) {
            (None, None) => {}
            (Some(mine), Some(theirs)) if mine == theirs => {}
            (Some(_), Some(_)) => return Err(NetError::OtherVlan),
            _ => return Err(NetError::TagMismatch),
        }
        match head.ethertype {
            ETHERTYPE_IPV4 => Ok(Received::Ip(&frame[head.payload_at..])),
            ETHERTYPE_IPV6 => {
                let packet = &frame[head.payload_at..];
                if packet.get(IPV6_NEXT_HEADER_AT) != Some(&NEXT_HEADER_ICMP6) {
                    return Ok(Received::Ip(packet));
                }
                match self.local.process(frame, reply) {
                    Verdict::Reply(_, len) => Ok(Received::Reply(len)),
                    _ => Ok(Received::Ignored),
                }
            }
            ETHERTYPE_ARP => match self.local.process(frame, reply) {
                Verdict::Reply(_, len) => Ok(Received::Reply(len)),
                _ => Ok(Received::Ignored),
            },
            _ => Ok(Received::Ignored),
        }
    }
}

#[cfg(test)]
#[path = "net_tests.rs"]
mod tests;
