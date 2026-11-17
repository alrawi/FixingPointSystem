// SPDX-License-Identifier: GPL-3.0-only
//! Ethernet, IPv4, and UDP framing for DHCP.
//!
//! A user space client owns the frames of its port, so it writes the headers
//! itself. [`udp4_frame`] builds a whole frame with both checksums, and
//! [`parse_udp4`] reads one back, checks the IPv4 header checksum, and refuses
//! fragments. It does not check the UDP checksum, because a frame that a host
//! with checksum offload sent, such as one that crossed a veth pair, carries
//! only the partial checksum of the pseudo header, and a receiver that dropped
//! it would lose every reply of such a server. Neither call is on the hot
//! path.

use std::net::Ipv4Addr;

use crate::pool::MacAddr;
use crate::wire::{ethernet_payload, ETHERTYPE_IPV4, MIN_FRAME_LEN, VLAN_TPIDS};

const PROTO_UDP: u8 = 17;
const IPV4_HEADER: usize = 20;
const UDP_HEADER: usize = 8;

/// A UDP datagram of an IPv4 packet in an Ethernet frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Udp4<'a> {
    /// The Ethernet destination.
    pub eth_dst: MacAddr,
    /// The Ethernet source.
    pub eth_src: MacAddr,
    /// The IPv4 source.
    pub ip_src: Ipv4Addr,
    /// The IPv4 destination.
    pub ip_dst: Ipv4Addr,
    /// The UDP source port.
    pub src_port: u16,
    /// The UDP destination port.
    pub dst_port: u16,
    /// The UDP payload.
    pub payload: &'a [u8],
}

fn sum_words(bytes: &[u8], mut sum: u32) -> u32 {
    let (pairs, rest) = bytes.as_chunks::<2>();
    for p in pairs {
        sum += u32::from(u16::from_be_bytes(*p));
    }
    if let Some(last) = rest.first() {
        sum += u32::from(*last) << 8;
    }
    sum
}

fn fold(mut sum: u32) -> u16 {
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    // The loop leaves 16 bits, so the cast keeps every bit.
    !(sum as u16)
}

/// The internet checksum of an IPv4 header, with the checksum field taken as
/// it stands. A header whose checksum field is right sums to 0.
#[must_use]
pub fn ipv4_checksum(header: &[u8]) -> u16 {
    fold(sum_words(header, 0))
}

/// The UDP checksum of `segment`, the UDP header and payload with the
/// checksum field taken as it stands, between `src` and `dst`. A segment whose
/// checksum field is right sums to 0.
#[must_use]
pub fn udp4_checksum(src: Ipv4Addr, dst: Ipv4Addr, segment: &[u8]) -> u16 {
    let mut sum = sum_words(&src.octets(), 0);
    sum = sum_words(&dst.octets(), sum);
    sum += u32::from(PROTO_UDP);
    // A segment is far below 64 KiB.
    sum += segment.len() as u32;
    fold(sum_words(segment, sum))
}

/// Builds an Ethernet frame with an IPv4 header, a UDP header, and `payload`.
/// `vlans` lists the VLAN ids of the tags, outermost first. A frame shorter
/// than 60 bytes is padded with zeros.
#[must_use]
#[allow(clippy::too_many_arguments)] // reason: the fields of the three headers
pub fn udp4_frame(
    eth_dst: MacAddr,
    eth_src: MacAddr,
    vlans: &[u16],
    ip_src: Ipv4Addr,
    ip_dst: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut f = Vec::with_capacity(14 + 4 * vlans.len() + IPV4_HEADER + UDP_HEADER + payload.len());
    f.extend_from_slice(&eth_dst.0);
    f.extend_from_slice(&eth_src.0);
    for vid in vlans {
        f.extend_from_slice(&VLAN_TPIDS[0].to_be_bytes());
        f.extend_from_slice(&(vid & 0x0fff).to_be_bytes());
    }
    f.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
    let ip_at = f.len();
    let udp_len = UDP_HEADER + payload.len();
    // A DHCP message is far below 64 KiB, so the lengths fit 16 bits.
    let total = (IPV4_HEADER + udp_len) as u16;
    f.extend_from_slice(&[0x45, 0]);
    f.extend_from_slice(&total.to_be_bytes());
    f.extend_from_slice(&[0, 0, 0, 0, 64, PROTO_UDP, 0, 0]);
    f.extend_from_slice(&ip_src.octets());
    f.extend_from_slice(&ip_dst.octets());
    let csum = ipv4_checksum(&f[ip_at..ip_at + IPV4_HEADER]);
    f[ip_at + 10..ip_at + 12].copy_from_slice(&csum.to_be_bytes());
    let udp_at = f.len();
    f.extend_from_slice(&src_port.to_be_bytes());
    f.extend_from_slice(&dst_port.to_be_bytes());
    f.extend_from_slice(&(udp_len as u16).to_be_bytes());
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(payload);
    let mut csum = udp4_checksum(ip_src, ip_dst, &f[udp_at..]);
    if csum == 0 {
        // A computed zero goes out as all ones, because zero means no checksum.
        csum = 0xffff;
    }
    f[udp_at + 6..udp_at + 8].copy_from_slice(&csum.to_be_bytes());
    if f.len() < MIN_FRAME_LEN {
        f.resize(MIN_FRAME_LEN, 0);
    }
    f
}

fn mac_at(b: &[u8], at: usize) -> Option<MacAddr> {
    Some(MacAddr(b.get(at..at + 6)?.try_into().ok()?))
}

fn ip_at(b: &[u8], at: usize) -> Option<Ipv4Addr> {
    Some(Ipv4Addr::from(
        <[u8; 4]>::try_from(b.get(at..at + 4)?).ok()?,
    ))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// Parses an Ethernet frame that carries an unfragmented UDP datagram over
/// IPv4, or returns `None` for anything else and for a bad IPv4 header
/// checksum. The UDP checksum is not checked, see the module documentation.
#[must_use]
pub fn parse_udp4(frame: &[u8]) -> Option<Udp4<'_>> {
    let (pos, ethertype, _, _) = ethernet_payload(frame)?;
    if ethertype != ETHERTYPE_IPV4 {
        return None;
    }
    let ip = frame.get(pos..)?;
    let first = *ip.first()?;
    if first >> 4 != 4 {
        return None;
    }
    let ihl = usize::from(first & 0x0f) * 4;
    if ihl < IPV4_HEADER {
        return None;
    }
    let header = ip.get(..ihl)?;
    if ipv4_checksum(header) != 0 {
        return None;
    }
    let total = usize::from(be16(ip, 2)?);
    // More Fragments, or a fragment offset: a DHCP message is never split.
    if be16(ip, 6)? & 0x3fff != 0 || *ip.get(9)? != PROTO_UDP {
        return None;
    }
    // The frame may carry padding after the packet.
    let packet = ip.get(..total)?;
    let udp = packet.get(ihl..)?;
    let udp_len = usize::from(be16(udp, 4)?);
    if udp_len < UDP_HEADER {
        return None;
    }
    let segment = udp.get(..udp_len)?;
    let ip_src = ip_at(ip, 12)?;
    let ip_dst = ip_at(ip, 16)?;
    Some(Udp4 {
        eth_dst: mac_at(frame, 0)?,
        eth_src: mac_at(frame, 6)?,
        ip_src,
        ip_dst,
        src_port: be16(segment, 0)?,
        dst_port: be16(segment, 2)?,
        payload: segment.get(UDP_HEADER..)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: MacAddr = MacAddr([2, 0, 0, 0, 0, 1]);
    const B: MacAddr = MacAddr([2, 0, 0, 0, 0, 2]);

    fn frame(payload: &[u8]) -> Vec<u8> {
        udp4_frame(
            B,
            A,
            &[],
            Ipv4Addr::new(10, 0, 0, 2),
            Ipv4Addr::new(10, 0, 0, 1),
            68,
            67,
            payload,
        )
    }

    #[test]
    fn a_frame_parses_back_and_both_checksums_hold() {
        let f = frame(b"hello world");
        let u = parse_udp4(&f).unwrap();
        assert_eq!(u.eth_dst, B);
        assert_eq!(u.eth_src, A);
        assert_eq!(u.ip_src, Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(u.ip_dst, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!((u.src_port, u.dst_port), (68, 67));
        assert_eq!(u.payload, b"hello world");
        assert_eq!(ipv4_checksum(&f[14..34]), 0);
        assert_eq!(
            udp4_checksum(u.ip_src, u.ip_dst, &f[34..34 + 8 + 11]),
            0,
            "the checksum field makes the segment sum to zero"
        );
    }

    #[test]
    fn the_ipv4_header_checksum_matches_a_known_header() {
        // The header of the example in RFC 1071 style references: 45 00 00 73
        // 00 00 40 00 40 11 b8 61 c0 a8 00 01 c0 a8 00 c7.
        let h = [
            0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0xb8, 0x61, 0xc0, 0xa8,
            0x00, 0x01, 0xc0, 0xa8, 0x00, 0xc7,
        ];
        assert_eq!(ipv4_checksum(&h), 0);
    }

    #[test]
    fn a_short_frame_is_padded_and_the_padding_is_ignored() {
        let f = frame(b"x");
        assert_eq!(f.len(), 60);
        assert_eq!(parse_udp4(&f).unwrap().payload, b"x");
    }

    #[test]
    fn vlan_tags_are_written_and_read() {
        let f = udp4_frame(
            B,
            A,
            &[100, 200],
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::BROADCAST,
            68,
            67,
            b"abc",
        );
        assert_eq!(f[12..14], [0x81, 0x00]);
        assert_eq!(f[14..16], [0x00, 100]);
        assert_eq!(f[16..18], [0x81, 0x00]);
        assert_eq!(f[18..20], [0x00, 200]);
        assert_eq!(f[20..22], [0x08, 0x00]);
        assert_eq!(parse_udp4(&f).unwrap().payload, b"abc");
    }

    #[test]
    fn a_damaged_frame_is_refused() {
        let good = frame(b"payload!");
        // A flipped bit in the IPv4 header, which the header checksum
        // catches, and in the protocol field.
        for at in [22usize, 23] {
            let mut f = good.clone();
            f[at] ^= 0x01;
            assert!(parse_udp4(&f).is_none(), "byte {at}");
        }
        // The UDP checksum is not checked, so a frame with the partial
        // checksum of an offloading sender, or with none, still parses.
        for csum in [[0, 0], [0x12, 0x34]] {
            let mut f = good.clone();
            f[40..42].copy_from_slice(&csum);
            assert_eq!(parse_udp4(&f).unwrap().payload, b"payload!");
        }
        // Not IPv4, truncated, fragmented, and a UDP length that is too long.
        let mut f = good.clone();
        f[12..14].copy_from_slice(&[0x08, 0x06]);
        assert!(parse_udp4(&f).is_none());
        assert!(parse_udp4(&good[..30]).is_none());
        assert!(parse_udp4(&good[..13]).is_none());
        let mut f = good.clone();
        f[20] = 0x20; // more fragments
        let c = ipv4_checksum(&{
            let mut h = f[14..34].to_vec();
            h[10] = 0;
            h[11] = 0;
            h
        });
        f[24..26].copy_from_slice(&c.to_be_bytes());
        assert!(parse_udp4(&f).is_none());
        let mut f = good;
        f[38..40].copy_from_slice(&200u16.to_be_bytes());
        assert!(parse_udp4(&f).is_none());
    }

    proptest::proptest! {
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..120)) {
            let _ = parse_udp4(&bytes);
        }

        #[test]
        fn a_built_frame_always_parses_back(payload in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
            let f = frame(&payload);
            proptest::prop_assert_eq!(parse_udp4(&f).unwrap().payload, &payload[..]);
        }
    }
}
