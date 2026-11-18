// SPDX-License-Identifier: GPL-3.0-only
//! The split of a received frame by Ethernet type and protocol.
//!
//! The classifier reads only the headers that decide where a frame goes: the
//! Ethernet header with its 802.1Q tags, the IPv4 header up to the transport
//! protocol and, for UDP, the two ports, and the IPv6 header up to the next
//! header (TCP, UDP, or ICMPv6 and then its type). It never allocates and never
//! panics on a short frame.

use fps_hosts::wire::{
    ethernet_payload, ETHERTYPE_ARP, ETHERTYPE_IPV4, ETHERTYPE_IPV6, ICMP6_NA, ICMP6_NS,
    NEXT_HEADER_ICMP6,
};

/// IP protocol number of TCP.
const PROTO_TCP: u8 = 6;
/// IP protocol number of UDP.
const PROTO_UDP: u8 = 17;
/// ICMPv6 types of router solicitation and advertisement and of redirect.
const ICMP6_RS: u8 = 133;
const ICMP6_RA: u8 = 134;
const ICMP6_REDIRECT: u8 = 137;
/// The UDP ports of DHCP.
const DHCP_PORTS: [u16; 2] = [67, 68];

/// Where a frame goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// ARP, which the neighbor responder of `fps-hosts` answers.
    Arp,
    /// IPv4 TCP, which the own stack takes.
    Ipv4Tcp,
    /// IPv4 UDP on the DHCP ports, for the DHCP hook.
    Dhcp,
    /// Other IPv4 UDP, for the UDP hook.
    Ipv4Udp,
    /// IPv4 with another protocol, which nothing takes.
    Ipv4Other(u8),
    /// IPv6 neighbor discovery (ICMPv6 types 133 to 137): the responder of
    /// `fps-hosts` answers the solicitations for the host pools, and the ND
    /// hook gets the rest.
    Nd,
    /// IPv6 TCP right after the IPv6 header, which the own stack takes.
    Ipv6Tcp,
    /// IPv6 UDP right after the IPv6 header, for the UDP hook.
    Ipv6Udp,
    /// Other IPv6: another ICMPv6 type, a next header that is not TCP, UDP,
    /// or ICMPv6, and a packet with an extension header in front of its
    /// transport header. Nothing takes it.
    Ipv6Other,
    /// Another Ethernet type.
    Unknown(u16),
    /// A frame too short for its headers, or with more than two VLAN tags.
    Malformed,
}

/// The 802.1Q tagging of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tagging {
    /// No tag.
    Untagged,
    /// One tag with this VLAN identifier.
    Tag(u16),
    /// Two or more tags.
    Stacked,
}

impl Tagging {
    /// Reports whether a port with the VLAN `expected` accepts a frame with
    /// this tagging: an untagged port takes untagged frames, and a tagged port
    /// takes frames with its tag and no other.
    #[must_use]
    pub fn matches(self, expected: Option<u16>) -> bool {
        match (expected, self) {
            (None, Self::Untagged) => true,
            (Some(want), Self::Tag(got)) => want == got,
            _ => false,
        }
    }
}

/// The class and the tagging of one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Where the frame goes.
    pub class: Class,
    /// How the frame is tagged.
    pub tagging: Tagging,
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let pair = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*pair.first()?, *pair.get(1)?]))
}

/// Classifies one Ethernet frame.
#[must_use]
pub fn classify(frame: &[u8]) -> Frame {
    let Some((pos, ethertype, vlans, tags)) = ethernet_payload(frame) else {
        // `ethernet_payload` refuses a short header and a third tag.
        let tagging = if frame.len() >= 22 {
            Tagging::Stacked
        } else {
            Tagging::Untagged
        };
        return Frame {
            class: Class::Malformed,
            tagging,
        };
    };
    let tagging = match tags {
        0 => Tagging::Untagged,
        1 => Tagging::Tag(vlans.first().copied().unwrap_or(0)),
        _ => Tagging::Stacked,
    };
    let class = match ethertype {
        ETHERTYPE_ARP => Class::Arp,
        ETHERTYPE_IPV4 => ipv4(frame, pos),
        ETHERTYPE_IPV6 => ipv6(frame, pos),
        other => Class::Unknown(other),
    };
    Frame { class, tagging }
}

fn ipv4(frame: &[u8], pos: usize) -> Class {
    let (Some(&first), Some(&protocol)) = (frame.get(pos), frame.get(pos + 9)) else {
        return Class::Malformed;
    };
    let header = usize::from(first & 0x0f) * 4;
    if first >> 4 != 4 || header < 20 {
        return Class::Malformed;
    }
    match protocol {
        PROTO_TCP => Class::Ipv4Tcp,
        PROTO_UDP => {
            // Only the first fragment carries the ports.
            let first_fragment = be16(frame, pos + 6).is_some_and(|f| f & 0x1fff == 0);
            let Some(dst) = first_fragment
                .then(|| be16(frame, pos + header + 2))
                .flatten()
            else {
                return Class::Ipv4Udp;
            };
            let src = be16(frame, pos + header);
            if DHCP_PORTS.contains(&dst) && src.is_some_and(|s| DHCP_PORTS.contains(&s)) {
                Class::Dhcp
            } else {
                Class::Ipv4Udp
            }
        }
        other => Class::Ipv4Other(other),
    }
}

fn ipv6(frame: &[u8], pos: usize) -> Class {
    let Some(&next) = frame.get(pos + 6) else {
        return Class::Malformed;
    };
    if frame.get(pos).is_some_and(|first| first >> 4 != 6) {
        return Class::Malformed;
    }
    match next {
        PROTO_TCP => Class::Ipv6Tcp,
        PROTO_UDP => Class::Ipv6Udp,
        NEXT_HEADER_ICMP6 => match frame.get(pos + 40) {
            Some(&(ICMP6_RS | ICMP6_RA | ICMP6_NS | ICMP6_NA | ICMP6_REDIRECT)) => Class::Nd,
            _ => Class::Ipv6Other,
        },
        _ => Class::Ipv6Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eth(ethertype: u16, tag: Option<u16>, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 12];
        if let Some(id) = tag {
            f.extend_from_slice(&0x8100u16.to_be_bytes());
            f.extend_from_slice(&id.to_be_bytes());
        }
        f.extend_from_slice(&ethertype.to_be_bytes());
        f.extend_from_slice(payload);
        f
    }

    fn ip4(protocol: u8, l4: &[u8]) -> Vec<u8> {
        let mut p = vec![
            0x45, 0, 0, 0, 0, 0, 0, 0, 64, protocol, 0, 0, 10, 0, 0, 1, 10, 0, 1, 1,
        ];
        p.extend_from_slice(l4);
        p
    }

    fn udp(src: u16, dst: u16) -> Vec<u8> {
        let mut u = src.to_be_bytes().to_vec();
        u.extend_from_slice(&dst.to_be_bytes());
        u.extend_from_slice(&[0, 8, 0, 0]);
        u
    }

    #[test]
    fn splits_by_ethertype_and_protocol() {
        let class = |f: &[u8]| classify(f).class;
        assert_eq!(class(&eth(0x0806, None, &[0; 28])), Class::Arp);
        assert_eq!(class(&eth(0x0800, None, &ip4(6, &[0; 20]))), Class::Ipv4Tcp);
        assert_eq!(
            class(&eth(0x0800, None, &ip4(17, &udp(5000, 53)))),
            Class::Ipv4Udp
        );
        assert_eq!(
            class(&eth(0x0800, None, &ip4(17, &udp(68, 67)))),
            Class::Dhcp
        );
        assert_eq!(
            class(&eth(0x0800, None, &ip4(1, &[0; 8]))),
            Class::Ipv4Other(1)
        );
        assert_eq!(class(&eth(0x88b5, None, &[0; 20])), Class::Unknown(0x88b5));
        let mut nd = vec![0x60, 0, 0, 0, 0, 0, 58, 255];
        nd.resize(40, 0);
        nd.push(135);
        assert_eq!(class(&eth(0x86dd, None, &nd)), Class::Nd);
        nd[6] = 17;
        assert_eq!(class(&eth(0x86dd, None, &nd)), Class::Ipv6Udp);
        nd[6] = 6;
        assert_eq!(class(&eth(0x86dd, None, &nd)), Class::Ipv6Tcp);
        // An echo request, a hop by hop header, and a fragment header go
        // nowhere.
        nd[6] = 58;
        nd[40] = 128;
        assert_eq!(class(&eth(0x86dd, None, &nd)), Class::Ipv6Other);
        for next in [0, 43, 44, 59] {
            nd[6] = next;
            assert_eq!(class(&eth(0x86dd, None, &nd)), Class::Ipv6Other, "{next}");
        }
        // Every neighbor discovery type is ND.
        nd[6] = 58;
        for kind in 133..=137 {
            nd[40] = kind;
            assert_eq!(class(&eth(0x86dd, None, &nd)), Class::Nd, "{kind}");
        }
        // Version 4 in an IPv6 frame is malformed.
        nd[0] = 0x40;
        assert_eq!(class(&eth(0x86dd, None, &nd)), Class::Malformed);
    }

    #[test]
    fn tags_are_read_and_matched() {
        let tcp = ip4(6, &[0; 20]);
        let tagged = classify(&eth(0x0800, Some(100), &tcp));
        assert_eq!(tagged.class, Class::Ipv4Tcp);
        assert_eq!(tagged.tagging, Tagging::Tag(100));
        assert!(tagged.tagging.matches(Some(100)));
        assert!(!tagged.tagging.matches(Some(101)));
        assert!(!tagged.tagging.matches(None));
        assert!(Tagging::Untagged.matches(None));
        assert!(!Tagging::Untagged.matches(Some(100)));
        assert!(!Tagging::Stacked.matches(Some(100)));
    }

    #[test]
    fn short_and_broken_frames_are_malformed() {
        assert_eq!(classify(&[]).class, Class::Malformed);
        assert_eq!(classify(&[0; 13]).class, Class::Malformed);
        assert_eq!(
            classify(&eth(0x0800, None, &[0x45; 4])).class,
            Class::Malformed
        );
        // Version 6 in an IPv4 frame.
        assert_eq!(
            classify(&eth(0x0800, None, &[0x65; 20])).class,
            Class::Malformed
        );
    }

    #[test]
    fn no_prefix_of_a_frame_panics() {
        let full = eth(0x0800, Some(7), &ip4(17, &udp(68, 67)));
        for n in 0..=full.len() {
            let _ = classify(&full[..n]);
        }
    }
}
