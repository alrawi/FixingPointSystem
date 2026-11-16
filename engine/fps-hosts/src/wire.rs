// SPDX-License-Identifier: GPL-3.0-only
//! Wire formats of ARP and IPv6 neighbor discovery.
//!
//! The builders make whole Ethernet frames for the active side, a host that
//! resolves a gateway, and for tests. They allocate, so they stay off the hot
//! path. The parsers read the same frames back and never panic on short or
//! odd input. [`crate::Responder`] answers on the hot path with its own
//! allocation free code and uses only the constants and the checksum of this
//! module.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::pool::MacAddr;

/// The Ethernet type of IPv4.
pub const ETHERTYPE_IPV4: u16 = 0x0800;
/// The Ethernet type of ARP.
pub const ETHERTYPE_ARP: u16 = 0x0806;
/// The Ethernet type of IPv6.
pub const ETHERTYPE_IPV6: u16 = 0x86DD;
/// The Ethernet types of a VLAN tag: 802.1Q, 802.1ad, and the legacy QinQ type.
pub const VLAN_TPIDS: [u16; 3] = [0x8100, 0x88A8, 0x9100];
/// The shortest Ethernet frame without the frame check sequence. Shorter
/// frames get zero padding.
pub const MIN_FRAME_LEN: usize = 60;
/// The ARP operation of a request.
pub const ARP_REQUEST: u16 = 1;
/// The ARP operation of a reply.
pub const ARP_REPLY: u16 = 2;
/// The ICMPv6 type of a neighbor solicitation.
pub const ICMP6_NS: u8 = 135;
/// The ICMPv6 type of a neighbor advertisement.
pub const ICMP6_NA: u8 = 136;
/// The IPv6 next header value of ICMPv6.
pub const NEXT_HEADER_ICMP6: u8 = 58;
/// The neighbor discovery option that carries the sender link layer address.
pub const OPT_SOURCE_LLA: u8 = 1;
/// The neighbor discovery option that carries the target link layer address.
pub const OPT_TARGET_LLA: u8 = 2;
/// The flag of a neighbor advertisement that answers a solicitation.
pub const NA_SOLICITED: u8 = 0x40;
/// The flag of a neighbor advertisement that overrides a cached address.
pub const NA_OVERRIDE: u8 = 0x20;
/// The most VLAN tags that the responder looks through.
pub const MAX_VLAN_TAGS: usize = 2;
/// The size of the longest frame that [`crate::Responder`] writes: an Ethernet
/// header with two VLAN tags, an IPv6 header, and a neighbor advertisement
/// with one option.
pub const MAX_REPLY_LEN: usize = 14 + 4 * MAX_VLAN_TAGS + 40 + 32;

/// The Ethernet broadcast address.
pub const BROADCAST_MAC: MacAddr = MacAddr([0xff; 6]);

/// The all nodes link local multicast address `ff02::1`.
pub const ALL_NODES: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

/// The Ethernet multicast address of an IPv6 multicast address.
#[must_use]
pub fn multicast_mac(ip: Ipv6Addr) -> MacAddr {
    let o = ip.octets();
    MacAddr([0x33, 0x33, o[12], o[13], o[14], o[15]])
}

/// The solicited node multicast address of an IPv6 address, which every
/// neighbor solicitation for it goes to.
#[must_use]
pub fn solicited_node(ip: Ipv6Addr) -> Ipv6Addr {
    let o = ip.octets();
    Ipv6Addr::from([
        0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xff, o[13], o[14], o[15],
    ])
}

/// The ICMPv6 checksum of `message` between `src` and `dst`, with the
/// checksum field taken as it stands in `message`. A message whose checksum
/// field is right sums to 0.
#[must_use]
pub fn icmp6_checksum(src: Ipv6Addr, dst: Ipv6Addr, message: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut add = |bytes: &[u8]| {
        let mut chunks = bytes.chunks_exact(2);
        for c in &mut chunks {
            sum += u32::from(u16::from_be_bytes([c[0], c[1]]));
        }
        if let [last] = chunks.remainder() {
            sum += u32::from(u16::from_be_bytes([*last, 0]));
        }
    };
    add(&src.octets());
    add(&dst.octets());
    add(&(message.len() as u32).to_be_bytes());
    add(&[0, 0, 0, NEXT_HEADER_ICMP6]);
    add(message);
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    // The fold above leaves 16 bits, so the cast keeps every bit.
    !(sum as u16)
}

fn ethernet(dst: MacAddr, src: MacAddr, vlans: &[u16], ethertype: u16) -> Vec<u8> {
    let mut f = Vec::with_capacity(MIN_FRAME_LEN + 8);
    f.extend_from_slice(&dst.0);
    f.extend_from_slice(&src.0);
    for vid in vlans {
        f.extend_from_slice(&VLAN_TPIDS[0].to_be_bytes());
        f.extend_from_slice(&(vid & 0x0fff).to_be_bytes());
    }
    f.extend_from_slice(&ethertype.to_be_bytes());
    f
}

fn pad(mut frame: Vec<u8>) -> Vec<u8> {
    if frame.len() < MIN_FRAME_LEN {
        frame.resize(MIN_FRAME_LEN, 0);
    }
    frame
}

/// Builds an ARP frame. `vlans` lists the VLAN ids of the tags, outermost
/// first.
#[must_use]
#[allow(clippy::too_many_arguments)] // reason: the fields of the ARP wire format
pub fn arp_frame(
    op: u16,
    eth_dst: MacAddr,
    eth_src: MacAddr,
    vlans: &[u16],
    sender_mac: MacAddr,
    sender_ip: Ipv4Addr,
    target_mac: MacAddr,
    target_ip: Ipv4Addr,
) -> Vec<u8> {
    let mut f = ethernet(eth_dst, eth_src, vlans, ETHERTYPE_ARP);
    f.extend_from_slice(&1u16.to_be_bytes());
    f.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
    f.extend_from_slice(&[6, 4]);
    f.extend_from_slice(&op.to_be_bytes());
    f.extend_from_slice(&sender_mac.0);
    f.extend_from_slice(&sender_ip.octets());
    f.extend_from_slice(&target_mac.0);
    f.extend_from_slice(&target_ip.octets());
    pad(f)
}

/// Builds the broadcast ARP request of a host that asks who has `target_ip`.
#[must_use]
pub fn arp_request(
    src_mac: MacAddr,
    src_ip: Ipv4Addr,
    target_ip: Ipv4Addr,
    vlans: &[u16],
) -> Vec<u8> {
    arp_frame(
        ARP_REQUEST,
        BROADCAST_MAC,
        src_mac,
        vlans,
        src_mac,
        src_ip,
        MacAddr([0; 6]),
        target_ip,
    )
}

/// Builds a neighbor solicitation for `target`. A solicitation from an
/// unspecified source address is a duplicate address detection probe and has
/// no source link layer option.
#[must_use]
pub fn ns_frame(src_mac: MacAddr, src_ip: Ipv6Addr, target: Ipv6Addr, vlans: &[u16]) -> Vec<u8> {
    let dst = solicited_node(target);
    let with_option = !src_ip.is_unspecified();
    let icmp_len = 24 + if with_option { 8 } else { 0 };
    let mut f = ethernet(multicast_mac(dst), src_mac, vlans, ETHERTYPE_IPV6);
    let mut icmp = Vec::with_capacity(icmp_len);
    icmp.extend_from_slice(&[ICMP6_NS, 0, 0, 0, 0, 0, 0, 0]);
    icmp.extend_from_slice(&target.octets());
    if with_option {
        icmp.extend_from_slice(&[OPT_SOURCE_LLA, 1]);
        icmp.extend_from_slice(&src_mac.0);
    }
    let csum = icmp6_checksum(src_ip, dst, &icmp);
    icmp[2..4].copy_from_slice(&csum.to_be_bytes());
    ipv6_header(&mut f, icmp.len(), src_ip, dst);
    f.extend_from_slice(&icmp);
    pad(f)
}

fn ipv6_header(f: &mut Vec<u8>, payload_len: usize, src: Ipv6Addr, dst: Ipv6Addr) {
    f.extend_from_slice(&[0x60, 0, 0, 0]);
    // The payloads of this module are far below 64 KiB.
    f.extend_from_slice(&(payload_len as u16).to_be_bytes());
    f.extend_from_slice(&[NEXT_HEADER_ICMP6, 255]);
    f.extend_from_slice(&src.octets());
    f.extend_from_slice(&dst.octets());
}

/// A parsed ARP packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArpPacket {
    /// The Ethernet source address of the frame.
    pub eth_src: MacAddr,
    /// The Ethernet destination address of the frame.
    pub eth_dst: MacAddr,
    /// The VLAN ids of the tags, outermost first, with unused slots 0.
    pub vlans: [u16; MAX_VLAN_TAGS],
    /// How many VLAN tags the frame has.
    pub vlan_tags: usize,
    /// The operation, [`ARP_REQUEST`] or [`ARP_REPLY`].
    pub op: u16,
    /// The sender hardware address.
    pub sender_mac: MacAddr,
    /// The sender protocol address.
    pub sender_ip: Ipv4Addr,
    /// The target hardware address.
    pub target_mac: MacAddr,
    /// The target protocol address.
    pub target_ip: Ipv4Addr,
}

fn mac_at(b: &[u8], at: usize) -> Option<MacAddr> {
    let s: [u8; 6] = b.get(at..at + 6)?.try_into().ok()?;
    Some(MacAddr(s))
}

fn v6_at(b: &[u8], at: usize) -> Option<Ipv6Addr> {
    let s: [u8; 16] = b.get(at..at + 16)?.try_into().ok()?;
    Some(Ipv6Addr::from(s))
}

fn v4_at(b: &[u8], at: usize) -> Option<Ipv4Addr> {
    let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(Ipv4Addr::from(s))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// Looks through at most [`MAX_VLAN_TAGS`] tags and returns the position of
/// the payload, the Ethernet type of the payload, and the tags.
#[must_use]
pub fn ethernet_payload(frame: &[u8]) -> Option<(usize, u16, [u16; MAX_VLAN_TAGS], usize)> {
    let mut pos = 12;
    let mut ethertype = be16(frame, pos)?;
    pos += 2;
    let mut vlans = [0u16; MAX_VLAN_TAGS];
    let mut tags = 0;
    while VLAN_TPIDS.contains(&ethertype) {
        if tags == MAX_VLAN_TAGS {
            return None;
        }
        *vlans.get_mut(tags)? = be16(frame, pos)? & 0x0fff;
        ethertype = be16(frame, pos + 2)?;
        pos += 4;
        tags += 1;
    }
    Some((pos, ethertype, vlans, tags))
}

/// Parses an Ethernet and IPv4 ARP frame, or returns `None` for anything
/// else.
#[must_use]
pub fn parse_arp(frame: &[u8]) -> Option<ArpPacket> {
    let (pos, ethertype, vlans, vlan_tags) = ethernet_payload(frame)?;
    if ethertype != ETHERTYPE_ARP {
        return None;
    }
    let b = frame.get(pos..pos + 28)?;
    if be16(b, 0)? != 1 || be16(b, 2)? != ETHERTYPE_IPV4 || b.get(4..6)? != [6, 4] {
        return None;
    }
    Some(ArpPacket {
        eth_dst: mac_at(frame, 0)?,
        eth_src: mac_at(frame, 6)?,
        vlans,
        vlan_tags,
        op: be16(b, 6)?,
        sender_mac: mac_at(b, 8)?,
        sender_ip: v4_at(b, 14)?,
        target_mac: mac_at(b, 18)?,
        target_ip: v4_at(b, 24)?,
    })
}

/// A parsed neighbor advertisement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeighborAdvert {
    /// The Ethernet source address of the frame.
    pub eth_src: MacAddr,
    /// The Ethernet destination address of the frame.
    pub eth_dst: MacAddr,
    /// The IPv6 source address.
    pub ip_src: Ipv6Addr,
    /// The IPv6 destination address.
    pub ip_dst: Ipv6Addr,
    /// The hop limit, which must be 255.
    pub hop_limit: u8,
    /// Whether the ICMPv6 checksum is right.
    pub checksum_ok: bool,
    /// The flag byte: [`NA_SOLICITED`] and [`NA_OVERRIDE`] and the router bit.
    pub flags: u8,
    /// The address that the advertisement is about.
    pub target: Ipv6Addr,
    /// The address of the target link layer option, if the frame has one.
    pub target_lla: Option<MacAddr>,
}

/// Parses an Ethernet frame that carries an IPv6 neighbor advertisement, or
/// returns `None` for anything else.
#[must_use]
pub fn parse_neighbor_advert(frame: &[u8]) -> Option<NeighborAdvert> {
    let (pos, ethertype, _, _) = ethernet_payload(frame)?;
    if ethertype != ETHERTYPE_IPV6 {
        return None;
    }
    let ip = frame.get(pos..)?;
    if ip.first()? >> 4 != 6 || *ip.get(6)? != NEXT_HEADER_ICMP6 {
        return None;
    }
    let len = usize::from(be16(ip, 4)?);
    let icmp = ip.get(40..40 + len)?;
    if *icmp.first()? != ICMP6_NA || len < 24 {
        return None;
    }
    let (ip_src, ip_dst) = (v6_at(ip, 8)?, v6_at(ip, 24)?);
    let mut target_lla = None;
    let mut i = 24;
    while i + 2 <= len {
        let size = usize::from(*icmp.get(i + 1)?) * 8;
        if size == 0 || i + size > len {
            return None;
        }
        if *icmp.get(i)? == OPT_TARGET_LLA && size == 8 {
            target_lla = mac_at(icmp, i + 2);
        }
        i += size;
    }
    Some(NeighborAdvert {
        eth_dst: mac_at(frame, 0)?,
        eth_src: mac_at(frame, 6)?,
        ip_src,
        ip_dst,
        hop_limit: *ip.get(7)?,
        checksum_ok: icmp6_checksum(ip_src, ip_dst, icmp) == 0,
        flags: *icmp.get(4)?,
        target: v6_at(icmp, 8)?,
        target_lla,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solicited_node_follows_rfc_4291() {
        let a: Ipv6Addr = "2001:db8::1:2:3456".parse().unwrap();
        assert_eq!(
            solicited_node(a),
            "ff02::1:ff02:3456".parse::<Ipv6Addr>().unwrap()
        );
        assert_eq!(
            multicast_mac(solicited_node(a)).0,
            [0x33, 0x33, 0xff, 0x02, 0x34, 0x56]
        );
    }

    #[test]
    fn checksum_of_a_known_message() {
        // The echo request of RFC 4443 style examples is not published with a
        // checksum, so check the property: a message with its own checksum
        // sums to zero, and flipping a bit breaks it.
        let src: Ipv6Addr = "fe80::1".parse().unwrap();
        let dst: Ipv6Addr = "fe80::2".parse().unwrap();
        let mut m = vec![128, 0, 0, 0, 0x12, 0x34, 0, 1, 1, 2, 3];
        let c = icmp6_checksum(src, dst, &m);
        m[2..4].copy_from_slice(&c.to_be_bytes());
        assert_eq!(icmp6_checksum(src, dst, &m), 0);
        m[10] ^= 1;
        assert_ne!(icmp6_checksum(src, dst, &m), 0);
    }

    #[test]
    fn arp_request_round_trips() {
        let mac = MacAddr([2, 0, 0, 0, 0, 9]);
        let f = arp_request(
            mac,
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 0, 0, 7),
            &[100],
        );
        assert_eq!(f.len(), MIN_FRAME_LEN);
        let p = parse_arp(&f).unwrap();
        assert_eq!(p.op, ARP_REQUEST);
        assert_eq!(p.vlans, [100, 0]);
        assert_eq!(p.vlan_tags, 1);
        assert_eq!(p.target_ip, Ipv4Addr::new(10, 0, 0, 7));
        assert_eq!(p.eth_dst, BROADCAST_MAC);
    }

    #[test]
    fn parsers_refuse_short_and_foreign_input() {
        assert!(parse_arp(&[]).is_none());
        assert!(parse_arp(&[0; 13]).is_none());
        assert!(parse_neighbor_advert(&[0; 60]).is_none());
        let mut f = arp_request(
            MacAddr([2; 6]),
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::LOCALHOST,
            &[],
        );
        f.truncate(30);
        assert!(parse_arp(&f).is_none());
        // Three VLAN tags are more than the parsers follow.
        let mut deep = vec![0u8; 12];
        for _ in 0..3 {
            deep.extend_from_slice(&[0x81, 0x00, 0, 1]);
        }
        deep.extend_from_slice(&[0x08, 0x06]);
        deep.resize(80, 0);
        assert!(ethernet_payload(&deep).is_none());
    }
}
