// SPDX-License-Identifier: GPL-3.0-only
//! The UDP layer between two arms in memory: the client arm and the server arm
//! of one pair of host pools, with the frames of one handed to the other.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use fps_io::{DatagramIo, DatagramIoExt, MAX_DATAGRAM};
use fps_tcp_own::Pools;
use fps_udp_own::wire;
use fps_udp_own::{UdpArm, UdpConfig, UdpListener, UdpSession};

struct Pair {
    client: UdpArm,
    server: UdpArm,
    pools: Pools,
}

fn pair_with(pools: Pools, config: &UdpConfig) -> Pair {
    let built = pools.stations().unwrap();
    Pair {
        client: UdpArm::new(built.client, config.clone()),
        server: UdpArm::new(built.server, config.clone()),
        pools,
    }
}

fn pair() -> Pair {
    pair_with(Pools::counts(4, 4, None).unwrap(), &UdpConfig::default())
}

impl Pair {
    fn client_ip(&self, i: u32) -> Ipv4Addr {
        Ipv4Addr::from(self.pools.client.address(i).unwrap())
    }

    fn server_addr(&self, i: u32, port: u16) -> SocketAddr {
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::from(self.pools.server.address(i).unwrap())),
            port,
        )
    }

    /// Hands every waiting frame to the other arm until both are quiet, and
    /// returns the frames that moved.
    fn shuttle(&self) -> usize {
        let mut total = 0;
        loop {
            let mut moved = 0;
            moved += self.client.drain_tx(64, &mut |f| self.server.receive(&f));
            moved += self.server.drain_tx(64, &mut |f| self.client.receive(&f));
            if moved == 0 {
                return total;
            }
            total += moved;
        }
    }
}

async fn recv(s: &mut UdpSession) -> Vec<u8> {
    let mut buf = vec![0u8; 70_000];
    let n = s.recv_datagram(&mut buf).await.unwrap();
    buf.truncate(n);
    buf
}

#[tokio::test(flavor = "current_thread")]
async fn a_request_and_a_response_cross_the_two_arms() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    assert_eq!(listener.port(), 53);
    let mut client = p
        .client
        .connect(p.client_ip(1), p.server_addr(2, 53))
        .unwrap();
    assert_eq!(client.peer_addr(), p.server_addr(2, 53));
    assert_eq!(client.local_addr().ip(), IpAddr::V4(p.client_ip(1)));
    client.send_datagram(b"query one").await.unwrap();
    p.shuttle();

    let (mut server, peer) = listener.accept().await.unwrap();
    assert_eq!(peer, client.local_addr());
    assert_eq!(server.local_addr(), p.server_addr(2, 53));
    assert_eq!(server.peer_addr(), client.local_addr());
    assert_eq!(
        recv(&mut server).await,
        b"query one",
        "queued for the first receive"
    );

    server.send_datagram(b"answer one").await.unwrap();
    p.shuttle();
    assert_eq!(recv(&mut client).await, b"answer one");

    // A second datagram of the same pair stays in the open session.
    client.send_datagram(b"query two").await.unwrap();
    p.shuttle();
    assert_eq!(recv(&mut server).await, b"query two");
    let (cs, ss) = (p.client.stats(), p.server.stats());
    assert_eq!((cs.tx_datagrams, ss.tx_datagrams), (2, 1));
    assert_eq!((ss.rx_accepted, ss.rx_delivered), (1, 2));
    assert_eq!(
        (cs.rx_delivered, cs.sessions_open, ss.sessions_open),
        (1, 1, 1)
    );
    assert_eq!(cs.tx_bytes, 18);
}

#[tokio::test(flavor = "current_thread")]
async fn frames_carry_the_macs_of_the_pools_valid_checksums_and_padding() {
    let p = pair();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(3, 7000))
        .unwrap();
    client.send_datagram(b"x").await.unwrap();
    let mut frames = Vec::new();
    p.client.drain_tx(8, &mut |f| frames.push(f));
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert_eq!(f.len(), 60, "padded to the shortest Ethernet frame");
    let built = Pools::counts(4, 4, None).unwrap().stations().unwrap();
    let src = built.client.local_mac(u32::from(p.client_ip(0))).unwrap();
    let dst = built
        .client
        .resolve(u32::from(Ipv4Addr::from(
            p.pools.server.address(3).unwrap(),
        )))
        .unwrap();
    assert_eq!(&f[0..6], &dst.0, "destination MAC from the neighbor table");
    assert_eq!(&f[6..12], &src.0, "source MAC of the client host");
    assert_eq!(&f[12..14], &[0x08, 0x00]);
    let parsed = wire::parse(&f[14..]).unwrap();
    assert_eq!(parsed.payload, b"x");
    assert_eq!(parsed.dst_port, 7000);
    assert_eq!(parsed.src_ip, p.client_ip(0));
}

#[tokio::test(flavor = "current_thread")]
async fn tagged_pools_send_and_accept_tagged_frames_only() {
    let p = pair_with(
        Pools::counts(2, 2, Some(100)).unwrap(),
        &UdpConfig::default(),
    );
    let mut listener = UdpListener::bind(&p.server, 9).unwrap();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 9))
        .unwrap();
    client.send_datagram(b"tagged").await.unwrap();
    let mut frames = Vec::new();
    p.client.drain_tx(8, &mut |f| frames.push(f));
    assert_eq!(&frames[0][12..16], &[0x81, 0x00, 0x00, 100]);
    assert_eq!(&frames[0][16..18], &[0x08, 0x00]);
    p.server.receive(&frames[0]);
    let (mut s, _) = listener.accept().await.unwrap();
    assert_eq!(recv(&mut s).await, b"tagged");
    // The reply is tagged too.
    s.send_datagram(b"back").await.unwrap();
    let mut back = Vec::new();
    p.server.drain_tx(8, &mut |f| back.push(f));
    assert_eq!(&back[0][12..16], &[0x81, 0x00, 0x00, 100]);
    p.client.receive(&back[0]);
    assert_eq!(recv(&mut client).await, b"back");
}

#[tokio::test(flavor = "current_thread")]
async fn ephemeral_ports_run_per_host_in_the_configured_range_and_wrap() {
    let config = UdpConfig {
        ephemeral_ports: (5000, 5002),
        ..UdpConfig::default()
    };
    let p = pair_with(Pools::counts(4, 4, None).unwrap(), &config);
    let dst = p.server_addr(0, 53);
    let port = |s: &UdpSession| s.local_addr().port();
    let a = p.client.connect(p.client_ip(0), dst).unwrap();
    let b = p.client.connect(p.client_ip(0), dst).unwrap();
    let c = p.client.connect(p.client_ip(0), dst).unwrap();
    assert_eq!((port(&a), port(&b), port(&c)), (5000, 5001, 5002));
    // Another host starts at the first port of its own.
    let other = p.client.connect(p.client_ip(1), dst).unwrap();
    assert_eq!(port(&other), 5000);
    // The same host to another server host reuses a port: the tuple differs.
    let elsewhere = p
        .client
        .connect(p.client_ip(0), p.server_addr(1, 53))
        .unwrap();
    assert_eq!(port(&elsewhere), 5000);
    // Every port toward `dst` is taken.
    let err = p.client.connect(p.client_ip(0), dst).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AddrInUse);
    // A closed session frees its port, and the cursor wraps to it.
    drop(b);
    let again = p.client.connect(p.client_ip(0), dst).unwrap();
    assert_eq!(port(&again), 5001);
    assert_eq!(p.client.stats().sessions_open, 5);
}

#[test]
fn default_ephemeral_ports_follow_the_linux_range() {
    let p = pair();
    let s = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    assert_eq!(s.local_addr().port(), 32768);
}

#[tokio::test(flavor = "current_thread")]
async fn connect_refuses_what_it_cannot_serve() {
    let p = pair();
    let dst = p.server_addr(0, 53);
    let kind = |r: std::io::Result<UdpSession>| r.unwrap_err().kind();
    assert_eq!(
        kind(p.client.connect(Ipv4Addr::new(9, 9, 9, 9), dst)),
        ErrorKind::AddrNotAvailable
    );
    assert_eq!(
        kind(
            p.client
                .connect(p.client_ip(0), "9.9.9.9:53".parse().unwrap())
        ),
        ErrorKind::HostUnreachable
    );
    assert_eq!(
        kind(p.client.connect(p.client_ip(0), p.server_addr(0, 0))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(
            p.client
                .connect(p.client_ip(0), "[2001:db8::1]:53".parse().unwrap())
        ),
        ErrorKind::Unsupported
    );
    assert_eq!(
        UdpListener::bind(&p.server, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let _first = UdpListener::bind(&p.server, 53).unwrap();
    assert_eq!(
        UdpListener::bind(&p.server, 53).unwrap_err().kind(),
        ErrorKind::AddrInUse
    );
}

#[tokio::test(flavor = "current_thread")]
async fn closing_a_session_lets_the_next_datagram_open_a_new_one() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    client.send_datagram(b"one").await.unwrap();
    p.shuttle();
    let (mut first, _) = listener.accept().await.unwrap();
    assert_eq!(recv(&mut first).await, b"one");
    first.close();
    first.close();
    let mut buf = [0u8; 8];
    assert_eq!(
        first.recv_datagram(&mut buf).await.unwrap_err().kind(),
        ErrorKind::NotConnected
    );
    assert_eq!(
        first.send_datagram(b"late").await.unwrap_err().kind(),
        ErrorKind::NotConnected
    );
    assert_eq!(p.server.stats().sessions_open, 0);
    client.send_datagram(b"two").await.unwrap();
    p.shuttle();
    let (mut second, peer) = listener.accept().await.unwrap();
    assert_eq!(peer, client.local_addr());
    assert_eq!(recv(&mut second).await, b"two");
    assert_eq!(p.server.stats().rx_accepted, 2);
    // Dropping a session closes it as well.
    drop(second);
    assert_eq!(p.server.stats().sessions_open, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn a_long_datagram_is_cut_to_the_buffer_and_reports_its_length() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    client.send_datagram(&[9u8; 900]).await.unwrap();
    client.send_datagram(b"next").await.unwrap();
    p.shuttle();
    let (mut s, _) = listener.accept().await.unwrap();
    let mut small = [0u8; 100];
    assert_eq!(s.recv_datagram(&mut small).await.unwrap(), 900);
    assert_eq!(small, [9u8; 100]);
    assert_eq!(
        recv(&mut s).await,
        b"next",
        "the rest of the long one is gone"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_session_receives_only_from_its_peer() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut a = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    let mut b = p
        .client
        .connect(p.client_ip(1), p.server_addr(0, 53))
        .unwrap();
    a.send_datagram(b"from a").await.unwrap();
    b.send_datagram(b"from b").await.unwrap();
    p.shuttle();
    let (mut sa, pa) = listener.accept().await.unwrap();
    let (mut sb, pb) = listener.accept().await.unwrap();
    assert_eq!((pa, pb), (a.local_addr(), b.local_addr()));
    assert_eq!(recv(&mut sa).await, b"from a");
    assert_eq!(recv(&mut sb).await, b"from b");
    sb.send_datagram(b"to b").await.unwrap();
    p.shuttle();
    assert_eq!(recv(&mut b).await, b"to b");
    // Nothing reached `a`.
    let mut buf = [0u8; 8];
    let waiting = tokio::time::timeout(
        std::time::Duration::from_millis(20),
        a.recv_datagram(&mut buf),
    )
    .await;
    assert!(waiting.is_err(), "a has nothing to read");
}

#[tokio::test(flavor = "current_thread")]
async fn a_datagram_that_exceeds_the_mtu_leaves_as_fragments_and_is_not_heard() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    // 1,472 bytes fill an MTU of 1,500 exactly: one frame.
    client.send_datagram(&[1u8; 1472]).await.unwrap();
    assert_eq!(p.client.stats().tx_frames, 1);
    client.send_datagram(&[2u8; 1473]).await.unwrap();
    let s = p.client.stats();
    assert_eq!((s.tx_frames, s.tx_fragmented), (3, 1));
    p.shuttle();
    let srv = p.server.stats();
    assert_eq!(
        srv.rx_fragments, 2,
        "both fragments are counted and dropped"
    );
    assert_eq!(srv.rx_delivered, 1, "only the datagram that fit one frame");
    let (mut s, _) = listener.accept().await.unwrap();
    assert_eq!(recv(&mut s).await.len(), 1472);
    // The largest datagram is accepted for sending; one byte more is not.
    client
        .send_datagram(&vec![3u8; MAX_DATAGRAM])
        .await
        .unwrap();
    assert_eq!(
        client
            .send_datagram(&vec![3u8; MAX_DATAGRAM + 1])
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_smaller_mtu_fragments_earlier() {
    let config = UdpConfig {
        mtu: 600,
        ..UdpConfig::default()
    };
    let p = pair_with(Pools::counts(2, 2, None).unwrap(), &config);
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    client.send_datagram(&[1u8; 1000]).await.unwrap();
    let mut frames = Vec::new();
    p.client.drain_tx(8, &mut |f| frames.push(f));
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|f| f.len() <= 14 + 600));
}

#[tokio::test(flavor = "current_thread")]
async fn corrupted_frames_are_counted_by_cause_and_dropped() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    client.send_datagram(&[0x42; 100]).await.unwrap();
    let mut frames = Vec::new();
    p.client.drain_tx(8, &mut |f| frames.push(f));
    let good = frames.remove(0);

    let mut bad_udp = good.clone();
    bad_udp[14 + 20 + 8 + 5] ^= 0xff;
    p.server.receive(&bad_udp);
    let mut bad_ip = good.clone();
    bad_ip[14 + 8] = 1;
    p.server.receive(&bad_ip);
    let mut short = good.clone();
    short.truncate(14 + 20 + 4);
    p.server.receive(&short);
    p.server.receive(&good[..10]);
    let mut other_host = good.clone();
    other_host[14 + 16..14 + 20].copy_from_slice(&[172, 16, 0, 1]);
    // Keep the IP header checksum valid, and send no UDP checksum, so only
    // the destination is wrong.
    other_host[14 + 20 + 6..14 + 20 + 8].copy_from_slice(&[0, 0]);
    other_host[14 + 10..14 + 12].copy_from_slice(&[0, 0]);
    let sum = wire::checksum(&other_host[14..34]).to_be_bytes();
    other_host[14 + 10..14 + 12].copy_from_slice(&sum);
    p.server.receive(&other_host);
    let s = p.server.stats();
    assert_eq!(s.rx_bad_udp_checksum, 1);
    assert_eq!(s.rx_bad_ip_checksum, 1);
    assert_eq!(s.rx_malformed, 2);
    assert_eq!(s.rx_delivered + s.rx_accepted, 0);
    assert_eq!(s.rx_not_ours, 1, "an address that is not a host of the arm");
    assert_eq!(s.rx_frames, 5);

    // A UDP checksum of zero means none was computed, and is accepted.
    let mut none = good.clone();
    none[14 + 20 + 6..14 + 20 + 8].copy_from_slice(&[0, 0]);
    p.server.receive(&none);
    let (mut s, _) = listener.accept().await.unwrap();
    assert_eq!(recv(&mut s).await, vec![0x42; 100]);
}

#[tokio::test(flavor = "current_thread")]
async fn datagrams_for_ports_without_a_listener_are_counted() {
    let p = pair();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 4444))
        .unwrap();
    client.send_datagram(b"anyone?").await.unwrap();
    p.shuttle();
    let s = p.server.stats();
    assert_eq!(
        (s.rx_no_listener, s.rx_delivered, s.sessions_open),
        (1, 0, 0)
    );
    // The client is not told: the layer sends no ICMP.
    assert_eq!(p.client.stats().rx_frames, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn full_queues_drop_and_count() {
    let config = UdpConfig {
        recv_queue: 2,
        accept_queue: 1,
        ..UdpConfig::default()
    };
    let p = pair_with(Pools::counts(4, 4, None).unwrap(), &config);
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut a = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    for n in 0..4u8 {
        a.send_datagram(&[n]).await.unwrap();
    }
    p.shuttle();
    let s = p.server.stats();
    assert_eq!((s.rx_accepted, s.rx_delivered, s.rx_queue_full), (1, 2, 2));
    // A second peer finds the accept queue full: the session is not kept.
    let mut b = p
        .client
        .connect(p.client_ip(1), p.server_addr(0, 53))
        .unwrap();
    b.send_datagram(b"late").await.unwrap();
    p.shuttle();
    let s = p.server.stats();
    assert_eq!((s.rx_accept_full, s.sessions_open), (1, 1));
    let (mut first, _) = listener.accept().await.unwrap();
    assert_eq!(recv(&mut first).await, [0]);
    assert_eq!(recv(&mut first).await, [1]);
}

#[tokio::test(flavor = "current_thread")]
async fn a_full_send_queue_holds_the_sender_until_the_loop_takes_frames() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    const N: usize = 3000;
    let sender = tokio::spawn(async move {
        for n in 0..N {
            client
                .send_datagram(&u32::try_from(n).unwrap().to_be_bytes())
                .await
                .unwrap();
        }
        client
    });
    // Let the sender run into the limit before the loop takes anything.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let held = p.client.stats();
    assert!(held.tx_blocked > 0, "the sender must have waited: {held:?}");
    assert!(held.tx_queued <= 1024 + 1);
    let mut got = Vec::new();
    let mut server: Option<UdpSession> = None;
    let mut quiet = 0;
    while quiet < 3 {
        let moved = p.shuttle();
        if server.is_none() {
            if let Ok(Ok((s, _))) =
                tokio::time::timeout(std::time::Duration::from_millis(1), listener.accept()).await
            {
                server = Some(s);
            }
        }
        let before = got.len();
        if let Some(s) = server.as_mut() {
            // The receive queue holds 64: read it dry between shuttles.
            let mut buf = [0u8; 8];
            while let Ok(Ok(n)) = tokio::time::timeout(
                std::time::Duration::from_millis(1),
                s.recv_datagram(&mut buf),
            )
            .await
            {
                got.push(u32::from_be_bytes(buf[..n].try_into().unwrap()));
            }
        }
        tokio::task::yield_now().await;
        if moved == 0 && got.len() == before && sender.is_finished() && !p.client.has_tx() {
            quiet += 1;
        } else {
            quiet = 0;
        }
    }
    let _client = sender.await.unwrap();
    // Datagrams that arrived at a full receive queue are gone, so count what
    // came and check that it is in order and that nothing was invented.
    assert!(
        got.windows(2).all(|w| w[0] < w[1]),
        "in order, no duplicates"
    );
    assert_eq!(
        u64::try_from(got.len()).unwrap() + p.server.stats().rx_queue_full,
        N as u64
    );
}

#[tokio::test(flavor = "current_thread")]
async fn one_listener_serves_the_arms_that_it_is_attached_to() {
    let p = pair();
    // A second pair stands for a second lane.
    let q = pair();
    let mut listener = UdpListener::new(123, 16);
    listener.attach(&p.server).unwrap();
    listener.attach(&q.server).unwrap();
    assert_eq!(
        listener.attach(&p.server).unwrap_err().kind(),
        ErrorKind::AddrInUse
    );
    let mut a = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 123))
        .unwrap();
    let mut b = q
        .client
        .connect(q.client_ip(0), q.server_addr(0, 123))
        .unwrap();
    a.send_datagram(b"lane p").await.unwrap();
    b.send_datagram(b"lane q").await.unwrap();
    p.shuttle();
    q.shuttle();
    let mut seen = Vec::new();
    for _ in 0..2 {
        let (mut s, _) = listener.accept().await.unwrap();
        seen.push(recv(&mut s).await);
    }
    seen.sort();
    assert_eq!(seen, vec![b"lane p".to_vec(), b"lane q".to_vec()]);
    // Dropping the listener frees the port on both arms.
    drop(listener);
    assert!(UdpListener::bind(&p.server, 123).is_ok());
    assert!(UdpListener::bind(&q.server, 123).is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn a_peer_outside_the_neighbor_table_is_answered_at_its_source_mac() {
    let p = pair();
    let mut listener = UdpListener::bind(&p.server, 53).unwrap();
    let mut client = p
        .client
        .connect(p.client_ip(0), p.server_addr(0, 53))
        .unwrap();
    client.send_datagram(b"ping").await.unwrap();
    let mut frames = Vec::new();
    p.client.drain_tx(8, &mut |f| frames.push(f));
    // The same datagram from an address that the server does not know,
    // with a source MAC of its own, as it would come from a router.
    let mut f = frames.remove(0);
    let foreign_mac = [0x02, 0xde, 0xad, 0xbe, 0xef, 0x01];
    f[6..12].copy_from_slice(&foreign_mac);
    f[14 + 12..14 + 16].copy_from_slice(&[198, 51, 100, 7]);
    f[14 + 10..14 + 12].copy_from_slice(&[0, 0]);
    let sum = wire::checksum(&f[14..34]).to_be_bytes();
    f[14 + 10..14 + 12].copy_from_slice(&sum);
    // The UDP checksum covers the source address, so clear it.
    f[14 + 26..14 + 28].copy_from_slice(&[0, 0]);
    p.server.receive(&f);
    let (mut s, peer) = listener.accept().await.unwrap();
    assert_eq!(peer.ip(), IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)));
    assert_eq!(recv(&mut s).await, b"ping");
    s.send_datagram(b"pong").await.unwrap();
    let mut back = Vec::new();
    p.server.drain_tx(8, &mut |f| back.push(f));
    assert_eq!(&back[0][0..6], &foreign_mac);
}
