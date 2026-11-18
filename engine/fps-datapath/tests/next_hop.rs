// SPDX-License-Identifier: GPL-3.0-only
//! Next hop resolution of the datapath over in memory links (TRN-35). No
//! root is needed, so these tests run in every job. The same resolution on
//! veth pairs, behind a kernel router, is in `router.rs`.
//!
//! The splitter tests drive one queue and play the neighbor by hand. The
//! datapath tests put a router between the two arms of a lane: a thread that
//! answers ARP and neighbor solicitation for the gateway of each side and
//! forwards what it is sent, so the stack reaches its peers only through the
//! gateway.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use fps_backend_afxdp::memory::{frame_pair, MemoryFrames};
use fps_backend_afxdp::FrameIo;
use fps_datapath::{
    Datapath, DatapathConfig, DatapathError, FrameCounters, Hooks, Lane, NeighborConfig, Route,
    SharedCounters, Splitter,
};
use fps_hosts::wire::{
    arp_frame, ethernet_payload, icmp6_checksum, parse_arp, ARP_REPLY, ARP_REQUEST, ETHERTYPE_IPV4,
    ETHERTYPE_IPV6, ICMP6_NA, ICMP6_NS, OPT_TARGET_LLA,
};
use fps_hosts::{HostPool, MacAddr, NeighborTable, PoolSpec, Responder};
use fps_tcp_own::clock::{Clock, ManualClock};
use fps_tcp_own::net::NEXT_HOP_MAC;
use fps_tcp_own::{HostSlice, Pools};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FRAME: usize = 1600;
const OWN_MAC: MacAddr = MacAddr([0x02, 0xaa, 0, 0, 0, 1]);
const GW_MAC: MacAddr = MacAddr([0x02, 0xee, 0, 0, 0, 0x0a]);
const US: i64 = 1_000_000;

fn pool(name: &str, v4: Option<&str>, v6: Option<&str>, gateway: &str, count: u32) -> HostPool {
    HostPool::parse(&PoolSpec {
        name,
        ipv4: v4,
        ipv6: v6,
        count,
        gateway: Some(gateway),
        ..PoolSpec::default()
    })
    .unwrap()
}

/// Client hosts on 10.0.0.0/24 behind the gateway 10.0.0.1, and server hosts
/// on 10.0.1.0/24 behind 10.0.1.1, or the IPv6 counterparts.
fn routed_pools(v6: bool, vlan: Option<u16>) -> Pools {
    let (client, server) = if v6 {
        (
            pool("client", None, Some("fd00:1::/64"), "fd00:1::1", 2),
            pool("server", None, Some("fd00:2::/64"), "fd00:2::1", 2),
        )
    } else {
        (
            pool("client", Some("10.0.0.0/24"), None, "10.0.0.1", 2),
            pool("server", Some("10.0.1.0/24"), None, "10.0.1.1", 2),
        )
    };
    Pools {
        client: HostSlice::whole(client),
        server: HostSlice::whole(server),
        vlan,
    }
}

fn gateway_of(slice: &HostSlice) -> IpAddr {
    slice.pool.gateway().unwrap()
}

/// An IPv4 frame with the marker as destination, from the client host to
/// `dst`, as the stack builds it.
fn stack_frame(src: [u8; 4], dst: [u8; 4], tag: u8) -> Vec<u8> {
    let mut f = NEXT_HOP_MAC.0.to_vec();
    f.extend_from_slice(&OWN_MAC.0);
    f.extend_from_slice(&[0x08, 0x00, 0x45, 0, 0, 40, 0, tag, 0, 0, 64, 6, 0, 0]);
    f.extend_from_slice(&src);
    f.extend_from_slice(&dst);
    f.resize(60, 0);
    f
}

fn drain(tap: &mut MemoryFrames) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    tap.recv_batch(256, &mut |f| out.push(f.to_vec())).unwrap();
    out
}

/// A splitter for the client arm of `pools` that resolves the next hop, and
/// the other end of its queue.
fn resolving_splitter(
    depth: usize,
    config: NeighborConfig,
) -> (
    Splitter,
    MemoryFrames,
    Arc<ManualClock>,
    Arc<SharedCounters>,
) {
    let pools = routed_pools(false, None);
    let (queue, tap) = frame_pair(depth, FRAME);
    let responder = Responder::new(pools.tables().unwrap().client);
    let (splitter, counters) = Splitter::new(Box::new(queue), responder, None, Hooks::default());
    let clock = Arc::new(ManualClock::new());
    let splitter = splitter.with_next_hop(
        config,
        Route::of(&pools.client.pool),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    (splitter, tap, clock, counters)
}

fn arp_reply_from_gateway(to: MacAddr, to_ip: [u8; 4]) -> Vec<u8> {
    arp_frame(
        ARP_REPLY,
        to,
        GW_MAC,
        &[],
        GW_MAC,
        Ipv4Addr::new(10, 0, 0, 1),
        to,
        Ipv4Addr::from(to_ip),
    )
}

#[test]
fn the_splitter_holds_a_frame_asks_for_the_gateway_and_sends_it_after_the_reply() {
    let (mut splitter, mut tap, _clock, counters) =
        resolving_splitter(64, NeighborConfig::default());
    let on_link = stack_frame([10, 0, 0, 2], [10, 0, 0, 99], 1);
    // Held counts as taken, so the stack does not offer the frame again.
    let off_link = stack_frame([10, 0, 0, 2], [10, 7, 0, 9], 2);
    assert_eq!(splitter.send_batch(&[&off_link]).unwrap(), 1);
    splitter.flush().unwrap();
    let seen = drain(&mut tap);
    assert_eq!(seen.len(), 1, "only the request is on the wire: {seen:?}");
    let ask = parse_arp(&seen[0]).expect("an ARP request");
    assert_eq!(ask.op, ARP_REQUEST);
    assert_eq!(ask.target_ip, Ipv4Addr::new(10, 0, 0, 1), "the gateway");
    assert_eq!(ask.sender_mac, OWN_MAC);

    // 10.0.0.99 is on the link, so it is its own next hop and asks for itself.
    assert_eq!(splitter.send_batch(&[&on_link]).unwrap(), 1);
    splitter.flush().unwrap();
    let seen = drain(&mut tap);
    assert_eq!(seen.len(), 1);
    assert_eq!(
        parse_arp(&seen[0]).unwrap().target_ip,
        Ipv4Addr::new(10, 0, 0, 99)
    );

    // The gateway answers: the frame for it leaves with its MAC address, and
    // the frame for the silent host stays.
    tap.send_batch(&[&arp_reply_from_gateway(OWN_MAC, [10, 0, 0, 2])])
        .unwrap();
    let mut none = |_: &[u8]| {};
    splitter.recv_batch(16, &mut none).unwrap();
    let seen = drain(&mut tap);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0][..6], GW_MAC.0);
    assert_eq!(seen[0][6..], off_link[6..]);
    let c = counters.snapshot();
    assert_eq!((c.neighbor_held, c.neighbor_released), (2, 1), "{c:?}");

    // And later frames go at once.
    let again = stack_frame([10, 0, 0, 2], [10, 7, 0, 10], 3);
    assert_eq!(splitter.send_batch(&[&again]).unwrap(), 1);
    let seen = drain(&mut tap);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0][..6], GW_MAC.0);
}

#[test]
fn a_full_queue_does_not_make_the_splitter_hold_a_frame_twice() {
    let (mut splitter, mut tap, _clock, counters) =
        resolving_splitter(2, NeighborConfig::default());
    // Resolve the gateway.
    let first = stack_frame([10, 0, 0, 2], [10, 7, 0, 1], 1);
    splitter.send_batch(&[&first]).unwrap();
    splitter.flush().unwrap();
    drain(&mut tap);
    tap.send_batch(&[&arp_reply_from_gateway(OWN_MAC, [10, 0, 0, 2])])
        .unwrap();
    let mut none = |_: &[u8]| {};
    splitter.recv_batch(16, &mut none).unwrap();
    drain(&mut tap);
    let held_before = counters.snapshot().neighbor_held;

    // Three frames for the known gateway do not fit the queue (depth 2), and
    // the fourth needs a request. The cache must not hold it while the frames
    // in front of it are refused: the stack would offer it again.
    let a = stack_frame([10, 0, 0, 2], [10, 7, 0, 2], 2);
    let b = stack_frame([10, 0, 0, 2], [10, 7, 0, 3], 3);
    let c = stack_frame([10, 0, 0, 2], [10, 7, 0, 4], 5);
    let on_link = stack_frame([10, 0, 0, 2], [10, 0, 0, 50], 4);
    assert_eq!(splitter.send_batch(&[&a, &b, &c, &on_link]).unwrap(), 2);
    assert_eq!(
        counters.snapshot().neighbor_held,
        held_before,
        "nothing held"
    );
    assert_eq!(drain(&mut tap).len(), 2);

    // The stack offers the rest again: c goes, and the frame that needs a
    // request is held once.
    assert_eq!(splitter.send_batch(&[&c, &on_link]).unwrap(), 2);
    splitter.flush().unwrap();
    assert_eq!(counters.snapshot().neighbor_held, held_before + 1);
    let wire = drain(&mut tap);
    assert_eq!(wire.len(), 2, "c and one request: {wire:?}");
    assert_eq!(wire[0][..6], GW_MAC.0);
    assert_eq!(
        parse_arp(&wire[1]).unwrap().target_ip,
        Ipv4Addr::new(10, 0, 0, 50)
    );
}

#[test]
fn the_splitter_repeats_the_request_and_gives_up() {
    let config = NeighborConfig {
        retry_interval: Duration::from_secs(1),
        max_requests: 2,
        ..NeighborConfig::default()
    };
    let (mut splitter, mut tap, clock, _counters) = resolving_splitter(64, config);
    let frame = stack_frame([10, 0, 0, 2], [10, 7, 0, 1], 1);
    splitter.send_batch(&[&frame]).unwrap();
    splitter.flush().unwrap();
    assert_eq!(drain(&mut tap).len(), 1);
    let mut none = |_: &[u8]| {};
    clock.advance_micros(US);
    // The pass that sends the repeat counts as work.
    assert_eq!(splitter.recv_batch(16, &mut none).unwrap(), 1);
    assert_eq!(drain(&mut tap).len(), 1, "the second request");
    clock.advance_micros(US);
    assert_eq!(splitter.recv_batch(16, &mut none).unwrap(), 0);
    assert!(drain(&mut tap).is_empty(), "no third request");
}

#[test]
fn a_neighbor_advertisement_resolves_an_ipv6_gateway_and_a_forged_one_does_not() {
    let pools = routed_pools(true, None);
    let (queue, mut tap) = frame_pair(64, FRAME);
    let responder = Responder::new(pools.tables().unwrap().client);
    let (splitter, counters) = Splitter::new(Box::new(queue), responder, None, Hooks::default());
    let clock = Arc::new(ManualClock::new());
    let mut splitter = splitter.with_next_hop(
        NeighborConfig::default(),
        Route::of(&pools.client.pool),
        clock as Arc<dyn Clock>,
    );
    let src: Ipv6Addr = "fd00:1::2".parse().unwrap();
    let gateway: Ipv6Addr = "fd00:1::1".parse().unwrap();
    let mut frame = NEXT_HOP_MAC.0.to_vec();
    frame.extend_from_slice(&OWN_MAC.0);
    frame.extend_from_slice(&[0x86, 0xdd, 0x60, 0, 0, 0, 0, 20, 6, 64]);
    frame.extend_from_slice(&src.octets());
    frame.extend_from_slice(&"fd00:2::9".parse::<Ipv6Addr>().unwrap().octets());
    frame.resize(74, 0);
    assert_eq!(splitter.send_batch(&[&frame]).unwrap(), 1);
    splitter.flush().unwrap();
    let wire = drain(&mut tap);
    assert_eq!(wire.len(), 1);
    let (pos, ethertype, _, _) = ethernet_payload(&wire[0]).unwrap();
    assert_eq!(ethertype, ETHERTYPE_IPV6);
    assert_eq!(wire[0][pos + 40], ICMP6_NS);
    assert_eq!(wire[0][pos + 48..pos + 64], gateway.octets());

    let mut none = |_: &[u8]| {};
    // A hop limit of 64 means that the advertisement crossed a router.
    tap.send_batch(&[&advert(src, gateway, GW_MAC, 64)])
        .unwrap();
    splitter.recv_batch(16, &mut none).unwrap();
    assert!(drain(&mut tap).is_empty(), "not believed, nothing leaves");
    // A good one.
    tap.send_batch(&[&advert(src, gateway, GW_MAC, 255)])
        .unwrap();
    splitter.recv_batch(16, &mut none).unwrap();
    let wire = drain(&mut tap);
    assert_eq!(wire.len(), 1);
    assert_eq!(wire[0][..6], GW_MAC.0);
    let c = counters.snapshot();
    assert_eq!((c.neighbor_resolved, c.neighbor_released), (1, 1));
    assert_eq!(c.neighbor_answers, 1, "{c:?}");
}

/// A solicited neighbor advertisement for `target` with the link layer
/// address `mac`, sent to `to`.
fn advert(to: Ipv6Addr, target: Ipv6Addr, mac: MacAddr, hop_limit: u8) -> Vec<u8> {
    let mut icmp = vec![ICMP6_NA, 0, 0, 0, 0x60, 0, 0, 0];
    icmp.extend_from_slice(&target.octets());
    icmp.extend_from_slice(&[OPT_TARGET_LLA, 1]);
    icmp.extend_from_slice(&mac.0);
    let csum = icmp6_checksum(target, to, &icmp);
    icmp[2..4].copy_from_slice(&csum.to_be_bytes());
    let mut f = OWN_MAC.0.to_vec();
    f.extend_from_slice(&mac.0);
    f.extend_from_slice(&[0x86, 0xdd, 0x60, 0, 0, 0]);
    f.extend_from_slice(&u16::try_from(icmp.len()).unwrap().to_be_bytes());
    f.extend_from_slice(&[58, hop_limit]);
    f.extend_from_slice(&target.octets());
    f.extend_from_slice(&to.octets());
    f.extend_from_slice(&icmp);
    f
}

// ---------------------------------------------------------------------------
// The datapath with a router between the arms
// ---------------------------------------------------------------------------

const MAC_A: MacAddr = MacAddr([0x02, 0xee, 0, 0, 0, 0x0a]);
const MAC_B: MacAddr = MacAddr([0x02, 0xee, 0, 0, 0, 0x0b]);

/// One router interface: the end of the queue, its MAC address, its address,
/// and the table of the hosts behind it.
struct Interface {
    queue: MemoryFrames,
    mac: MacAddr,
    address: IpAddr,
    hosts: NeighborTable,
}

fn host_mac(table: &NeighborTable, ip: IpAddr) -> Option<MacAddr> {
    match ip {
        IpAddr::V4(a) => table.lookup_v4(a),
        IpAddr::V6(a) => table.lookup_v6(a),
    }
}

/// What the router sends in answer to `frame`, which came in on `me`, and
/// what it forwards to the other interface (with the interface's MAC address
/// and the MAC address of the host).
fn answer(me: &Interface, other: &Interface, frame: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let (mut replies, mut forwards) = (Vec::new(), Vec::new());
    let Some((pos, ethertype, vlans, tags)) = ethernet_payload(frame) else {
        return (replies, forwards);
    };
    let tagged = &vlans[..tags];
    match ethertype {
        0x0806 => {
            if let Some(arp) = parse_arp(frame) {
                if arp.op == ARP_REQUEST && IpAddr::V4(arp.target_ip) == me.address {
                    replies.push(arp_frame(
                        ARP_REPLY,
                        arp.sender_mac,
                        me.mac,
                        tagged,
                        me.mac,
                        arp.target_ip,
                        arp.sender_mac,
                        arp.sender_ip,
                    ));
                }
            }
        }
        ETHERTYPE_IPV6
            if frame.get(pos + 6) == Some(&58) && frame.get(pos + 40) == Some(&ICMP6_NS) =>
        {
            let target: [u8; 16] = frame[pos + 48..pos + 64].try_into().unwrap();
            if IpAddr::V6(Ipv6Addr::from(target)) == me.address {
                let to: Ipv6Addr = <[u8; 16]>::try_from(&frame[pos + 8..pos + 24])
                    .unwrap()
                    .into();
                let mut na = advert(to, Ipv6Addr::from(target), me.mac, 255);
                na[..6].copy_from_slice(&frame[6..12]);
                replies.push(na);
            }
        }
        ETHERTYPE_IPV4 | ETHERTYPE_IPV6 if frame.get(..6) == Some(&me.mac.0[..]) => {
            let ip = if ethertype == ETHERTYPE_IPV4 {
                <[u8; 4]>::try_from(&frame[pos + 16..pos + 20])
                    .map(|b| IpAddr::V4(b.into()))
                    .ok()
            } else {
                <[u8; 16]>::try_from(&frame[pos + 24..pos + 40])
                    .map(|b| IpAddr::V6(b.into()))
                    .ok()
            };
            if let Some(mac) = ip.and_then(|ip| host_mac(&other.hosts, ip)) {
                let mut out = frame.to_vec();
                out[..6].copy_from_slice(&mac.0);
                out[6..12].copy_from_slice(&other.mac.0);
                forwards.push(out);
            }
        }
        _ => {}
    }
    (replies, forwards)
}

struct Router {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Router {
    fn start(mut a: Interface, mut b: Interface) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let mut idle = true;
                for from_a in [true, false] {
                    let (me, other) = if from_a { (&mut a, &b) } else { (&mut b, &a) };
                    let mut frames = Vec::new();
                    me.queue
                        .recv_batch(64, &mut |f| frames.push(f.to_vec()))
                        .unwrap();
                    idle &= frames.is_empty();
                    let mut to_me = Vec::new();
                    let mut to_other = Vec::new();
                    for f in &frames {
                        let (r, w) = answer(me, other, f);
                        to_me.extend(r);
                        to_other.extend(w);
                    }
                    for f in &to_me {
                        let _ = me.queue.send_batch(&[f]);
                    }
                    let _ = me.queue.flush();
                    if !to_other.is_empty() {
                        for f in &to_other {
                            let _ = send_to(&mut a, &mut b, !from_a, f);
                        }
                    }
                }
                if idle {
                    std::thread::sleep(Duration::from_micros(50));
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

fn send_to(
    a: &mut Interface,
    b: &mut Interface,
    to_a: bool,
    frame: &[u8],
) -> std::io::Result<usize> {
    let target = if to_a { a } else { b };
    target.queue.send_batch(&[frame])
}

impl Drop for Router {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A lane whose arms meet at a router. The stack knows its peers only
/// through the gateway of each side.
fn routed_lane(pools: &Pools) -> (Lane, Router) {
    let (client, a_queue) = frame_pair(1024, FRAME);
    let (server, b_queue) = frame_pair(1024, FRAME);
    let tables = pools.tables().unwrap();
    let a = Interface {
        queue: a_queue,
        mac: MAC_A,
        address: gateway_of(&pools.client),
        hosts: tables.client,
    };
    let b = Interface {
        queue: b_queue,
        mac: MAC_B,
        address: gateway_of(&pools.server),
        hosts: tables.server,
    };
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: pools.clone(),
    };
    (lane, Router::start(a, b))
}

fn resolving() -> DatapathConfig {
    DatapathConfig {
        neighbors: Some(NeighborConfig::default()),
        ..DatapathConfig::default()
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).unwrap_or(0) ^ seed)
        .collect()
}

async fn echo_through_the_router(pools: Pools) -> FrameCountersPair {
    let (lane, router) = routed_lane(&pools);
    let dp = Datapath::start(vec![lane], resolving()).unwrap();
    let mut listener = dp.listen(9100).unwrap();
    let server = tokio::spawn(async move {
        let (mut session, peer) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 20_000];
        session.read_exact(&mut buf).await.unwrap();
        session.write_all(&buf).await.unwrap();
        session.shutdown().await.unwrap();
        peer
    });
    let source = pools.client.ip(1).unwrap();
    let destination = SocketAddr::new(pools.server.ip(0).unwrap(), 9100);
    let mut session = dp.connector().connect(source, destination).await.unwrap();
    let request = pattern(20_000, 9);
    session.write_all(&request).await.unwrap();
    let mut answer = Vec::new();
    session.read_to_end(&mut answer).await.unwrap();
    assert_eq!(answer, request, "the echo is intact");
    let peer = server.await.unwrap();
    assert_eq!(peer.ip(), source, "the server saw the client host");
    let stats = dp.stats().remove(0);
    drop(session);
    let reports = dp.stop();
    assert!(!reports[0].panicked);
    drop(router);
    FrameCountersPair {
        client: stats.client,
        server: stats.server,
    }
}

struct FrameCountersPair {
    client: FrameCounters,
    server: FrameCounters,
}

fn check_resolved(c: &FrameCountersPair) {
    for (arm, n) in [("client", &c.client), ("server", &c.server)] {
        assert_eq!(
            n.neighbor_resolved, 1,
            "{arm} arm resolved its gateway: {n:?}"
        );
        assert!(n.neighbor_requests >= 1, "{arm}: {n:?}");
        assert_eq!(n.neighbor_failed, 0, "{arm}: {n:?}");
        assert!(n.neighbor_released >= 1, "{arm}: frames waited: {n:?}");
        assert_eq!(n.neighbor_entries, 1, "{arm}: the gateway only: {n:?}");
        assert!(n.neighbor_answers >= 1, "{arm}: {n:?}");
        assert!(n.tcp > 0, "{arm}: {n:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn tcp_reaches_a_host_behind_a_router_over_ipv4_with_no_neighbor_entry() {
    let c = echo_through_the_router(routed_pools(false, None)).await;
    check_resolved(&c);
}

#[tokio::test(flavor = "current_thread")]
async fn tcp_reaches_a_host_behind_a_router_over_ipv6_with_no_neighbor_entry() {
    let c = echo_through_the_router(routed_pools(true, None)).await;
    check_resolved(&c);
}

#[tokio::test(flavor = "current_thread")]
async fn the_requests_and_the_frames_carry_the_vlan_of_the_port() {
    let c = echo_through_the_router(routed_pools(false, Some(120))).await;
    check_resolved(&c);
}

#[tokio::test(flavor = "current_thread")]
async fn a_gateway_that_never_answers_fails_the_connect_and_is_counted() {
    let pools = routed_pools(false, None);
    let (client, _wire_a) = frame_pair(64, FRAME);
    let (server, _wire_b) = frame_pair(64, FRAME);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: pools.clone(),
    };
    let config = DatapathConfig {
        neighbors: Some(NeighborConfig {
            retry_interval: Duration::from_millis(40),
            max_requests: 3,
            failure_hold: Duration::from_secs(30),
            ..NeighborConfig::default()
        }),
        connect_timeout: Duration::from_millis(400),
        ..DatapathConfig::default()
    };
    let dp = Datapath::start(vec![lane], config).unwrap();
    let source = pools.client.ip(0).unwrap();
    // A peer that no pool serves is allowed once the datapath resolves.
    let stranger = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 16, 5, 5)), 80);
    let error = dp.connector().connect(source, stranger).await.unwrap_err();
    assert!(
        matches!(error, DatapathError::ConnectTimeout { .. }),
        "{error:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let c = loop {
        let c = dp.stats().remove(0).client;
        if c.neighbor_failed == 1 || Instant::now() > deadline {
            break c;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(c.neighbor_requests, 3, "{c:?}");
    assert_eq!((c.neighbor_failed, c.neighbor_resolved), (1, 0), "{c:?}");
    assert!(
        c.neighbor_dropped >= 1,
        "the SYN that waited was dropped: {c:?}"
    );
    assert_eq!(c.tcp, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn without_next_hop_resolution_a_peer_outside_the_pools_is_refused() {
    let pools = routed_pools(false, None);
    let (client, _wire_a) = frame_pair(64, FRAME);
    let (server, _wire_b) = frame_pair(64, FRAME);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: pools.clone(),
    };
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();
    let source = pools.client.ip(0).unwrap();
    let stranger = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 16, 5, 5)), 80);
    let error = dp.connector().connect(source, stranger).await.unwrap_err();
    assert!(
        matches!(error, DatapathError::UnknownDestination(_)),
        "{error:?}"
    );
    let c = dp.stats().remove(0).client;
    assert_eq!((c.neighbor_requests, c.neighbor_entries), (0, 0));
}
