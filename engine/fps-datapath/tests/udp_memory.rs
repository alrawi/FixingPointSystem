// SPDX-License-Identifier: GPL-3.0-only
//! UDP on the datapath over in memory links. No root is needed. The dns
//! scenarios over the datapath are in `dns_scenarios.rs`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use fps_backend_afxdp::memory::frame_pair;
use fps_datapath::{split_pools, Datapath, DatapathConfig, DatapathError, Lane, UdpConfig};
use fps_io::{DatagramIo, DatagramIoExt, MAX_DATAGRAM};
use fps_tcp_own::Pools;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn v4(addr: u32) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(addr))
}

fn looped_lane(pools: Pools, frame: usize) -> Lane {
    let (client, server) = frame_pair(512, frame);
    Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools,
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).unwrap_or(0) ^ seed)
        .collect()
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

async fn recv(s: &mut impl DatagramIo) -> Vec<u8> {
    let mut buf = vec![0u8; 70_000];
    let n = s.recv_datagram(&mut buf).await.unwrap();
    buf.truncate(n);
    buf
}

#[tokio::test(flavor = "current_thread")]
async fn echo_sessions_complete_on_two_lanes_with_every_size_that_fits_a_frame() {
    let pools = Pools::counts(8, 8, None).unwrap();
    let slices = split_pools(&pools, 2).unwrap();
    let lanes: Vec<Lane> = slices
        .iter()
        .cloned()
        .map(|p| looped_lane(p, 1600))
        .collect();
    let dp = Datapath::start(lanes, DatapathConfig::default()).unwrap();
    let mut listener = dp.listen_udp(5300).unwrap();
    assert_eq!(listener.port(), 5300);
    const SESSIONS: u8 = 40;
    let server = tokio::spawn(async move {
        for _ in 0..SESSIONS {
            let (mut session, peer) = listener.accept().await.unwrap();
            let request = recv(&mut session).await;
            assert_eq!(session.peer_addr(), peer);
            let mut answer = request.clone();
            answer.reverse();
            session.send_datagram(&answer).await.unwrap();
        }
        listener
    });
    let connector = dp.connector();
    let mut clients = Vec::new();
    for n in 0..SESSIONS {
        // Hosts of lane `n % 2`, so both lanes carry sessions. The sizes run
        // from empty to the largest payload of one frame.
        let lane = &slices[usize::from(n % 2)];
        let source = v4(lane
            .client
            .address(u32::from(n) % lane.client.hosts())
            .unwrap());
        let destination = SocketAddr::new(v4(lane.server.address(0).unwrap()), 5300);
        let mut session = connector.connect_udp(source, destination).unwrap();
        let request = pattern(usize::from(n) * 37 % 1473, n);
        clients.push(tokio::spawn(async move {
            session.send_datagram(&request).await.unwrap();
            let answer = recv(&mut session).await;
            let mut want = request;
            want.reverse();
            assert_eq!(answer, want, "session {n}");
        }));
    }
    for c in clients {
        c.await.unwrap();
    }
    let _listener = server.await.unwrap();
    let stats = dp.stats();
    for (n, lane) in stats.iter().enumerate() {
        assert_eq!(lane.udp_client.tx_datagrams, 20, "lane {n}");
        assert_eq!(lane.udp_server.tx_datagrams, 20, "lane {n}");
        assert_eq!(lane.udp_client.rx_delivered, 20, "lane {n}");
        assert_eq!(lane.udp_server.rx_accepted, 20, "lane {n}");
        assert_eq!(lane.client.udp_handled, 20, "lane {n}");
        assert_eq!(lane.client.udp_unsupported + lane.server.udp_unsupported, 0);
        assert_eq!(lane.client.malformed + lane.server.malformed, 0);
    }
    assert!(dp.stop().iter().all(|r| !r.panicked));
}

#[tokio::test(flavor = "current_thread")]
async fn udp_and_tcp_share_a_lane_and_a_port_number() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let source = v4(pools.client.address(1).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(1).unwrap()), 7777);
    let dp = Datapath::start(vec![looped_lane(pools, 1600)], DatapathConfig::default()).unwrap();
    let mut tcp = dp.listen(7777).unwrap();
    let mut udp = dp.listen_udp(7777).unwrap();
    let tcp_server = tokio::spawn(async move {
        let mut s = tcp.accept().await.unwrap();
        let mut buf = [0u8; 32];
        s.read_exact(&mut buf).await.unwrap();
        s.write_all(&buf).await.unwrap();
        tcp
    });
    let udp_server = tokio::spawn(async move {
        let (mut s, _) = udp.accept().await.unwrap();
        let d = recv(&mut s).await;
        s.send_datagram(&d).await.unwrap();
        udp
    });
    let connector = dp.connector();
    let mut stream = connector.connect(source, destination).await.unwrap();
    let mut datagram = connector.connect_udp(source, destination).unwrap();
    stream.write_all(&[5; 32]).await.unwrap();
    datagram.send_datagram(&[6; 33]).await.unwrap();
    let mut back = [0u8; 32];
    stream.read_exact(&mut back).await.unwrap();
    assert_eq!(back, [5; 32]);
    assert_eq!(recv(&mut datagram).await, vec![6; 33]);
    let _ = (tcp_server.await.unwrap(), udp_server.await.unwrap());
    let lane = dp.stats().remove(0);
    assert!(lane.client.tcp > 0 && lane.server.tcp > 0);
    assert_eq!(lane.udp_client.rx_delivered, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn datagrams_over_the_mtu_are_fragmented_and_not_reassembled() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let source = v4(pools.client.address(0).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(0).unwrap()), 5301);
    let dp = Datapath::start(vec![looped_lane(pools, 1600)], DatapathConfig::default()).unwrap();
    let mut listener = dp.listen_udp(5301).unwrap();
    let mut client = dp.connector().connect_udp(source, destination).unwrap();
    client.send_datagram(&pattern(5000, 1)).await.unwrap();
    client.send_datagram(&pattern(100, 2)).await.unwrap();
    assert!(wait_until(|| dp.stats()[0].udp_server.rx_fragments == 4));
    let (mut s, _) = listener.accept().await.unwrap();
    assert_eq!(
        recv(&mut s).await,
        pattern(100, 2),
        "only the datagram that fit"
    );
    let lane = dp.stats().remove(0);
    assert_eq!(lane.udp_client.tx_fragmented, 1);
    assert_eq!(lane.udp_client.tx_frames, 5, "four fragments and one frame");
    assert_eq!(lane.udp_server.rx_delivered, 1);
    // The largest datagram that DatagramIo allows can still be sent.
    client
        .send_datagram(&vec![0u8; MAX_DATAGRAM])
        .await
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn the_udp_mtu_is_lowered_to_what_the_queue_carries() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let source = v4(pools.client.address(0).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(0).unwrap()), 5302);
    // Queues of 1,000 bytes per frame: 986 bytes of IP at most.
    let dp = Datapath::start(vec![looped_lane(pools, 1000)], DatapathConfig::default()).unwrap();
    let mut listener = dp.listen_udp(5302).unwrap();
    let mut client = dp.connector().connect_udp(source, destination).unwrap();
    client.send_datagram(&pattern(900, 3)).await.unwrap();
    client.send_datagram(&pattern(1200, 4)).await.unwrap();
    assert!(wait_until(|| dp.stats()[0].udp_server.rx_fragments == 2));
    let (mut s, _) = listener.accept().await.unwrap();
    assert_eq!(recv(&mut s).await, pattern(900, 3));
    let lane = dp.stats().remove(0);
    assert_eq!(lane.udp_client.tx_frames, 3);
    assert_eq!(lane.client.replies_dropped, 0, "no frame was too long");
}

#[tokio::test(flavor = "current_thread")]
async fn udp_ports_are_freed_when_a_listener_or_a_session_goes() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let source = v4(pools.client.address(0).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(0).unwrap()), 5303);
    let dp = Datapath::start(vec![looped_lane(pools, 1600)], DatapathConfig::default()).unwrap();
    let first = dp.listen_udp(5303).unwrap();
    assert!(matches!(
        dp.listen_udp(5303),
        Err(DatapathError::Stack {
            op: "listen_udp",
            ..
        })
    ));
    drop(first);
    let mut again = dp.listen_udp(5303).unwrap();
    let mut session = dp.connector().connect_udp(source, destination).unwrap();
    assert_eq!(session.local_addr().ip(), source);
    assert_eq!(session.peer_addr(), destination);
    session.send_datagram(b"hello").await.unwrap();
    let (mut accepted, peer) = again.accept().await.unwrap();
    assert_eq!(peer, session.local_addr());
    assert_eq!(recv(&mut accepted).await, b"hello");
    assert_eq!(dp.stats()[0].udp_client.sessions_open, 1);
    session.close();
    assert_eq!(dp.stats()[0].udp_client.sessions_open, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn the_udp_settings_reach_the_layer() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let source = v4(pools.client.address(0).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(0).unwrap()), 5304);
    let config = DatapathConfig {
        udp: Some(UdpConfig {
            ephemeral_ports: (4000, 4001),
            ..UdpConfig::default()
        }),
        ..DatapathConfig::default()
    };
    let dp = Datapath::start(vec![looped_lane(pools, 1600)], config).unwrap();
    let connector = dp.connector();
    let a = connector.connect_udp(source, destination).unwrap();
    let b = connector.connect_udp(source, destination).unwrap();
    assert_eq!((a.local_addr().port(), b.local_addr().port()), (4000, 4001));
    assert!(matches!(
        connector.connect_udp(source, destination),
        Err(DatapathError::Stack {
            op: "connect_udp",
            ..
        })
    ));
}
