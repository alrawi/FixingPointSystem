// SPDX-License-Identifier: GPL-3.0-only
//! The listener of the datapath (TRN-33): `accept` returns the peer address,
//! the session knows both ends, and a port is free again when its listener is
//! dropped.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use fps_backend_afxdp::memory::frame_pair;
use fps_datapath::{split_pools, Datapath, DatapathConfig, DatapathError, Lane};
use fps_tcp_own::Pools;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FRAME: usize = 1600;

fn v4(addr: u32) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(addr))
}

fn looped_lane(pools: Pools) -> Lane {
    let (client, server) = frame_pair(512, FRAME);
    Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn accept_returns_the_peer_and_the_session_knows_both_ends() {
    let pools = Pools::counts(3, 3, None).unwrap();
    let source = v4(pools.client.address(2).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(1).unwrap()), 9100);
    let dp = Datapath::start(vec![looped_lane(pools)], DatapathConfig::default()).unwrap();
    let mut listener = dp.listen(9100).unwrap();
    let connector = dp.connector();
    let (accepted, client) =
        tokio::join!(listener.accept(), connector.connect(source, destination));
    let ((server, peer), client) = (accepted.unwrap(), client.unwrap());
    assert_eq!(peer.ip(), source, "the peer is the client host");
    assert_eq!(client.local_addr().unwrap(), peer);
    assert_eq!(client.peer_addr().unwrap(), destination);
    assert_eq!(server.local_addr().unwrap(), destination);
    assert_eq!(server.peer_addr().unwrap(), peer);
    drop((server, client, listener));
    assert!(dp.stop().iter().all(|r| !r.panicked));
}

#[tokio::test(flavor = "current_thread")]
async fn a_port_is_free_again_when_its_listener_is_dropped() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let source = v4(pools.client.address(0).unwrap());
    let destination = SocketAddr::new(v4(pools.server.address(0).unwrap()), 9101);
    let lanes = split_pools(&pools, 2)
        .unwrap()
        .into_iter()
        .map(looped_lane)
        .collect();
    let dp = Datapath::start(lanes, DatapathConfig::default()).unwrap();
    let connector = dp.connector();
    for round in 0..3 {
        let mut listener = dp.listen(9101).unwrap();
        match dp.listen(9101) {
            Err(DatapathError::Stack { op: "listen", .. }) => {}
            other => panic!("round {round}: a second listener on the port: {other:?}"),
        }
        let (accepted, client) =
            tokio::join!(listener.accept(), connector.connect(source, destination));
        let ((mut server, _), mut client) = (accepted.unwrap(), client.unwrap());
        client.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping", "round {round}");
        // The session that was accepted outlives its listener.
        drop(listener);
        server.write_all(b"pong").await.unwrap();
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"pong", "round {round}");
        // Nobody listens now: the connection is refused.
        let refused = connector.connect(source, destination).await;
        match refused {
            Err(DatapathError::Stack {
                op: "connect",
                source,
            }) if source.kind() == ErrorKind::ConnectionRefused => {}
            other => panic!("round {round}: expected a refusal, got {other:?}"),
        }
    }
    assert!(dp.stop().iter().all(|r| !r.panicked));
}

#[tokio::test(flavor = "current_thread")]
async fn a_second_listener_fails_and_leaves_the_first_one_its_port() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let lanes = split_pools(&pools, 2)
        .unwrap()
        .into_iter()
        .map(looped_lane)
        .collect();
    let dp = Datapath::start(lanes, DatapathConfig::default()).unwrap();
    let first = dp.listen(9102).unwrap();
    // The second listen fails on the first lane, so it has nothing to undo;
    // the first listener must keep its port.
    assert!(dp.listen(9102).is_err());
    drop(first);
    // Both lanes are free again.
    let again = dp.listen(9102).unwrap();
    drop(again);
    assert!(dp.stop().iter().all(|r| !r.panicked));
}
