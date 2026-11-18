// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests of Ethernet framing, VLAN tags, and neighbor lookup.

use super::*;
use fps_hosts::wire::{arp_request, parse_arp};
use fps_hosts::{HostPool, HostRange, MacAllocator, PoolSpec};

fn pool(name: &str, prefix: &str, count: u32, mac_base: Option<&str>) -> HostPool {
    HostPool::parse(&PoolSpec {
        name,
        ipv4: Some(prefix),
        count,
        mac_base,
        ..PoolSpec::default()
    })
    .expect("pool")
}

fn table(pool: &HostPool, ledger: &mut MacAllocator) -> NeighborTable {
    let block = ledger
        .claim(pool, HostRange::new(0, pool.count()))
        .expect("claim");
    let mut t = NeighborTable::new();
    t.add(pool, &block).expect("add");
    t
}

/// The station of a client pool with the MAC base 02:aa:00:00:00:10 whose
/// neighbors are a server pool with the base 02:bb:00:00:00:00.
fn station(vlan: Option<u16>) -> Station {
    let client = pool("c", "10.1.0.0/24", 10, Some("02:aa:00:00:00:10"));
    let server = pool("s", "10.2.0.0/24", 3, Some("02:bb:00:00:00:00"));
    let mut ledger = MacAllocator::new();
    let (c, s) = (table(&client, &mut ledger), table(&server, &mut ledger));
    Station::new(c, s, vlan).expect("station")
}

const C0: u32 = 0x0A01_0001;
const S2: u32 = 0x0A02_0003;

fn mac(last: u8, base: u8) -> MacAddr {
    MacAddr([0x02, base, 0, 0, 0, last])
}

#[test]
fn an_untagged_header_round_trips() {
    let mut out = Vec::new();
    encode_header(&mut out, mac(1, 0xbb), mac(2, 0xaa), None, ETHERTYPE_IPV4);
    assert_eq!(out.len(), ETH_LEN);
    assert_eq!(&out[12..14], &[0x08, 0x00]);
    let head = decode_header(&out).expect("decode");
    assert_eq!(head.dst, mac(1, 0xbb));
    assert_eq!(head.src, mac(2, 0xaa));
    assert_eq!((head.vlan, head.pcp), (None, 0));
    assert_eq!(head.ethertype, ETHERTYPE_IPV4);
    assert_eq!(head.payload_at, ETH_LEN);
}

#[test]
fn a_tagged_header_round_trips_and_carries_the_vlan_id() {
    for id in [1u16, 100, 4094] {
        let mut out = Vec::new();
        encode_header(
            &mut out,
            mac(1, 0xbb),
            mac(2, 0xaa),
            Some(id),
            ETHERTYPE_ARP,
        );
        assert_eq!(out.len(), ETH_LEN + VLAN_TAG_LEN);
        assert_eq!(&out[12..14], &[0x81, 0x00]);
        let head = decode_header(&out).expect("decode");
        assert_eq!(head.vlan, Some(id));
        assert_eq!(head.ethertype, ETHERTYPE_ARP);
        assert_eq!(head.payload_at, 18);
    }
}

#[test]
fn the_priority_bits_do_not_leak_into_the_vlan_id() {
    let mut out = Vec::new();
    encode_header(
        &mut out,
        mac(1, 0xbb),
        mac(2, 0xaa),
        Some(7),
        ETHERTYPE_IPV4,
    );
    out[14] |= 0xE0; // priority 7
    let head = decode_header(&out).expect("decode");
    assert_eq!((head.vlan, head.pcp), (Some(7), 7));
}

#[test]
fn short_and_stacked_tags_are_rejected() {
    assert_eq!(decode_header(&[0; 13]), Err(NetError::Short));
    let mut tagged = Vec::new();
    encode_header(
        &mut tagged,
        mac(1, 0xbb),
        mac(2, 0xaa),
        Some(5),
        ETHERTYPE_IPV4,
    );
    assert_eq!(decode_header(&tagged[..16]), Err(NetError::Short));
    // A service tag (802.1ad) in the first slot, and a second 802.1Q tag.
    let mut qinq = tagged.clone();
    qinq[12..14].copy_from_slice(&[0x88, 0xA8]);
    assert_eq!(decode_header(&qinq), Err(NetError::UnsupportedTag));
    let mut stacked = tagged;
    stacked[16..18].copy_from_slice(&[0x81, 0x00]);
    assert_eq!(decode_header(&stacked), Err(NetError::UnsupportedTag));
}

#[test]
fn the_station_rejects_vlan_ids_outside_the_range() {
    let t = NeighborTable::new();
    for id in [0u16, 4095, 5000] {
        assert_eq!(
            Station::new(t.clone(), t.clone(), Some(id)).err(),
            Some(NetError::BadVlan(id))
        );
    }
    assert!(Station::new(t.clone(), t, Some(4094)).is_ok());
}

#[test]
fn host_i_of_a_pool_has_the_mac_base_plus_i() {
    let st = station(None);
    // Host 0 of the client pool is 10.1.0.1, host 9 is 10.1.0.10.
    assert_eq!(st.local_mac(C0), Some(MacAddr([0x02, 0xaa, 0, 0, 0, 0x10])));
    assert_eq!(
        st.local_mac(0x0A01_000A),
        Some(MacAddr([0x02, 0xaa, 0, 0, 0, 0x19]))
    );
    assert_eq!(st.resolve(0x0A02_0001), Some(mac(0, 0xbb)));
    assert_eq!(st.resolve(S2), Some(mac(2, 0xbb)));
    assert_eq!(st.hosts(), 10);
}

#[test]
fn neighbor_lookup_fails_outside_the_tables() {
    let st = station(None);
    // Past the last host, the network address, another prefix, and a local
    // host looked up as a neighbor.
    for ip in [0x0A02_0004, 0x0A02_0000, 0x0A03_0001, C0] {
        assert_eq!(st.resolve(ip), None, "{ip:#x}");
    }
    assert!(st.owns(C0));
    assert!(!st.owns(S2));
    let mut out = Vec::new();
    assert!(!st.head(&mut out, C0, 0x0A02_0004));
    assert!(!st.head(&mut out, S2, S2));
    assert!(out.is_empty(), "a failed lookup writes nothing");
}

#[test]
fn the_head_names_the_neighbor_and_the_local_host() {
    for vlan in [None, Some(42)] {
        let st = station(vlan);
        let mut out = Vec::new();
        assert!(st.head(&mut out, C0, S2));
        let head = decode_header(&out).expect("decode");
        assert_eq!(head.src, MacAddr([0x02, 0xaa, 0, 0, 0, 0x10]));
        assert_eq!(head.dst, mac(2, 0xbb));
        assert_eq!(head.vlan, vlan);
        assert_eq!(head.ethertype, ETHERTYPE_IPV4);
    }
}

fn ip_frame(vlan: Option<u16>) -> Vec<u8> {
    let mut f = Vec::new();
    encode_header(&mut f, mac(0, 0xaa), mac(0, 0xbb), vlan, ETHERTYPE_IPV4);
    f.extend_from_slice(&[0x45, 0, 0, 20]);
    f
}

#[test]
fn receive_hands_up_ip_and_checks_the_vlan() {
    let mut reply = [0u8; MAX_REPLY_LEN];
    let plain = station(None);
    let tagged = station(Some(42));
    let f = ip_frame(None);
    assert_eq!(
        plain.receive(&f, &mut reply),
        Ok(Received::Ip(&f[ETH_LEN..]))
    );
    let f = ip_frame(Some(42));
    assert_eq!(tagged.receive(&f, &mut reply), Ok(Received::Ip(&f[18..])));
    // Another VLAN, a tag on an untagged port, and no tag on a tagged one.
    assert_eq!(
        tagged.receive(&ip_frame(Some(43)), &mut reply),
        Err(NetError::OtherVlan)
    );
    assert_eq!(
        plain.receive(&ip_frame(Some(42)), &mut reply),
        Err(NetError::TagMismatch)
    );
    assert_eq!(
        tagged.receive(&ip_frame(None), &mut reply),
        Err(NetError::TagMismatch)
    );
    assert_eq!(plain.receive(&[0; 5], &mut reply), Err(NetError::Short));
}

#[test]
fn receive_ignores_other_ethertypes() {
    let st = station(None);
    let mut reply = [0u8; MAX_REPLY_LEN];
    let mut f = Vec::new();
    encode_header(&mut f, mac(0, 0xaa), mac(0, 0xbb), None, 0x88B5);
    f.extend_from_slice(&[0; 40]);
    assert_eq!(st.receive(&f, &mut reply), Ok(Received::Ignored));
}

/// A station whose pools have an IPv6 prefix beside the IPv4 one, so a host
/// has both addresses and one MAC address.
fn dual_station() -> Station {
    let pool = |name, v4, v6, count, base| {
        HostPool::parse(&PoolSpec {
            name,
            ipv4: Some(v4),
            ipv6: Some(v6),
            count,
            mac_base: Some(base),
            ..PoolSpec::default()
        })
        .expect("pool")
    };
    let client = pool("c", "10.1.0.0/24", "fd00:1::/64", 10, "02:aa:00:00:00:10");
    let server = pool("s", "10.2.0.0/24", "fd00:2::/64", 3, "02:bb:00:00:00:00");
    let mut ledger = MacAllocator::new();
    let (c, s) = (table(&client, &mut ledger), table(&server, &mut ledger));
    Station::new(c, s, None).expect("station")
}

#[test]
fn the_head_of_an_ipv6_packet_has_the_ipv6_ethertype_and_the_same_macs() {
    let st = dual_station();
    let c0: Ipv6Addr = "fd00:1::1".parse().expect("addr");
    let s2: Ipv6Addr = "fd00:2::3".parse().expect("addr");
    let mut v6 = Vec::new();
    assert!(st.head(&mut v6, c0, s2));
    assert_eq!(&v6[12..14], &[0x86, 0xDD]);
    let mut v4 = Vec::new();
    assert!(st.head(&mut v4, C0, S2));
    assert_eq!(&v4[12..14], &[0x08, 0x00]);
    assert_eq!(&v6[..12], &v4[..12], "one MAC address per host");
    assert!(st.owns(c0));
    assert!(!st.owns(s2));
    assert_eq!(st.resolve(s2), Some(mac(2, 0xbb)));
    // An address outside the prefix, and a pair of mixed families.
    let mut out = Vec::new();
    assert!(!st.head(&mut out, c0, "fd00:3::1".parse::<Ipv6Addr>().expect("addr")));
    assert!(!st.head(&mut out, IpAddr::V6(c0), IpAddr::V4(Ipv4Addr::from(S2))));
    assert!(out.is_empty());
}

#[test]
fn an_ipv6_packet_comes_back_as_ip_and_a_solicitation_is_answered() {
    use fps_hosts::wire::ns_frame;
    let st = dual_station();
    let mut reply = [0u8; MAX_REPLY_LEN];
    let mut f = Vec::new();
    encode_header(&mut f, mac(0, 0xaa), mac(0, 0xbb), None, ETHERTYPE_IPV6);
    let mut packet = vec![0x60, 0, 0, 0, 0, 20, 6, 64];
    packet.resize(40 + 20, 0);
    f.extend_from_slice(&packet);
    assert_eq!(st.receive(&f, &mut reply), Ok(Received::Ip(&f[ETH_LEN..])));
    // A solicitation for the host fd00:1::3 (host 2, MAC 02:aa:00:00:00:12).
    let asker = MacAddr([0x02, 0xcc, 0, 0, 0, 1]);
    let target: Ipv6Addr = "fd00:1::3".parse().expect("addr");
    let ns = ns_frame(asker, "fd00:1::99".parse().expect("addr"), target, &[]);
    let Ok(Received::Reply(len)) = st.receive(&ns, &mut reply) else {
        panic!("no reply");
    };
    let advert = fps_hosts::wire::parse_neighbor_advert(&reply[..len]).expect("advert");
    assert_eq!(advert.target, target);
    assert_eq!(
        advert.target_lla,
        Some(MacAddr([0x02, 0xaa, 0, 0, 0, 0x12]))
    );
    assert!(advert.checksum_ok);
    // A solicitation for a host of the other table is not ours to answer.
    let other = ns_frame(
        asker,
        "fd00:1::99".parse().expect("addr"),
        "fd00:2::1".parse().expect("addr"),
        &[],
    );
    assert_eq!(st.receive(&other, &mut reply), Ok(Received::Ignored));
}

#[test]
fn the_station_answers_arp_for_its_hosts_and_for_no_one_else() {
    let st = station(Some(42));
    let asker = MacAddr([0x02, 0xcc, 0, 0, 0, 1]);
    let mut reply = [0u8; MAX_REPLY_LEN];
    let ask = |target: [u8; 4]| arp_request(asker, [10, 1, 0, 99].into(), target.into(), &[42]);
    let request = ask([10, 1, 0, 3]);
    let Ok(Received::Reply(len)) = st.receive(&request, &mut reply) else {
        panic!("no reply");
    };
    let answer = parse_arp(&reply[..len]).expect("arp");
    // Host 2 of the client pool, with the VLAN tag of the request.
    assert_eq!(answer.sender_mac, MacAddr([0x02, 0xaa, 0, 0, 0, 0x12]));
    assert_eq!(answer.target_mac, asker);
    assert_eq!((answer.vlan_tags, answer.vlans[0]), (1, 42));
    // A neighbor of the other table is not ours to answer for.
    assert_eq!(
        st.receive(&ask([10, 2, 0, 1]), &mut reply),
        Ok(Received::Ignored)
    );
}

#[test]
fn deferred_peers_resolve_to_the_next_hop_marker_for_any_address() {
    let s = station(None);
    assert!(!s.defers_peers());
    assert_eq!(s.resolve(S2), Some(mac(3, 0xbb)), "the table answers");
    assert_eq!(s.resolve(0xC0A8_0001u32), None, "and knows no one else");

    let s = station(None).defer_peers();
    assert!(s.defers_peers());
    for ip in [S2, 0xC0A8_0001u32] {
        assert_eq!(s.resolve(ip), Some(NEXT_HOP_MAC), "{ip:#x}");
    }
    assert_eq!(
        s.resolve(std::net::Ipv6Addr::LOCALHOST),
        Some(NEXT_HOP_MAC),
        "a peer of either family"
    );
    // The local hosts keep their MAC addresses, and the frame carries the
    // marker where the neighbor goes.
    let local = s.local_mac(C0).expect("a local host");
    let mut out = Vec::new();
    assert!(s.head(&mut out, C0, 0xC0A8_0001u32));
    assert_eq!(out.get(..6), Some(&NEXT_HOP_MAC.0[..]));
    assert_eq!(out.get(6..12), Some(&local.0[..]));
}
