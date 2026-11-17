// SPDX-License-Identifier: GPL-3.0-only
//! Tests of the DHCP clients of a pool over the in memory frame pair of the
//! AF_XDP backend. No root and no host are needed.
//!
//! The far end of the pair is a small DHCP server of this file. It gives host
//! `i` the address `10.50.0.2 + i`, follows RFC 2131 for the replies (a
//! broadcast reply to a client that asks for one, a unicast reply to the
//! address of a renewing client), and can be told to NAK a host. Time is a
//! `Duration` that the test advances, so a lease of a minute renews in
//! microseconds of wall time.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::time::Duration;

use fps_backend_afxdp::memory::{frame_pair, MemoryFrames};
use fps_backend_afxdp::FrameIo;
use fps_hosts::dhcp::frame::{parse_udp4, udp4_frame};
use fps_hosts::dhcp::{Message, MessageType, Timing, CLIENT_PORT, SERVER_PORT};
use fps_hosts::wire::{arp_request, ethernet_payload, parse_arp, ARP_REPLY, BROADCAST_MAC};
use fps_hosts::{
    DhcpConfig, DhcpError, DhcpHosts, HostPool, HostRange, MacAddr, MacAllocator, NeighborError,
    NeighborService, NeighborTable, Responder,
};

const SERVER_MAC: MacAddr = MacAddr([0x02, 0xee, 0, 0, 0, 0x01]);
const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 50, 0, 1);
const ROUTER_MAC: MacAddr = SERVER_MAC;
const LEASE_SECS: u32 = 120;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The MAC address of host `index` of the pool with `mac_base` 02:aa:00:00:00:00.
fn mac_of(index: u32) -> MacAddr {
    let b = index.to_be_bytes();
    MacAddr([0x02, 0xaa, 0, 0, b[2], b[3]])
}

/// The address that the server gives host `index`.
fn addr_of(index: u32) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(Ipv4Addr::new(10, 50, 0, 2)) + index)
}

/// What the server saw and did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ServerStats {
    discovers: u64,
    selecting_requests: u64,
    renewals: u64,
    rebinds: u64,
    releases: u64,
    naks: u64,
    unicast_replies: u64,
    broadcast_replies: u64,
}

struct Server {
    end: MemoryFrames,
    /// Hosts whose next REQUEST gets a NAK.
    nak: HashSet<MacAddr>,
    /// The address each host gets, by MAC. A NAKed host gets a new one.
    assigned: HashMap<MacAddr, Ipv4Addr>,
    next_extra: u32,
    stats: ServerStats,
    /// Frames of the host that are not DHCP, such as ARP replies.
    other: Vec<Vec<u8>>,
    /// The numbers of VLAN tags that the DHCP frames of the host carried.
    tag_counts: HashSet<usize>,
}

impl Server {
    fn new(end: MemoryFrames) -> Self {
        Self {
            end,
            nak: HashSet::new(),
            assigned: HashMap::new(),
            next_extra: 60_000,
            stats: ServerStats::default(),
            other: Vec::new(),
            tag_counts: HashSet::new(),
        }
    }

    fn address(&mut self, mac: MacAddr) -> Ipv4Addr {
        *self.assigned.entry(mac).or_insert_with(|| {
            let b = mac.0;
            addr_of(u32::from(b[4]) << 8 | u32::from(b[5]))
        })
    }

    fn reply(&mut self, req: &Message, kind: MessageType, vlans: &[u16]) -> Vec<u8> {
        let mut m = Message::new(kind, req.xid, req.chaddr);
        m.options.server_id = Some(SERVER_IP);
        if kind != MessageType::Nak {
            m.yiaddr = self.address(req.chaddr);
            m.options.lease_secs = Some(LEASE_SECS);
            m.options.subnet_mask = Some(Ipv4Addr::new(255, 255, 0, 0));
            m.options.routers = vec![SERVER_IP];
        }
        let (eth_dst, ip_dst) = if req.wants_broadcast() || kind == MessageType::Nak {
            self.stats.broadcast_replies += 1;
            (BROADCAST_MAC, Ipv4Addr::BROADCAST)
        } else {
            self.stats.unicast_replies += 1;
            (req.chaddr, req.ciaddr)
        };
        udp4_frame(
            eth_dst,
            SERVER_MAC,
            vlans,
            SERVER_IP,
            ip_dst,
            SERVER_PORT,
            CLIENT_PORT,
            &m.encode(),
        )
    }

    fn serve(&mut self) {
        let mut inbox = Vec::new();
        self.end
            .recv_batch(4096, &mut |f| inbox.push(f.to_vec()))
            .unwrap();
        let mut out = Vec::new();
        for frame in &inbox {
            let udp = match parse_udp4(frame) {
                Some(u) if u.dst_port == SERVER_PORT => u,
                _ => {
                    self.other.push(frame.clone());
                    continue;
                }
            };
            let msg = Message::parse(udp.payload).unwrap();
            let (_, _, tags, count) = ethernet_payload(frame).unwrap();
            let vlans = &tags[..count];
            self.tag_counts.insert(count);
            match msg.kind {
                MessageType::Discover => {
                    self.stats.discovers += 1;
                    out.push(self.reply(&msg, MessageType::Offer, vlans));
                }
                MessageType::Request => {
                    if msg.ciaddr.is_unspecified() {
                        self.stats.selecting_requests += 1;
                    } else if udp.ip_dst == Ipv4Addr::BROADCAST {
                        self.stats.rebinds += 1;
                    } else {
                        assert_eq!(udp.ip_dst, SERVER_IP, "a renewal goes to the server");
                        assert_eq!(udp.eth_dst, SERVER_MAC, "and to its MAC address");
                        assert_eq!(udp.ip_src, msg.ciaddr, "from the leased address");
                        self.stats.renewals += 1;
                    }
                    let kind = if self.nak.remove(&msg.chaddr) {
                        self.stats.naks += 1;
                        // The host gets another address after the NAK.
                        self.assigned.insert(msg.chaddr, addr_of(self.next_extra));
                        self.next_extra += 1;
                        MessageType::Nak
                    } else {
                        MessageType::Ack
                    };
                    out.push(self.reply(&msg, kind, vlans));
                }
                MessageType::Release => {
                    assert_eq!(msg.ciaddr, self.address(msg.chaddr));
                    self.stats.releases += 1;
                }
                other => panic!("the server got a {other:?}"),
            }
        }
        for f in &out {
            let queued = self.end.send_batch(&[f.as_slice()]).unwrap();
            assert_eq!(queued, 1, "the server queue is full");
        }
    }
}

struct Rig {
    pool: HostPool,
    host: MemoryFrames,
    hosts: DhcpHosts,
    server: Server,
    responder: Responder,
    seen_generation: u64,
    passed: Vec<Vec<u8>>,
    now: Duration,
}

fn pool(count: u32) -> HostPool {
    HostPool::dhcp("clients", count, Some("02:aa:00:00:00:00")).unwrap()
}

fn fast() -> Timing {
    Timing {
        start_delay_max: ms(400),
        initial_retransmit: secs(2),
        max_retransmit: secs(8),
        min_renew_retry: secs(5),
        ..Timing::default()
    }
}

impl Rig {
    fn new(count: u32, range: HostRange, depth: usize, config: DhcpConfig) -> Self {
        let pool = pool(count);
        let mut alloc = MacAllocator::new();
        alloc.reserve("server", SERVER_MAC).unwrap();
        let block = alloc.claim(&pool, range).unwrap();
        let (host, net) = frame_pair(depth, 1514);
        let hosts = DhcpHosts::new(&pool, &block, &config, Duration::ZERO).unwrap();
        let mut rig = Self {
            pool,
            host,
            hosts,
            server: Server::new(net),
            responder: Responder::new(NeighborTable::new()),
            seen_generation: 0,
            passed: Vec::new(),
            now: Duration::ZERO,
        };
        rig.rebuild();
        rig
    }

    fn rebuild(&mut self) {
        let block = {
            // The ledger holds the claim already, so a second allocator makes
            // the same block for the table.
            let mut a = MacAllocator::new();
            a.reserve("server", SERVER_MAC).unwrap();
            a.claim(&self.pool, self.hosts.leases().range()).unwrap()
        };
        let mut t = NeighborTable::new();
        t.add_leased(&self.pool, &block, self.hosts.leases())
            .unwrap();
        self.responder = Responder::new(t);
        self.seen_generation = self.hosts.leases().generation();
    }

    /// One pass of the loop of an agent worker: receive through the neighbor
    /// service, read the DHCP replies, send what is due, then let the server
    /// answer.
    fn turn(&mut self) {
        if self.hosts.leases().generation() != self.seen_generation {
            self.rebuild();
        }
        let now = self.now;
        let hosts = &mut self.hosts;
        let passed = &mut self.passed;
        let mut svc = NeighborService::new(&self.responder, 512);
        svc.poll(&mut self.host, &mut |f| {
            if !hosts.handle_frame(f, now) {
                passed.push(f.to_vec());
            }
        })
        .unwrap();
        hosts.send_due(&mut self.host, now).unwrap();
        self.server.serve();
    }

    /// Advances the clock by `by` in steps of 100 ms, with a few turns per
    /// step so that a message and its reply cross.
    fn advance(&mut self, by: Duration) {
        let end = self.now + by;
        while self.now < end {
            self.now += ms(100);
            for _ in 0..4 {
                self.turn();
            }
        }
    }

    fn advance_until(&mut self, limit: Duration, done: impl Fn(&Rig) -> bool) {
        let end = self.now + limit;
        while !done(self) {
            assert!(self.now < end, "not done after {limit:?}");
            self.advance(ms(100));
        }
    }

    /// Asks `target` as the router does and returns the MAC address of the
    /// ARP reply, if the service answered.
    fn arp(&mut self, target: Ipv4Addr) -> Option<MacAddr> {
        let req = arp_request(ROUTER_MAC, SERVER_IP, target, &[]);
        self.server.end.send_batch(&[req.as_slice()]).unwrap();
        self.turn();
        let mut answer = None;
        for f in self.server.other.drain(..) {
            if let Some(a) = parse_arp(&f) {
                if a.op == ARP_REPLY && a.sender_ip == target {
                    answer = Some(a.sender_mac);
                }
            }
        }
        answer
    }
}

fn config(seed: u64) -> DhcpConfig {
    DhcpConfig {
        timing: fast(),
        seed,
        ..DhcpConfig::default()
    }
}

#[test]
fn the_hosts_of_a_slice_lease_their_addresses_and_answer_arp_for_them() {
    // The slice starts at host 100, so a lease table that counted from 0 would
    // put every address one hundred hosts off.
    let mut rig = Rig::new(1000, HostRange::new(100, 40), 4096, config(1));
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    let leases = rig.hosts.leases();
    assert_eq!(leases.bound(), 40);
    assert_eq!(leases.duplicates(), 0);
    for index in 100..140 {
        assert_eq!(leases.addr_of(index), Some(addr_of(index)), "host {index}");
        assert_eq!(leases.router_of(index), Some(SERVER_IP));
        assert_eq!(leases.index_of(addr_of(index)), Some(index));
    }
    assert_eq!(leases.addr_of(99), None);
    assert_eq!(leases.host_prefixes().len(), 40);
    assert_eq!(leases.subnets().len(), 1);
    assert_eq!(leases.subnets()[0].to_string(), "10.50.0.0/16");
    let stats = rig.hosts.stats();
    assert_eq!(stats.bound, 40);
    assert_eq!(rig.server.stats.discovers, stats.discovers);
    assert_eq!(rig.server.stats.selecting_requests, 40);
    assert_eq!(rig.server.tag_counts, HashSet::from([0]));
    assert_eq!(stats.foreign + stats.malformed, 0);

    // The responder answers for the leased addresses with the MAC of the host,
    // and for nothing else.
    rig.turn();
    for index in [100, 117, 139] {
        assert_eq!(rig.arp(addr_of(index)), Some(mac_of(index)), "host {index}");
    }
    assert_eq!(rig.arp(addr_of(140)), None, "no host leased it");
    assert_eq!(rig.arp(addr_of(99)), None, "host 99 is another slice");
    assert_eq!(rig.arp(SERVER_IP), None, "the server answers for itself");
}

#[test]
fn leases_renew_by_unicast_at_t1_and_the_addresses_stay() {
    let mut rig = Rig::new(100, HostRange::new(0, 25), 4096, config(2));
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    let bound_at = rig.now;
    let generation = rig.hosts.leases().generation();
    let before: Vec<_> = (0..25).map(|i| rig.hosts.leases().addr_of(i)).collect();

    // Before T1 nothing happens, and at T1 every host renews once.
    assert!(rig.now >= bound_at);
    rig.advance(secs(50));
    assert_eq!(rig.server.stats.renewals, 0);
    rig.advance(secs(15));
    assert_eq!(rig.server.stats.renewals, 25);
    assert_eq!(rig.server.stats.rebinds, 0);
    assert_eq!(rig.hosts.stats().renews, 25);
    assert_eq!(rig.hosts.stats().renewed, 25);

    // The addresses and the generation are the same, so the responder that
    // was built after the first binding is still right.
    assert_eq!(rig.hosts.leases().generation(), generation);
    let after: Vec<_> = (0..25).map(|i| rig.hosts.leases().addr_of(i)).collect();
    assert_eq!(before, after);
    assert_eq!(rig.arp(addr_of(3)), Some(mac_of(3)));

    // The new lease renews again one half lease later, and so on forever.
    rig.advance(secs(85));
    assert_eq!(rig.server.stats.renewals, 50);
    assert_eq!(rig.hosts.stats().lost, 0);
    // The server answered the renewals by unicast, which needs the MAC of
    // the host and the address in ciaddr, as the server above asserted.
    assert!(rig.server.stats.unicast_replies >= 50);
}

#[test]
fn a_nak_costs_the_host_its_address_and_it_leases_another() {
    let mut rig = Rig::new(100, HostRange::new(0, 10), 4096, config(3));
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    let old = addr_of(4);
    assert_eq!(rig.arp(old), Some(mac_of(4)));
    let generation = rig.hosts.leases().generation();
    rig.server.nak.insert(mac_of(4));

    rig.advance(secs(u64::from(LEASE_SECS) / 2 + 2));
    assert_eq!(rig.server.stats.naks, 1);
    assert_eq!(rig.hosts.stats().lost, 1);
    // The host starts over after the NAK delay and leases the new address.
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    let new = rig.hosts.leases().addr_of(4).unwrap();
    assert_ne!(new, old);
    assert!(rig.hosts.leases().generation() > generation);
    assert_eq!(rig.hosts.leases().index_of(old), None);
    rig.turn();
    assert_eq!(rig.arp(new), Some(mac_of(4)));
    assert_eq!(
        rig.arp(old),
        None,
        "the old address is not answered any more"
    );
    assert_eq!(rig.hosts.stats().bound, 11);
}

#[test]
fn a_full_transmit_queue_delays_the_messages_and_loses_none() {
    // The queue holds eight frames, and 60 hosts discover within 400 ms.
    let mut rig = Rig::new(100, HostRange::new(0, 60), 8, config(4));
    rig.advance_until(secs(60), |r| r.hosts.leases().is_complete());
    assert_eq!(rig.hosts.pending(), 0);
    assert_eq!(rig.hosts.leases().bound(), 60);
}

#[test]
fn a_burst_limit_spreads_the_first_discovers_over_several_calls() {
    let cfg = DhcpConfig {
        burst: 5,
        timing: Timing {
            start_delay_max: Duration::ZERO,
            ..fast()
        },
        ..config(5)
    };
    let mut rig = Rig::new(100, HostRange::new(0, 20), 4096, cfg);
    rig.now = ms(1);
    let sent = rig.hosts.send_due(&mut rig.host, rig.now).unwrap();
    assert_eq!(sent, 5);
    let sent = rig.hosts.send_due(&mut rig.host, rig.now).unwrap();
    assert_eq!(sent, 5);
    rig.advance_until(secs(60), |r| r.hosts.leases().is_complete());
}

#[test]
fn release_gives_every_lease_back() {
    let mut rig = Rig::new(100, HostRange::new(0, 12), 4096, config(6));
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    let made = rig.hosts.release_all(&mut rig.host, rig.now).unwrap();
    assert_eq!(made, 12);
    rig.server.serve();
    assert_eq!(rig.server.stats.releases, 12);
    assert_eq!(rig.hosts.leases().bound(), 0);
    assert_eq!(rig.hosts.next_deadline(), None);
    // The clients stopped: no message follows, however long the run goes on.
    let sent = rig.hosts.stats().discovers;
    rig.advance(secs(300));
    assert_eq!(rig.hosts.stats().discovers, sent);
}

#[test]
fn a_vlan_port_sends_and_reads_tagged_frames() {
    let cfg = DhcpConfig {
        vlans: vec![100, 200],
        ..config(7)
    };
    let mut rig = Rig::new(100, HostRange::new(0, 5), 4096, cfg);
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    assert_eq!(rig.hosts.leases().bound(), 5);
    assert_eq!(
        rig.server.tag_counts,
        HashSet::from([2]),
        "every frame of the clients carried both tags"
    );
}

#[test]
fn twenty_thousand_hosts_lease_distinct_addresses() {
    let mut rig = Rig::new(20_000, HostRange::new(0, 20_000), 8192, config(8));
    rig.advance_until(secs(120), |r| r.hosts.leases().is_complete());
    let leases = rig.hosts.leases();
    assert_eq!(leases.bound(), 20_000);
    assert_eq!(leases.duplicates(), 0);
    assert_eq!(leases.addr_of(19_999), Some(addr_of(19_999)));
    // One responder for 20,000 leased addresses answers each of them.
    rig.rebuild();
    for index in [0, 9_999, 19_999] {
        assert_eq!(rig.arp(addr_of(index)), Some(mac_of(index)));
    }
}

#[test]
fn frames_that_are_not_dhcp_replies_go_to_the_caller() {
    let mut rig = Rig::new(100, HostRange::new(0, 3), 4096, config(9));
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    rig.passed.clear();
    // An ARP reply, a UDP datagram to another port, and a DHCP reply for a
    // host of another agent.
    let arp = fps_hosts::wire::arp_frame(
        ARP_REPLY,
        mac_of(0),
        ROUTER_MAC,
        &[],
        ROUTER_MAC,
        SERVER_IP,
        mac_of(0),
        addr_of(0),
    );
    let other_port = udp4_frame(
        mac_of(0),
        ROUTER_MAC,
        &[],
        SERVER_IP,
        addr_of(0),
        1000,
        2000,
        b"data",
    );
    let mut m = Message::new(MessageType::Ack, 1, mac_of(50));
    m.yiaddr = addr_of(50);
    m.options.lease_secs = Some(100);
    let foreign = udp4_frame(
        BROADCAST_MAC,
        ROUTER_MAC,
        &[],
        SERVER_IP,
        Ipv4Addr::BROADCAST,
        SERVER_PORT,
        CLIENT_PORT,
        &m.encode(),
    );
    let junk = udp4_frame(
        BROADCAST_MAC,
        ROUTER_MAC,
        &[],
        SERVER_IP,
        Ipv4Addr::BROADCAST,
        SERVER_PORT,
        CLIENT_PORT,
        b"not a dhcp message at all, far too short and without a cookie",
    );
    for f in [&arp, &other_port, &foreign, &junk] {
        rig.server.end.send_batch(&[f.as_slice()]).unwrap();
    }
    rig.turn();
    assert_eq!(rig.passed, vec![arp, other_port]);
    let stats = rig.hosts.stats();
    assert_eq!(stats.foreign, 1, "{stats:?}");
    assert_eq!(stats.malformed, 1, "{stats:?}");
}

#[test]
fn a_reply_addressed_to_another_mac_is_not_ours() {
    let mut rig = Rig::new(100, HostRange::new(0, 3), 4096, config(10));
    rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
    let before = rig.hosts.stats().replies;
    let mut m = Message::new(MessageType::Nak, rig.hosts.clients()[1].xid(), mac_of(1));
    m.options.server_id = Some(SERVER_IP);
    // Host 1's chaddr, but sent to the unicast address of host 2.
    let f = udp4_frame(
        mac_of(2),
        ROUTER_MAC,
        &[],
        SERVER_IP,
        addr_of(2),
        SERVER_PORT,
        CLIENT_PORT,
        &m.encode(),
    );
    rig.server.end.send_batch(&[f.as_slice()]).unwrap();
    rig.turn();
    assert_eq!(rig.hosts.stats().replies, before);
    assert_eq!(rig.hosts.leases().bound(), 3);
}

#[test]
fn the_same_seed_gives_the_same_run() {
    let run = |seed| {
        let mut rig = Rig::new(100, HostRange::new(0, 15), 4096, config(seed));
        rig.advance_until(secs(30), |r| r.hosts.leases().is_complete());
        (rig.hosts.stats(), rig.now)
    };
    assert_eq!(run(11), run(11));
}

#[test]
fn a_static_pool_a_wrong_block_and_too_many_tags_are_refused() {
    let dhcp = pool(10);
    let mut alloc = MacAllocator::new();
    let block = alloc.claim(&dhcp, dhcp.range()).unwrap();
    let static_pool = HostPool::parse(&fps_hosts::PoolSpec {
        name: "clients",
        ipv4: Some("10.0.0.0/24"),
        count: 10,
        ..fps_hosts::PoolSpec::default()
    })
    .unwrap();
    assert!(matches!(
        DhcpHosts::new(&static_pool, &block, &DhcpConfig::default(), Duration::ZERO),
        Err(DhcpError::NotDhcp { .. })
    ));
    let other = HostPool::dhcp("other", 10, None).unwrap();
    assert!(matches!(
        DhcpHosts::new(&other, &block, &DhcpConfig::default(), Duration::ZERO),
        Err(DhcpError::BlockMismatch { .. })
    ));
    let small = HostPool::dhcp("clients", 5, Some("02:aa:00:00:00:00")).unwrap();
    assert!(matches!(
        DhcpHosts::new(&small, &block, &DhcpConfig::default(), Duration::ZERO),
        Err(DhcpError::BadSlice { .. })
    ));
    let tags = DhcpConfig {
        vlans: vec![1, 2, 3],
        ..DhcpConfig::default()
    };
    assert!(matches!(
        DhcpHosts::new(&dhcp, &block, &tags, Duration::ZERO),
        Err(DhcpError::TooManyVlans { tags: 3, .. })
    ));
}

#[test]
fn the_neighbor_table_keeps_static_and_leased_pools_apart() {
    let leased = pool(10);
    let mut alloc = MacAllocator::new();
    let block = alloc.claim(&leased, leased.range()).unwrap();
    let hosts = DhcpHosts::new(&leased, &block, &DhcpConfig::default(), Duration::ZERO).unwrap();
    let mut table = NeighborTable::new();
    // `add` takes a pool with a prefix, and says where a DHCP pool goes.
    assert!(matches!(
        table.add(&leased, &block),
        Err(NeighborError::LeasedPool { .. })
    ));
    let static_pool = HostPool::parse(&fps_hosts::PoolSpec {
        name: "servers",
        ipv4: Some("10.50.0.0/24"),
        count: 10,
        gateway: Some("10.50.0.1"),
        ..fps_hosts::PoolSpec::default()
    })
    .unwrap();
    let static_block = alloc.claim(&static_pool, static_pool.range()).unwrap();
    assert!(matches!(
        table.add_leased(&static_pool, &static_block, hosts.leases()),
        Err(NeighborError::StaticPool { .. })
    ));
    // A lease table of another slice is refused.
    let other_range = hosts_range_table(&leased, HostRange::new(0, 4));
    assert!(matches!(
        table.add_leased(&leased, &block, &other_range),
        Err(NeighborError::LeaseMismatch { .. })
    ));
    // An empty lease table adds no host, and answers for nothing.
    table.add_leased(&leased, &block, hosts.leases()).unwrap();
    assert_eq!(table.hosts(), 0);
    assert_eq!(table.lookup_v4(addr_of(0)), None);
}

fn hosts_range_table(pool: &HostPool, range: HostRange) -> fps_hosts::LeaseTable {
    let mut alloc = MacAllocator::new();
    let block = alloc.claim(pool, range).unwrap();
    DhcpHosts::new(pool, &block, &DhcpConfig::default(), Duration::ZERO)
        .unwrap()
        .leases()
        .clone()
}
