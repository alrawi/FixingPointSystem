// SPDX-License-Identifier: GPL-3.0-only
//! ARP and neighbor discovery responders for the hosts of a port.
//!
//! A user space backend owns the frames of its port, so no kernel answers
//! "who has 10.0.7.3" for the hosts of a pool. [`NeighborTable`] knows which
//! addresses belong to which host and which MAC address each host has, and
//! [`Responder`] turns a received ARP request or IPv6 neighbor solicitation
//! for one of those addresses into the reply that the host would send. Every
//! other frame goes back to the caller untouched.
//!
//! The responder follows these rules.
//!
//! * It answers an ARP request whose target is a host address with an ARP
//!   reply from the host MAC address to the sender hardware address. A
//!   request with the same sender and target address is an announcement and
//!   gets no answer. A probe with sender address 0.0.0.0 gets an answer, as
//!   the defense of RFC 5227 says.
//! * It answers a neighbor solicitation for a host address with a neighbor
//!   advertisement with the solicited and override flags and the target link
//!   layer option. A solicitation from the unspecified address is duplicate
//!   address detection and gets an advertisement to `ff02::1` without the
//!   solicited flag. The solicitation must have hop limit 255, a good
//!   checksum, well formed options, and the target or its solicited node
//!   address as destination, as RFC 4861 section 7.1.1 says.
//! * VLAN tags of the request are copied to the reply.
//! * It never answers for the gateway or for any address outside the table.
//!
//! The reply is written into a buffer of the caller, so the hot path
//! allocates nothing. [`Responder`] has no mutable state, so workers share one.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::mac::MacBlock;
use crate::pool::{HostPool, MacAddr};
use crate::slice::HostRange;
use crate::wire::{
    self, ethernet_payload, icmp6_checksum, solicited_node, ALL_NODES, ARP_REPLY, ARP_REQUEST,
    ETHERTYPE_ARP, ETHERTYPE_IPV6, ICMP6_NA, ICMP6_NS, MAX_REPLY_LEN, MIN_FRAME_LEN, NA_OVERRIDE,
    NA_SOLICITED, NEXT_HEADER_ICMP6, OPT_SOURCE_LLA, OPT_TARGET_LLA,
};

/// Errors that building a [`NeighborTable`] reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NeighborError {
    /// The MAC block belongs to another pool or slice than the one added.
    #[error("host pool {pool}: the MAC block is for pool {block_pool} slice {block_offset}+{block_count}, not slice {offset}+{count}")]
    BlockMismatch {
        /// The pool that was added.
        pool: String,
        /// The pool of the block.
        block_pool: String,
        /// The first host index of the block.
        block_offset: u32,
        /// The number of hosts of the block.
        block_count: u32,
        /// The first host index of the slice.
        offset: u32,
        /// The number of hosts of the slice.
        count: u32,
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
    /// Two slices of the table answer for the same addresses.
    #[error("host pools {first} and {second} overlap in their {family} addresses")]
    AddressOverlap {
        /// The pool that is in the table.
        first: String,
        /// The pool that was added.
        second: String,
        /// `ipv4` or `ipv6`.
        family: &'static str,
    },
    /// A host index of the slice has no address, which a parsed pool rules out.
    #[error("host pool {pool}: host index {index} has no address")]
    NoAddress {
        /// The pool name.
        pool: String,
        /// The host index.
        index: u32,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    pool: HostPool,
    macs: MacBlock,
    v4: Option<(u32, u32)>,
    v6: Option<(u128, u128)>,
}

/// The hosts that one port answers for, with their addresses and MAC
/// addresses.
#[derive(Debug, Clone, Default)]
pub struct NeighborTable {
    entries: Vec<Entry>,
    hosts: u64,
}

impl NeighborTable {
    /// Returns an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the hosts of `macs.range()` in `pool`, which must be the block
    /// that [`crate::MacAllocator::claim`] returned for that slice.
    ///
    /// The addresses of the slice must not overlap those of a slice in the
    /// table, because one address then has two answers.
    pub fn add(&mut self, pool: &HostPool, macs: &MacBlock) -> Result<(), NeighborError> {
        let range: HostRange = macs.range();
        if macs.pool() != pool.name() {
            return Err(NeighborError::BlockMismatch {
                pool: pool.name().to_owned(),
                block_pool: macs.pool().to_owned(),
                block_offset: range.offset,
                block_count: range.count,
                offset: range.offset,
                count: range.count,
            });
        }
        if range.is_empty() || range.end() > u64::from(pool.count()) {
            return Err(NeighborError::BadSlice {
                pool: pool.name().to_owned(),
                offset: range.offset,
                count: range.count,
                pool_count: pool.count(),
            });
        }
        let last = range.offset + (range.count - 1);
        let no_address = |index| NeighborError::NoAddress {
            pool: pool.name().to_owned(),
            index,
        };
        // Addresses grow with the host index, so the first and last host give
        // the span of the slice.
        let v4 = if pool.ipv4().is_some() {
            let lo = pool
                .ipv4_addr(range.offset)
                .ok()
                .flatten()
                .ok_or_else(|| no_address(range.offset))?;
            let hi = pool
                .ipv4_addr(last)
                .ok()
                .flatten()
                .ok_or_else(|| no_address(last))?;
            Some((u32::from(lo), u32::from(hi)))
        } else {
            None
        };
        let v6 = if pool.ipv6().is_some() {
            let lo = pool
                .ipv6_addr(range.offset)
                .ok()
                .flatten()
                .ok_or_else(|| no_address(range.offset))?;
            let hi = pool
                .ipv6_addr(last)
                .ok()
                .flatten()
                .ok_or_else(|| no_address(last))?;
            Some((u128::from(lo), u128::from(hi)))
        } else {
            None
        };
        for e in &self.entries {
            let overlap = |family| NeighborError::AddressOverlap {
                first: e.pool.name().to_owned(),
                second: pool.name().to_owned(),
                family,
            };
            if let (Some((a, b)), Some((c, d))) = (e.v4, v4) {
                if a <= d && c <= b {
                    return Err(overlap("ipv4"));
                }
            }
            if let (Some((a, b)), Some((c, d))) = (e.v6, v6) {
                if a <= d && c <= b {
                    return Err(overlap("ipv6"));
                }
            }
        }
        self.hosts += u64::from(range.count);
        self.entries.push(Entry {
            pool: pool.clone(),
            macs: macs.clone(),
            v4,
            v6,
        });
        Ok(())
    }

    /// The number of hosts in the table.
    pub fn hosts(&self) -> u64 {
        self.hosts
    }

    /// The MAC address of the host that owns the IPv4 address `ip`.
    pub fn lookup_v4(&self, ip: Ipv4Addr) -> Option<MacAddr> {
        let n = u32::from(ip);
        self.entries.iter().find_map(|e| {
            let (lo, hi) = e.v4?;
            if n < lo || n > hi {
                return None;
            }
            e.macs.mac_of(e.pool.index_of_ipv4(ip)?)
        })
    }

    /// The MAC address of the host that owns the IPv6 address `ip`.
    pub fn lookup_v6(&self, ip: Ipv6Addr) -> Option<MacAddr> {
        let n = u128::from(ip);
        self.entries.iter().find_map(|e| {
            let (lo, hi) = e.v6?;
            if n < lo || n > hi {
                return None;
            }
            e.macs.mac_of(e.pool.index_of_ipv6(ip)?)
        })
    }
}

/// The protocol of a frame that the responder looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// ARP.
    Arp,
    /// IPv6 neighbor discovery.
    Nd,
}

/// What the responder did with one received frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The frame was a request for a host of the table. The reply is in the
    /// output buffer and is this many bytes long.
    Reply(Protocol, usize),
    /// The frame is not a neighbor protocol request. The caller keeps it, for
    /// example for the transport stack.
    Pass,
    /// The frame is a neighbor protocol request that no host of the table has
    /// to answer, such as a request for another station, an announcement, or a
    /// reply.
    NotForUs(Protocol),
    /// The frame claims to be a neighbor protocol request and breaks a rule of
    /// its protocol, or the output buffer is too short.
    Malformed(Protocol),
}

/// Answers ARP requests and neighbor solicitations for the hosts of a table.
#[derive(Debug, Clone)]
pub struct Responder {
    table: NeighborTable,
}

fn write(out: &mut [u8], at: usize, bytes: &[u8]) -> Option<()> {
    out.get_mut(at..at + bytes.len())?.copy_from_slice(bytes);
    Some(())
}

fn unicast_station(mac: MacAddr) -> bool {
    mac.0[0] & 1 == 0 && mac.0 != [0; 6]
}

impl Responder {
    /// Returns a responder for the hosts of `table`.
    pub fn new(table: NeighborTable) -> Self {
        Self { table }
    }

    /// The table that the responder answers from.
    pub fn table(&self) -> &NeighborTable {
        &self.table
    }

    /// Looks at one received frame and writes the reply, if any, into `out`.
    ///
    /// `out` must hold at least [`MAX_REPLY_LEN`] bytes. A shorter buffer makes
    /// every reply [`Verdict::Malformed`] rather than panic.
    pub fn process(&self, frame: &[u8], out: &mut [u8]) -> Verdict {
        let Some((pos, ethertype, _, _)) = ethernet_payload(frame) else {
            return Verdict::Pass;
        };
        match ethertype {
            ETHERTYPE_ARP => self.arp(frame, pos, out),
            ETHERTYPE_IPV6 => self.ipv6(frame, pos, out),
            _ => Verdict::Pass,
        }
    }

    fn arp(&self, frame: &[u8], pos: usize, out: &mut [u8]) -> Verdict {
        const P: Protocol = Protocol::Arp;
        let Some(b) = frame.get(pos..) else {
            return Verdict::Pass;
        };
        // Other hardware or protocol types are not ours to judge.
        if b.get(..6) != Some(&[0, 1, 8, 0, 6, 4]) {
            return Verdict::Pass;
        }
        let Some(op) = b.get(6..8) else {
            return Verdict::Malformed(P);
        };
        if op != ARP_REQUEST.to_be_bytes() {
            // Replies and the rarer operations belong to whoever asked.
            return Verdict::Pass;
        }
        let (Some(sender_mac), Some(sender_ip), Some(target_ip)) = (
            b.get(8..14).and_then(|s| <[u8; 6]>::try_from(s).ok()),
            b.get(14..18).and_then(|s| <[u8; 4]>::try_from(s).ok()),
            b.get(24..28).and_then(|s| <[u8; 4]>::try_from(s).ok()),
        ) else {
            return Verdict::Malformed(P);
        };
        let sender_mac = MacAddr(sender_mac);
        if !unicast_station(sender_mac) {
            return Verdict::Malformed(P);
        }
        if sender_ip == target_ip {
            return Verdict::NotForUs(P); // an announcement
        }
        let Some(host_mac) = self.table.lookup_v4(Ipv4Addr::from(target_ip)) else {
            return Verdict::NotForUs(P);
        };
        let total = (pos + 28).max(MIN_FRAME_LEN);
        let built = (|| {
            let head = frame.get(12..pos)?;
            write(out, 0, &sender_mac.0)?;
            write(out, 6, &host_mac.0)?;
            write(out, 12, head)?;
            write(out, pos, &[0, 1, 8, 0, 6, 4])?;
            write(out, pos + 6, &ARP_REPLY.to_be_bytes())?;
            write(out, pos + 8, &host_mac.0)?;
            write(out, pos + 14, &target_ip)?;
            write(out, pos + 18, &sender_mac.0)?;
            write(out, pos + 24, &sender_ip)?;
            out.get_mut(pos + 28..total)?.fill(0);
            Some(())
        })();
        match built {
            Some(()) => Verdict::Reply(P, total),
            None => Verdict::Malformed(P),
        }
    }

    fn ipv6(&self, frame: &[u8], pos: usize, out: &mut [u8]) -> Verdict {
        const P: Protocol = Protocol::Nd;
        let Some(ip) = frame.get(pos..) else {
            return Verdict::Pass;
        };
        // Only a packet with ICMPv6 right after the fixed header can be a
        // neighbor solicitation, and RFC 4861 allows no extension header.
        if ip.len() < 41
            || ip.first().map(|v| v >> 4) != Some(6)
            || ip.get(6) != Some(&NEXT_HEADER_ICMP6)
        {
            return Verdict::Pass;
        }
        if ip.get(40) != Some(&ICMP6_NS) {
            return Verdict::Pass;
        }
        let len = usize::from(u16::from_be_bytes([ip[4], ip[5]]));
        let Some(icmp) = ip.get(40..40 + len) else {
            return Verdict::Malformed(P);
        };
        let (Some(src), Some(dst), Some(target)) = (addr6(ip, 8), addr6(ip, 24), addr6(icmp, 8))
        else {
            return Verdict::Malformed(P);
        };
        if len < 24 || ip.get(7) != Some(&255) || icmp.get(1) != Some(&0) {
            return Verdict::Malformed(P);
        }
        if icmp6_checksum(src, dst, icmp) != 0 || target.is_multicast() || src.is_multicast() {
            return Verdict::Malformed(P);
        }
        let Some(source_lla) = nd_options(icmp) else {
            return Verdict::Malformed(P);
        };
        let probe = src.is_unspecified();
        if probe && (source_lla.is_some() || dst != solicited_node(target)) {
            return Verdict::Malformed(P);
        }
        if dst != target && dst != solicited_node(target) {
            return Verdict::NotForUs(P);
        }
        let Some(host_mac) = self.table.lookup_v6(target) else {
            return Verdict::NotForUs(P);
        };
        let eth_src = frame.get(6..12).and_then(|s| <[u8; 6]>::try_from(s).ok());
        let (reply_dst_ip, flags, eth_dst) = if probe {
            (ALL_NODES, NA_OVERRIDE, Some(wire::multicast_mac(ALL_NODES)))
        } else {
            let to = source_lla
                .filter(|m| unicast_station(*m))
                .or(eth_src.map(MacAddr))
                .filter(|m| unicast_station(*m));
            (src, NA_SOLICITED | NA_OVERRIDE, to)
        };
        let Some(eth_dst) = eth_dst else {
            return Verdict::Malformed(P);
        };
        let mut na = [0u8; 32];
        na[0] = ICMP6_NA;
        na[4] = flags;
        na[8..24].copy_from_slice(&target.octets());
        na[24] = OPT_TARGET_LLA;
        na[25] = 1;
        na[26..32].copy_from_slice(&host_mac.0);
        let csum = icmp6_checksum(target, reply_dst_ip, &na);
        na[2..4].copy_from_slice(&csum.to_be_bytes());
        let built = (|| {
            let head = frame.get(12..pos)?;
            write(out, 0, &eth_dst.0)?;
            write(out, 6, &host_mac.0)?;
            write(out, 12, head)?;
            write(out, pos, &[0x60, 0, 0, 0, 0, 32, NEXT_HEADER_ICMP6, 255])?;
            write(out, pos + 8, &target.octets())?;
            write(out, pos + 24, &reply_dst_ip.octets())?;
            write(out, pos + 40, &na)?;
            Some(pos + 72)
        })();
        match built {
            Some(total) => Verdict::Reply(P, total),
            None => Verdict::Malformed(P),
        }
    }
}

fn addr6(b: &[u8], at: usize) -> Option<Ipv6Addr> {
    let s: [u8; 16] = b.get(at..at + 16)?.try_into().ok()?;
    Some(Ipv6Addr::from(s))
}

/// Checks the options of a neighbor solicitation, which start at byte 24 of
/// the ICMPv6 message, and returns the source link layer address if the
/// message has one. `None` means that an option is malformed.
fn nd_options(icmp: &[u8]) -> Option<Option<MacAddr>> {
    let mut i = 24;
    let mut source = None;
    while i < icmp.len() {
        let kind = *icmp.get(i)?;
        let size = usize::from(*icmp.get(i + 1)?) * 8;
        if size == 0 || i + size > icmp.len() {
            return None;
        }
        if kind == OPT_SOURCE_LLA && size == 8 {
            let s: [u8; 6] = icmp.get(i + 2..i + 8)?.try_into().ok()?;
            source = Some(MacAddr(s));
        }
        i += size;
    }
    Some(source)
}

// The longest reply is an Ethernet header with two VLAN tags and a neighbor
// advertisement with one option, which is what `MAX_REPLY_LEN` promises.
const _: () = assert!(MAX_REPLY_LEN >= 14 + 8 + 40 + 32);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mac::MacAllocator;
    use crate::pool::PoolSpec;
    use crate::wire::{
        arp_frame, arp_request, ns_frame, parse_arp, parse_neighbor_advert, BROADCAST_MAC,
    };

    fn dual_pool() -> HostPool {
        HostPool::parse(&PoolSpec {
            name: "dual",
            ipv4: Some("10.1.0.0/16"),
            ipv6: Some("2001:db8:1::/64"),
            count: 1000,
            gateway: Some("10.1.0.1"),
            mac_base: Some("02:aa:00:00:00:00"),
        })
        .unwrap()
    }

    fn responder(pool: &HostPool, range: HostRange) -> Responder {
        let mut alloc = MacAllocator::new();
        let block = alloc.claim(pool, range).unwrap();
        let mut table = NeighborTable::new();
        table.add(pool, &block).unwrap();
        Responder::new(table)
    }

    const ROUTER: MacAddr = MacAddr([0x02, 0xbb, 0, 0, 0, 1]);
    const ROUTER_IP: Ipv4Addr = Ipv4Addr::new(10, 1, 0, 1);

    #[test]
    fn index_of_inverts_the_address_mapping() {
        let p = dual_pool();
        for i in [0, 1, 2, 500, 999] {
            let v4 = p.ipv4_addr(i).unwrap().unwrap();
            let v6 = p.ipv6_addr(i).unwrap().unwrap();
            assert_eq!(p.index_of_ipv4(v4), Some(i));
            assert_eq!(p.index_of_ipv6(v6), Some(i));
        }
        // Gateway, network, broadcast, outside, and past the count.
        assert_eq!(p.index_of_ipv4(ROUTER_IP), None);
        assert_eq!(p.index_of_ipv4(Ipv4Addr::new(10, 1, 0, 0)), None);
        assert_eq!(p.index_of_ipv4(Ipv4Addr::new(10, 1, 255, 255)), None);
        assert_eq!(p.index_of_ipv4(Ipv4Addr::new(10, 2, 0, 5)), None);
        assert_eq!(p.index_of_ipv4(Ipv4Addr::new(10, 1, 3, 250)), None);
        assert_eq!(p.index_of_ipv6("2001:db8:1::".parse().unwrap()), None);
        assert_eq!(p.index_of_ipv6("2001:db8:1::ffff".parse().unwrap()), None);
    }

    #[test]
    fn arp_request_for_a_host_gets_the_host_mac() {
        let p = dual_pool();
        let r = responder(&p, p.range());
        let target = p.ipv4_addr(41).unwrap().unwrap();
        let req = arp_request(ROUTER, ROUTER_IP, target, &[]);
        let mut out = [0u8; MAX_REPLY_LEN];
        let Verdict::Reply(Protocol::Arp, len) = r.process(&req, &mut out) else {
            panic!("no reply");
        };
        assert_eq!(len, 60);
        let reply = parse_arp(&out[..len]).unwrap();
        assert_eq!(reply.op, ARP_REPLY);
        assert_eq!(reply.sender_mac, p.mac(41).unwrap().unwrap());
        assert_eq!(reply.eth_src, reply.sender_mac);
        assert_eq!(reply.sender_ip, target);
        assert_eq!(reply.target_mac, ROUTER);
        assert_eq!(reply.target_ip, ROUTER_IP);
        assert_eq!(reply.eth_dst, ROUTER);
    }

    #[test]
    fn arp_reply_keeps_vlan_tags() {
        let p = dual_pool();
        let r = responder(&p, p.range());
        let target = p.ipv4_addr(3).unwrap().unwrap();
        let req = arp_request(ROUTER, ROUTER_IP, target, &[100, 7]);
        let mut out = [0u8; MAX_REPLY_LEN];
        let Verdict::Reply(_, len) = r.process(&req, &mut out) else {
            panic!("no reply");
        };
        let reply = parse_arp(&out[..len]).unwrap();
        assert_eq!((reply.vlans, reply.vlan_tags), ([100, 7], 2));
    }

    #[test]
    fn arp_for_others_announcements_replies_and_the_gateway_are_not_answered() {
        let p = dual_pool();
        let r = responder(&p, HostRange::new(0, 10));
        let mut out = [0u8; MAX_REPLY_LEN];
        // A host of the pool that is outside the slice.
        let other = p.ipv4_addr(500).unwrap().unwrap();
        let v = r.process(&arp_request(ROUTER, ROUTER_IP, other, &[]), &mut out);
        assert_eq!(v, Verdict::NotForUs(Protocol::Arp));
        // The gateway address.
        let v = r.process(
            &arp_request(ROUTER, Ipv4Addr::new(10, 1, 9, 9), ROUTER_IP, &[]),
            &mut out,
        );
        assert_eq!(v, Verdict::NotForUs(Protocol::Arp));
        // A gratuitous announcement of a host address.
        let mine = p.ipv4_addr(2).unwrap().unwrap();
        let v = r.process(&arp_request(ROUTER, mine, mine, &[]), &mut out);
        assert_eq!(v, Verdict::NotForUs(Protocol::Arp));
        // An ARP reply goes to the caller.
        let rep = arp_frame(
            ARP_REPLY,
            ROUTER,
            ROUTER,
            &[],
            ROUTER,
            ROUTER_IP,
            BROADCAST_MAC,
            mine,
        );
        assert_eq!(r.process(&rep, &mut out), Verdict::Pass);
    }

    #[test]
    fn arp_probe_is_defended() {
        let p = dual_pool();
        let r = responder(&p, p.range());
        let mine = p.ipv4_addr(9).unwrap().unwrap();
        let probe = arp_request(ROUTER, Ipv4Addr::UNSPECIFIED, mine, &[]);
        let mut out = [0u8; MAX_REPLY_LEN];
        let Verdict::Reply(_, len) = r.process(&probe, &mut out) else {
            panic!("probe not defended");
        };
        let reply = parse_arp(&out[..len]).unwrap();
        assert_eq!(reply.target_ip, Ipv4Addr::UNSPECIFIED);
        assert_eq!(reply.sender_ip, mine);
    }

    #[test]
    fn malformed_arp_and_short_buffers_never_panic() {
        let p = dual_pool();
        let r = responder(&p, p.range());
        let target = p.ipv4_addr(1).unwrap().unwrap();
        let good = arp_request(ROUTER, ROUTER_IP, target, &[]);
        let mut out = [0u8; MAX_REPLY_LEN];
        // Every truncation of a good request returns without a panic.
        for n in 0..good.len() {
            let _ = r.process(&good[..n], &mut out);
        }
        // A multicast sender is malformed.
        let bad = arp_request(MacAddr([0x01, 0, 0x5e, 0, 0, 1]), ROUTER_IP, target, &[]);
        assert_eq!(r.process(&bad, &mut out), Verdict::Malformed(Protocol::Arp));
        // A buffer that is too short is reported, not written past.
        let mut short = [0u8; 20];
        assert_eq!(
            r.process(&good, &mut short),
            Verdict::Malformed(Protocol::Arp)
        );
        // Other Ethernet types pass.
        let mut ipv4 = good.clone();
        ipv4[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        assert_eq!(r.process(&ipv4, &mut out), Verdict::Pass);
    }

    fn router6() -> Ipv6Addr {
        "2001:db8:ffff::1".parse().unwrap()
    }

    #[test]
    fn solicitation_for_a_host_gets_an_advertisement() {
        let p = dual_pool();
        let r = responder(&p, p.range());
        let target = p.ipv6_addr(77).unwrap().unwrap();
        let ns = ns_frame(ROUTER, router6(), target, &[200]);
        let mut out = [0u8; MAX_REPLY_LEN];
        let Verdict::Reply(Protocol::Nd, len) = r.process(&ns, &mut out) else {
            panic!("no reply");
        };
        let na = parse_neighbor_advert(&out[..len]).unwrap();
        assert!(na.checksum_ok);
        assert_eq!(na.hop_limit, 255);
        assert_eq!(na.flags, NA_SOLICITED | NA_OVERRIDE);
        assert_eq!(na.ip_src, target);
        assert_eq!(na.ip_dst, router6());
        assert_eq!(na.target, target);
        assert_eq!(na.target_lla, p.mac(77).unwrap());
        assert_eq!(na.eth_src, p.mac(77).unwrap().unwrap());
        assert_eq!(na.eth_dst, ROUTER);
        assert_eq!(&out[12..14], &[0x81, 0x00]);
        assert_eq!(&out[14..16], &[0x00, 200]);
    }

    #[test]
    fn duplicate_address_detection_is_answered_to_all_nodes() {
        let p = dual_pool();
        let r = responder(&p, p.range());
        let target = p.ipv6_addr(5).unwrap().unwrap();
        let ns = ns_frame(ROUTER, Ipv6Addr::UNSPECIFIED, target, &[]);
        let mut out = [0u8; MAX_REPLY_LEN];
        let Verdict::Reply(_, len) = r.process(&ns, &mut out) else {
            panic!("no reply");
        };
        let na = parse_neighbor_advert(&out[..len]).unwrap();
        assert_eq!(na.ip_dst, ALL_NODES);
        assert_eq!(na.eth_dst.0, [0x33, 0x33, 0, 0, 0, 1]);
        assert_eq!(na.flags, NA_OVERRIDE);
        assert!(na.checksum_ok);
    }

    #[test]
    fn solicitation_rules_of_rfc_4861_are_enforced() {
        let p = dual_pool();
        let r = responder(&p, HostRange::new(0, 100));
        let target = p.ipv6_addr(5).unwrap().unwrap();
        let good = ns_frame(ROUTER, router6(), target, &[]);
        let mut out = [0u8; MAX_REPLY_LEN];
        assert!(matches!(r.process(&good, &mut out), Verdict::Reply(..)));
        // Hop limit below 255.
        let mut f = good.clone();
        f[14 + 7] = 254;
        assert_eq!(r.process(&f, &mut out), Verdict::Malformed(Protocol::Nd));
        // A flipped bit breaks the checksum.
        let mut f = good.clone();
        f[14 + 40 + 10] ^= 1;
        assert_eq!(r.process(&f, &mut out), Verdict::Malformed(Protocol::Nd));
        // An option of length zero.
        let mut f = good.clone();
        f[14 + 40 + 25] = 0;
        assert_eq!(r.process(&f, &mut out), Verdict::Malformed(Protocol::Nd));
        // A target outside the slice is not ours.
        let far = p.ipv6_addr(600).unwrap().unwrap();
        assert_eq!(
            r.process(&ns_frame(ROUTER, router6(), far, &[]), &mut out),
            Verdict::NotForUs(Protocol::Nd)
        );
        // The network address is not a host.
        let net: Ipv6Addr = "2001:db8:1::".parse().unwrap();
        assert_eq!(
            r.process(&ns_frame(ROUTER, router6(), net, &[]), &mut out),
            Verdict::NotForUs(Protocol::Nd)
        );
        // Other ICMPv6 messages pass.
        let mut f = good.clone();
        f[14 + 40] = 128;
        assert_eq!(r.process(&f, &mut out), Verdict::Pass);
        // No truncation panics.
        for n in 0..good.len() {
            let _ = r.process(&good[..n], &mut out);
        }
        let mut short = [0u8; 40];
        assert_eq!(
            r.process(&good, &mut short),
            Verdict::Malformed(Protocol::Nd)
        );
    }

    #[test]
    fn table_refuses_overlap_and_mismatch() {
        let p = dual_pool();
        let mut alloc = MacAllocator::new();
        let block = alloc.claim(&p, HostRange::new(0, 100)).unwrap();
        let mut table = NeighborTable::new();
        table.add(&p, &block).unwrap();
        // The same pool again overlaps in addresses.
        let again = alloc.claim(&p, HostRange::new(100, 100)).unwrap();
        table.add(&p, &again).unwrap();
        assert_eq!(table.hosts(), 200);
        let dup = MacAllocator::new()
            .claim(&p, HostRange::new(50, 10))
            .unwrap();
        assert!(matches!(
            table.add(&p, &dup),
            Err(NeighborError::AddressOverlap { family: "ipv4", .. })
        ));
        // A block of another pool.
        let other = HostPool::parse(&PoolSpec {
            name: "other",
            ipv4: Some("10.9.0.0/16"),
            ipv6: None,
            count: 10,
            gateway: None,
            mac_base: Some("02:cc:00:00:00:00"),
        })
        .unwrap();
        let ob = MacAllocator::new().claim(&other, other.range()).unwrap();
        assert!(matches!(
            table.add(&p, &ob),
            Err(NeighborError::BlockMismatch { .. })
        ));
        table.add(&other, &ob).unwrap();
        assert_eq!(table.hosts(), 210);
        assert_eq!(
            table.lookup_v4(Ipv4Addr::new(10, 9, 0, 3)),
            other.mac(2).unwrap()
        );
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// `index_of_*` inverts `*_addr` for every host, with and without a
            /// gateway inside the prefix, and refuses what is not a host.
            #[test]
            fn index_of_is_the_inverse_of_the_address(
                count in 1u32..2000,
                gateway_offset in 0u32..40,
                probe in 0u32..4000,
            ) {
                let net = Ipv4Addr::new(10, 7, 0, 0);
                let gw = Ipv4Addr::from(u32::from(net) + gateway_offset);
                let gw6 = Ipv6Addr::from(u128::from("2001:db8:7::".parse::<Ipv6Addr>().unwrap()) + u128::from(gateway_offset));
                for (v4_gw, v6_gw) in [(gw.to_string(), gw6.to_string())] {
                    let p4 = HostPool::parse(&PoolSpec {
                        name: "p", ipv4: Some("10.7.0.0/20"), ipv6: None, count,
                        gateway: Some(&v4_gw), mac_base: None,
                    }).unwrap();
                    let p6 = HostPool::parse(&PoolSpec {
                        name: "p", ipv4: None, ipv6: Some("2001:db8:7::/64"), count,
                        gateway: Some(&v6_gw), mac_base: None,
                    }).unwrap();
                    for i in [0, count / 2, count - 1] {
                        let a4 = p4.ipv4_addr(i).unwrap().unwrap();
                        prop_assert_eq!(p4.index_of_ipv4(a4), Some(i));
                        let a6 = p6.ipv6_addr(i).unwrap().unwrap();
                        prop_assert_eq!(p6.index_of_ipv6(a6), Some(i));
                    }
                    // Any probe address maps to an index whose address is the probe.
                    let probe4 = Ipv4Addr::from(u32::from(net) + probe);
                    if let Some(i) = p4.index_of_ipv4(probe4) {
                        prop_assert_eq!(p4.ipv4_addr(i).unwrap(), Some(probe4));
                    }
                    let probe6 = Ipv6Addr::from(u128::from("2001:db8:7::".parse::<Ipv6Addr>().unwrap()) + u128::from(probe));
                    if let Some(i) = p6.index_of_ipv6(probe6) {
                        prop_assert_eq!(p6.ipv6_addr(i).unwrap(), Some(probe6));
                    }
                    prop_assert_eq!(p4.index_of_ipv4(gw), None);
                    prop_assert_eq!(p6.index_of_ipv6(gw6), None);
                }
            }

            /// Random bytes never panic the responder, and a reply, when there
            /// is one, never exceeds the promised length.
            #[test]
            fn random_frames_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..200), arp in any::<bool>()) {
                let p = dual_pool();
                let r = responder(&p, p.range());
                let mut frame = bytes;
                if frame.len() >= 14 {
                    let et: u16 = if arp { 0x0806 } else { 0x86dd };
                    frame[12..14].copy_from_slice(&et.to_be_bytes());
                }
                let mut out = [0u8; MAX_REPLY_LEN];
                if let Verdict::Reply(_, len) = r.process(&frame, &mut out) {
                    prop_assert!(len <= MAX_REPLY_LEN);
                }
            }

            /// The allocator never gives one address to two hosts, whatever
            /// order or sizes the pools come in.
            #[test]
            fn claims_never_overlap(sizes in proptest::collection::vec(1u32..5000, 1..12)) {
                let mut alloc = MacAllocator::new();
                let mut seen: Vec<(u64, u64)> = Vec::new();
                for (n, size) in sizes.iter().enumerate() {
                    let p = HostPool::parse(&PoolSpec {
                        name: &format!("pool-{n}"), ipv4: None, ipv6: Some("2001:db8::/64"),
                        count: *size, gateway: None, mac_base: None,
                    }).unwrap();
                    let b = alloc.claim(&p, p.range()).unwrap();
                    let lo = b.first().0.iter().fold(0u64, |a, x| (a << 8) | u64::from(*x));
                    let hi = lo + u64::from(*size) - 1;
                    prop_assert!(b.first().0[0] & 1 == 0);
                    for (a, z) in &seen {
                        prop_assert!(hi < *a || lo > *z);
                    }
                    seen.push((lo, hi));
                }
            }
        }
    }
}
