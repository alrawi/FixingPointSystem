// SPDX-License-Identifier: GPL-3.0-only
//! The datapath over in memory links. No root is needed, so these tests run in
//! every job. A real AF_XDP run over a veth pair is in the tests of
//! `fps-backend-afxdp` (`datapath_veth`).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fps_backend_afxdp::memory::{frame_pair, MemoryFrames};
use fps_backend_afxdp::FrameIo;
use fps_datapath::{
    split_pools, Class, Datapath, DatapathConfig, DatapathError, Feature, FrameHook, HookOutcome,
    Hooks, Lane, Replies, Side,
};
use fps_hosts::wire::{ns_frame, parse_neighbor_advert};
use fps_hosts::MacAddr;
use fps_tcp_own::Pools;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FRAME: usize = 1600;
const PEER_MAC: [u8; 6] = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
const PEER_IP: [u8; 4] = [192, 168, 77, 9];

fn v4(addr: u32) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(addr))
}

/// A lane over two in memory pairs: the client arm and the server arm are the
/// ends of the same pair, so the stack talks to itself, like the two arms of
/// a router that forwards everything.
fn looped_lane(pools: Pools) -> Lane {
    let (client, server) = frame_pair(512, FRAME);
    Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools,
    }
}

/// A lane whose client arm is cut open: the test holds the far end of the
/// client link and plays the neighbor.
fn tapped_lane(pools: Pools) -> (Lane, MemoryFrames) {
    let (client, tap) = frame_pair(512, FRAME);
    let (server, _unused) = frame_pair(512, FRAME);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools,
    };
    (lane, tap)
}

fn arp_request(vlan: Option<u16>, target: [u8; 4]) -> Vec<u8> {
    let mut f = vec![0xff; 6];
    f.extend_from_slice(&PEER_MAC);
    if let Some(id) = vlan {
        f.extend_from_slice(&0x8100u16.to_be_bytes());
        f.extend_from_slice(&id.to_be_bytes());
    }
    f.extend_from_slice(&0x0806u16.to_be_bytes());
    f.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
    f.extend_from_slice(&PEER_MAC);
    f.extend_from_slice(&PEER_IP);
    f.extend_from_slice(&[0; 6]);
    f.extend_from_slice(&target);
    f.resize(f.len().max(60), 0);
    f
}

fn ipv4_frame(protocol: u8, l4: &[u8]) -> Vec<u8> {
    let mut f = vec![0x02, 0, 0, 0, 0, 1];
    f.extend_from_slice(&PEER_MAC);
    f.extend_from_slice(&0x0800u16.to_be_bytes());
    f.extend_from_slice(&[0x45, 0, 0, 0, 0, 0, 0, 0, 64, protocol, 0, 0]);
    f.extend_from_slice(&PEER_IP);
    f.extend_from_slice(&[10, 0, 0, 1]);
    f.extend_from_slice(l4);
    f
}

fn udp(src: u16, dst: u16) -> Vec<u8> {
    let mut u = src.to_be_bytes().to_vec();
    u.extend_from_slice(&dst.to_be_bytes());
    u.extend_from_slice(&[0, 12, 0, 0, 1, 2, 3, 4]);
    u
}

fn nd_frame() -> Vec<u8> {
    let mut f = vec![0x33, 0x33, 0, 0, 0, 1];
    f.extend_from_slice(&PEER_MAC);
    f.extend_from_slice(&0x86ddu16.to_be_bytes());
    let mut ip = vec![0x60, 0, 0, 0, 0, 24, 58, 255];
    ip.resize(40, 0);
    // A router advertisement: ND, but not a solicitation for a host.
    ip.extend_from_slice(&[134, 0, 0, 0]);
    f.extend_from_slice(&ip);
    f
}

const PEER_IP6: &str = "fd00:77::9";

fn send(tap: &mut MemoryFrames, frame: &[u8]) {
    assert_eq!(tap.send_batch(&[frame]).unwrap(), 1);
}

/// Reads frames from the tap until `pick` returns something.
fn wait_for<T>(tap: &mut MemoryFrames, mut pick: impl FnMut(&[u8]) -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let mut found = None;
        tap.recv_batch(16, &mut |f| {
            if found.is_none() {
                found = pick(f);
            }
        })
        .unwrap();
        if found.is_some() {
            return found;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    done()
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).unwrap_or(0) ^ seed)
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn sessions_complete_on_two_lanes_and_the_splitters_count_tcp() {
    let pools = Pools::counts(8, 8, None).unwrap();
    let lanes: Vec<Lane> = split_pools(&pools, 2)
        .unwrap()
        .into_iter()
        .map(looped_lane)
        .collect();
    let slices = split_pools(&pools, 2).unwrap();
    let dp = Datapath::start(lanes, DatapathConfig::default()).unwrap();
    assert_eq!(dp.lanes(), 2);
    let mut listener = dp.listen(9000).unwrap();
    assert_eq!(listener.port(), 9000);
    let server = tokio::spawn(async move {
        let mut served = 0u32;
        while served < 40 {
            let mut session = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = [0u8; 300];
                session.read_exact(&mut request).await.unwrap();
                let mut answer = pattern(3000, request[0]);
                answer.extend_from_slice(&request);
                session.write_all(&answer).await.unwrap();
                session.shutdown().await.unwrap();
            });
            served += 1;
        }
    });
    let connector = dp.connector();
    let mut clients = Vec::new();
    for n in 0..40u8 {
        let connector = connector.clone();
        // Hosts of lane `n % 2`, so both lanes carry sessions.
        let lane = &slices[usize::from(n % 2)];
        let source = v4(lane
            .client
            .address(u32::from(n) % lane.client.hosts())
            .unwrap());
        let destination = SocketAddr::new(v4(lane.server.address(0).unwrap()), 9000);
        clients.push(tokio::spawn(async move {
            let mut session = connector.connect(source, destination).await.unwrap();
            let request = pattern(300, n);
            session.write_all(&request).await.unwrap();
            let mut answer = Vec::new();
            session.read_to_end(&mut answer).await.unwrap();
            let mut want = pattern(3000, n);
            want.extend_from_slice(&request);
            assert_eq!(answer, want, "session {n}");
        }));
    }
    for c in clients {
        c.await.unwrap();
    }
    server.await.unwrap();
    let stats = dp.stats();
    for (n, lane) in stats.iter().enumerate() {
        assert!(
            lane.client.tcp > 0 && lane.server.tcp > 0,
            "lane {n}: {lane:?}"
        );
        assert_eq!(lane.client.malformed + lane.server.malformed, 0);
        assert_eq!(lane.client.other_dropped + lane.server.other_dropped, 0);
    }
    let reports = dp.stop();
    assert!(reports.iter().all(|r| !r.panicked));
    assert!(reports.iter().all(|r| r.stats.loop_passes > 0));
}

#[tokio::test(flavor = "current_thread")]
async fn tagged_pools_carry_sessions_and_answer_tagged_arp_only() {
    let pools = Pools::counts(2, 2, Some(100)).unwrap();
    let source = v4(pools.client.address(1).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(1).unwrap()), 9001);
    let client_ip = pools.client.address(0).unwrap().to_be_bytes();
    let dp = Datapath::start(vec![looped_lane(pools.clone())], DatapathConfig::default()).unwrap();
    let mut listener = dp.listen(9001).unwrap();
    let echo = tokio::spawn(async move {
        let mut session = listener.accept().await.unwrap();
        let mut buf = [0u8; 64];
        session.read_exact(&mut buf).await.unwrap();
        session.write_all(&buf).await.unwrap();
    });
    let mut session = dp.connector().connect(source, destination).await.unwrap();
    session.write_all(&[7; 64]).await.unwrap();
    let mut back = [0u8; 64];
    session.read_exact(&mut back).await.unwrap();
    assert_eq!(back, [7; 64]);
    echo.await.unwrap();
    drop(session);
    drop(dp);

    // Tagged ARP is answered, and a frame with another tag is dropped.
    let (lane, mut tap) = tapped_lane(pools);
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();
    send(&mut tap, &arp_request(Some(101), client_ip));
    send(&mut tap, &arp_request(None, client_ip));
    send(&mut tap, &arp_request(Some(100), client_ip));
    let reply = wait_for(&mut tap, |f| {
        (f.get(12..14) == Some(&[0x81, 0x00][..])).then(|| f.to_vec())
    })
    .expect("a tagged reply");
    assert_eq!(
        reply.get(14..16),
        Some(&[0x00, 100][..]),
        "the reply keeps the tag"
    );
    assert!(wait_until(|| dp.stats()[0].client.vlan_dropped == 2));
    assert_eq!(dp.stats()[0].client.arp_replied, 1);
}

#[test]
fn arp_replies_carry_the_mac_that_the_stack_uses() {
    let pools = Pools::counts(4, 4, None).unwrap();
    let host0 = pools.client.address(0).unwrap();
    let unknown = [172, 16, 0, 1];
    let (lane, mut tap) = tapped_lane(pools.clone());
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();

    // A connect makes the stack send a SYN, whose source MAC is the MAC of
    // the client host.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let destination = SocketAddr::new(v4(pools.server.address(0).unwrap()), 80);
    let connector = dp.connector();
    let source = v4(host0);
    runtime.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(50),
            connector.connect(source, destination),
        )
        .await;
    });
    let syn_mac = wait_for(&mut tap, |f| {
        (f.get(12..14) == Some(&[0x08, 0x00][..])).then(|| f.get(6..12).map(<[u8]>::to_vec))?
    })
    .expect("a SYN from the client host");

    send(&mut tap, &arp_request(None, unknown));
    send(&mut tap, &arp_request(None, host0.to_be_bytes()));
    let reply = wait_for(&mut tap, |f| {
        (f.get(12..14) == Some(&[0x08, 0x06][..])).then(|| f.to_vec())
    })
    .expect("an ARP reply");
    assert_eq!(reply.get(20..22), Some(&[0, 2][..]), "an ARP reply");
    assert_eq!(
        reply.get(22..28),
        Some(&syn_mac[..]),
        "sender MAC of the reply"
    );
    assert_eq!(reply.get(6..12), Some(&syn_mac[..]));
    assert_eq!(reply.get(0..6), Some(&PEER_MAC[..]));
    assert_eq!(
        reply.get(38..42),
        Some(&PEER_IP[..]),
        "addressed to the asker"
    );
    assert!(wait_until(|| {
        let c = dp.stats()[0].client;
        c.arp_replied == 1 && c.arp_ignored == 1 && c.replies_sent == 1
    }));
}

#[test]
fn nd_dhcp_udp_and_other_frames_are_counted_as_unsupported() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let (lane, mut tap) = tapped_lane(pools);
    // With the UDP layer off, UDP frames reach the UDP hook, which answers
    // unsupported.
    let config = DatapathConfig {
        udp: None,
        ..DatapathConfig::default()
    };
    let dp = Datapath::start(vec![lane], config).unwrap();
    send(&mut tap, &ipv4_frame(17, &udp(5000, 53)));
    send(&mut tap, &ipv4_frame(17, &udp(5001, 53)));
    send(&mut tap, &ipv4_frame(17, &udp(68, 67)));
    send(&mut tap, &nd_frame());
    send(&mut tap, &ipv4_frame(1, &[8, 0, 0, 0, 0, 0, 0, 0]));
    let mut odd = vec![0u8; 60];
    odd[12..14].copy_from_slice(&0x88b5u16.to_be_bytes());
    send(&mut tap, &odd);
    send(&mut tap, &[1, 2, 3]);
    assert!(wait_until(|| dp.stats()[0].client.rx_frames == 7));
    let c = dp.stats()[0].client;
    assert_eq!(
        (c.udp_unsupported, c.dhcp_unsupported, c.nd_unsupported),
        (2, 1, 1)
    );
    assert_eq!((c.other_dropped, c.malformed, c.tcp), (2, 1, 0));
    assert_eq!(c.udp_handled + c.dhcp_handled + c.nd_handled, 0);
}

#[test]
fn udp_frames_reach_the_udp_layer_and_dhcp_and_nd_stay_unsupported() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let (lane, mut tap) = tapped_lane(pools);
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();
    // The test frames carry no valid checksums, so the layer counts them.
    send(&mut tap, &ipv4_frame(17, &udp(5000, 53)));
    send(&mut tap, &ipv4_frame(17, &udp(5001, 53)));
    send(&mut tap, &ipv4_frame(17, &udp(68, 67)));
    send(&mut tap, &nd_frame());
    assert!(wait_until(|| dp.stats()[0].client.rx_frames == 4));
    let lane = &dp.stats()[0];
    let c = lane.client;
    assert_eq!(
        (
            c.udp_handled,
            c.udp_unsupported,
            c.dhcp_unsupported,
            c.nd_unsupported
        ),
        (2, 0, 1, 1)
    );
    assert_eq!(
        lane.udp_client.rx_frames, 2,
        "the layer saw the two UDP frames"
    );
    assert_eq!(lane.udp_client.rx_malformed, 2, "their length field is 0");
    assert_eq!(lane.udp_server.rx_frames, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn unsupported_calls_say_what_is_missing() {
    let pools = Pools::pair();
    let dp = Datapath::start(vec![looped_lane(pools.clone())], DatapathConfig::default()).unwrap();
    let connector = dp.connector();
    let v6dst: SocketAddr = "[2001:db8::1]:80".parse().unwrap();
    let v4dst = SocketAddr::new(v4(pools.server.address(0).unwrap()), 80);
    let known = v4(pools.client.address(0).unwrap());
    // The pools have no IPv6 addresses, so IPv6 endpoints are unknown hosts.
    let err = connector
        .connect("2001:db8::2".parse().unwrap(), v6dst)
        .await;
    assert!(matches!(err, Err(DatapathError::UnknownSource(_))));
    assert!(matches!(
        connector.connect_udp("2001:db8::2".parse().unwrap(), v6dst),
        Err(DatapathError::UnknownSource(_))
    ));
    // Mixed families are not a pair of hosts of one lane.
    assert!(matches!(
        connector.connect(known, v6dst).await,
        Err(DatapathError::UnknownDestination(_))
    ));
    // With the UDP layer off, the UDP calls say so.
    let off = Datapath::start(
        vec![looped_lane(pools.clone())],
        DatapathConfig {
            udp: None,
            ..DatapathConfig::default()
        },
    )
    .unwrap();
    assert!(matches!(
        off.connector().connect_udp(known, v4dst),
        Err(DatapathError::Unsupported(Feature::Udp))
    ));
    assert!(matches!(
        off.listen_udp(53),
        Err(DatapathError::Unsupported(Feature::Udp))
    ));
    let message = DatapathError::Unsupported(Feature::Udp).to_string();
    assert!(
        message.contains("UDP") && message.contains("TRN-29"),
        "{message}"
    );
    // With the layer on, the calls check their addresses and ports.
    let nobody = v4(u32::from(Ipv4Addr::new(10, 9, 9, 9)));
    assert!(matches!(
        connector.connect_udp(nobody, v4dst),
        Err(DatapathError::UnknownSource(_))
    ));
    assert!(matches!(
        connector.connect_udp(known, SocketAddr::new(nobody, 53)),
        Err(DatapathError::UnknownDestination(_))
    ));
    assert!(matches!(
        connector.connect_udp(known, SocketAddr::new(v4dst.ip(), 0)),
        Err(DatapathError::Stack {
            op: "connect_udp",
            ..
        })
    ));
    assert!(connector.connect_udp(known, v4dst).is_ok());
    let _udp_first = dp.listen_udp(53).unwrap();
    assert!(matches!(
        dp.listen_udp(53),
        Err(DatapathError::Stack {
            op: "listen_udp",
            ..
        })
    ));
    assert!(matches!(
        dp.listen_udp(0),
        Err(DatapathError::Stack {
            op: "listen_udp",
            ..
        })
    ));
    let unknown_source = v4(u32::from(Ipv4Addr::new(10, 9, 9, 9)));
    assert!(matches!(
        connector.connect(unknown_source, v4dst).await,
        Err(DatapathError::UnknownSource(_))
    ));
    let other = SocketAddr::new(v4(u32::from(Ipv4Addr::new(10, 9, 9, 9))), 80);
    assert!(matches!(
        connector.connect(known, other).await,
        Err(DatapathError::UnknownDestination(_))
    ));
    let _first = dp.listen(80).unwrap();
    assert!(matches!(dp.listen(80), Err(DatapathError::Stack { .. })));
    assert!(matches!(
        Datapath::start(Vec::new(), DatapathConfig::default()),
        Err(DatapathError::NoLanes)
    ));
}

/// A UDP hook that answers every frame with a frame of its own.
struct Pong(Arc<AtomicU64>);

impl FrameHook for Pong {
    fn handle(&mut self, frame: &[u8], replies: &mut Replies<'_>) -> HookOutcome {
        self.0.fetch_add(1, Ordering::Relaxed);
        let mut reply = frame.to_vec();
        if let Some(dst) = reply.get_mut(0..6) {
            dst.copy_from_slice(&PEER_MAC);
        }
        replies.push(&reply);
        HookOutcome::Handled
    }
}

#[test]
fn a_hook_takes_frames_and_its_replies_leave_on_the_same_queue() {
    let seen = Arc::new(AtomicU64::new(0));
    let hook_seen = Arc::clone(&seen);
    let config = DatapathConfig {
        // The hook below replaces the UDP layer.
        udp: None,
        hooks: Some(Arc::new(move |_lane, side| {
            let mut hooks = Hooks::default();
            if side == Side::Client {
                hooks.udp = Box::new(Pong(Arc::clone(&hook_seen)));
            }
            hooks
        })),
        ..DatapathConfig::default()
    };
    let (lane, mut tap) = tapped_lane(Pools::pair());
    let dp = Datapath::start(vec![lane], config).unwrap();
    let probe = ipv4_frame(17, &udp(4000, 4001));
    send(&mut tap, &probe);
    let reply = wait_for(&mut tap, |f| (f.len() == probe.len()).then(|| f.to_vec()))
        .expect("the reply of the hook");
    assert_eq!(reply.get(14..), probe.get(14..));
    let c = dp.stats()[0].client;
    assert_eq!((c.udp_handled, c.udp_unsupported), (1, 0));
    assert_eq!(seen.load(Ordering::Relaxed), 1);
    assert_eq!(fps_datapath::classify(&probe).class, Class::Ipv4Udp);
}

#[test]
fn pools_split_into_lanes_that_cover_every_host_once() {
    let pools = Pools::counts(10, 6, None).unwrap();
    let parts = split_pools(&pools, 3).unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts.iter().map(|p| p.client.hosts()).sum::<u32>(), 10);
    assert_eq!(parts.iter().map(|p| p.server.hosts()).sum::<u32>(), 6);
    let mut seen: Vec<u32> = parts
        .iter()
        .flat_map(|p| (0..p.client.hosts()).filter_map(|i| p.client.address(i)))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 10);
    assert!(matches!(
        split_pools(&pools, 7),
        Err(DatapathError::TooManyLanes { parts: 7, hosts: 6 })
    ));
    assert!(split_pools(&pools, 0).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn ipv6_sessions_and_datagrams_complete_over_the_same_loop() {
    use fps_io::{DatagramIo, DatagramIoExt};

    let pools = Pools::sized_v6("fd00:1::/64", 4, "fd00:2::/64", 4, None).unwrap();
    let slices = split_pools(&pools, 2).unwrap();
    let lanes: Vec<Lane> = slices.iter().cloned().map(looped_lane).collect();
    let dp = Datapath::start(lanes, DatapathConfig::default()).unwrap();
    let mut listener = dp.listen(9100).unwrap();
    let mut udp_listener = dp.listen_udp(5400).unwrap();
    let tcp_server = tokio::spawn(async move {
        for _ in 0..8 {
            let mut session = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = [0u8; 200];
                session.read_exact(&mut request).await.unwrap();
                let mut answer = pattern(4000, request[0]);
                answer.extend_from_slice(&request);
                session.write_all(&answer).await.unwrap();
                session.shutdown().await.unwrap();
            });
        }
    });
    let udp_server = tokio::spawn(async move {
        for _ in 0..8 {
            let (mut session, _peer) = udp_listener.accept().await.unwrap();
            let mut buf = vec![0u8; 70_000];
            let n = session.recv_datagram(&mut buf).await.unwrap();
            buf.truncate(n);
            buf.reverse();
            session.send_datagram(&buf).await.unwrap();
        }
        udp_listener
    });
    let connector = dp.connector();
    let mut clients = Vec::new();
    for n in 0..8u8 {
        let connector = connector.clone();
        let lane = &slices[usize::from(n % 2)];
        let source = IpAddr::V6(
            lane.client
                .address6(u32::from(n) / 2 % lane.client.hosts())
                .unwrap(),
        );
        let target = IpAddr::V6(lane.server.address6(0).unwrap());
        clients.push(tokio::spawn(async move {
            let mut session = connector
                .connect(source, SocketAddr::new(target, 9100))
                .await
                .unwrap();
            let request = pattern(200, n);
            session.write_all(&request).await.unwrap();
            let mut answer = Vec::new();
            session.read_to_end(&mut answer).await.unwrap();
            let mut want = pattern(4000, n);
            want.extend_from_slice(&request);
            assert_eq!(answer, want, "tcp {n}");

            // Sizes from empty to one frame (the MTU is 1500).
            let mut dg = connector
                .connect_udp(source, SocketAddr::new(target, 5400))
                .unwrap();
            let payload = pattern(usize::from(n) * 150, n);
            dg.send_datagram(&payload).await.unwrap();
            let mut buf = vec![0u8; 70_000];
            let got = dg.recv_datagram(&mut buf).await.unwrap();
            let mut want = payload;
            want.reverse();
            assert_eq!(&buf[..got], &want[..], "udp {n}");
        }));
    }
    for c in clients {
        c.await.unwrap();
    }
    tcp_server.await.unwrap();
    let _udp_listener = udp_server.await.unwrap();
    for (n, lane) in dp.stats().iter().enumerate() {
        assert!(lane.client.tcp > 0 && lane.server.tcp > 0, "lane {n}");
        assert_eq!(lane.client.udp_handled, 4, "lane {n}");
        assert_eq!(lane.client.malformed + lane.server.malformed, 0);
        assert_eq!(lane.client.other_dropped + lane.server.other_dropped, 0);
        assert_eq!(lane.udp_client.rx_delivered, 4, "lane {n}");
    }
    // A mix of families is refused.
    assert!(matches!(
        connector
            .connect(
                IpAddr::V6(slices[0].client.address6(0).unwrap()),
                SocketAddr::new(v4(u32::from(Ipv4Addr::new(10, 1, 1, 1))), 80)
            )
            .await,
        Err(DatapathError::UnknownDestination(_))
    ));
    assert!(dp.stop().iter().all(|r| !r.panicked));
}

#[test]
fn neighbor_solicitations_for_the_host_pools_are_answered() {
    let pools = Pools::sized_v6("fd00:1::/64", 4, "fd00:2::/64", 4, None).unwrap();
    let host0 = pools.client.address6(0).unwrap();
    let unknown: std::net::Ipv6Addr = "fd00:1::ffff".parse().unwrap();
    let peer: std::net::Ipv6Addr = PEER_IP6.parse().unwrap();
    let (lane, mut tap) = tapped_lane(pools.clone());
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();

    // A connect makes the stack send a SYN, whose source MAC is the MAC of
    // the client host.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let destination = SocketAddr::new(IpAddr::V6(pools.server.address6(0).unwrap()), 80);
    let connector = dp.connector();
    runtime.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(50),
            connector.connect(IpAddr::V6(host0), destination),
        )
        .await;
    });
    let syn = wait_for(&mut tap, |f| {
        (f.get(12..14) == Some(&[0x86, 0xdd][..])).then(|| f.to_vec())
    })
    .expect("a SYN over IPv6");
    assert_eq!(syn.get(14).map(|b| b >> 4), Some(6));
    let syn_mac = syn.get(6..12).unwrap().to_vec();

    send(&mut tap, &ns_frame(MacAddr(PEER_MAC), peer, unknown, &[]));
    send(&mut tap, &ns_frame(MacAddr(PEER_MAC), peer, host0, &[]));
    let reply = wait_for(&mut tap, parse_neighbor_advert).expect("an advertisement");
    assert_eq!(reply.target, host0);
    assert_eq!(
        reply.target_lla.map(|m| m.0.to_vec()),
        Some(syn_mac.clone())
    );
    assert_eq!(
        reply.eth_src.0.to_vec(),
        syn_mac,
        "the MAC that the stack uses"
    );
    assert_eq!(reply.eth_dst.0, PEER_MAC);
    assert_eq!(reply.ip_dst, peer);
    assert_eq!(reply.hop_limit, 255);
    assert!(reply.checksum_ok);
    assert!(wait_until(|| {
        let c = dp.stats()[0].client;
        c.nd_replied == 1 && c.nd_ignored == 1 && c.nd_unsupported == 0
    }));
    // A router advertisement is not a solicitation: it goes to the hook.
    send(&mut tap, &nd_frame());
    assert!(wait_until(|| dp.stats()[0].client.nd_unsupported == 1));
}

#[test]
fn ipv6_frames_are_classified_for_the_stack_and_the_hooks() {
    let mut ip = vec![0x60, 0, 0, 0, 0, 8, 6, 64];
    ip.resize(40, 0);
    ip.extend_from_slice(&[0; 20]);
    let mut f = vec![0u8; 12];
    f.extend_from_slice(&0x86ddu16.to_be_bytes());
    f.extend_from_slice(&ip);
    assert_eq!(fps_datapath::classify(&f).class, Class::Ipv6Tcp);
    f[14 + 6] = 17;
    assert_eq!(fps_datapath::classify(&f).class, Class::Ipv6Udp);
    f[14 + 6] = 58;
    f[14 + 40] = 128;
    assert_eq!(fps_datapath::classify(&f).class, Class::Ipv6Other);
}
