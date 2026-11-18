// SPDX-License-Identifier: GPL-3.0-only
//! The calls that a datapath needs from the own stack (TRN-33): the addresses
//! of a session, a listener that gives its port back, and one source for the
//! neighbor tables of the host pools.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use fps_io::SessionIo;
use fps_tcp_own::clock::MonotonicClock;
use fps_tcp_own::{OwnConfig, OwnStack, Pools};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

const PORT: u16 = 8080;

fn stack_with(pools: Pools) -> OwnStack {
    OwnStack::with_pools(OwnConfig::default(), pools, Arc::new(MonotonicClock::new()))
        .expect("the pools are valid")
}

fn start_driver(stack: &OwnStack) -> JoinHandle<()> {
    let driver = stack.clone();
    tokio::spawn(async move { driver.drive().await })
}

async fn within<F: std::future::Future>(body: F) -> F::Output {
    match tokio::time::timeout(Duration::from_secs(30), body).await {
        Ok(output) => output,
        Err(_) => panic!("the test did not finish within 30 seconds"),
    }
}

fn host(pools: &Pools, side: &str, i: u32) -> IpAddr {
    let slice = if side == "client" {
        &pools.client
    } else {
        &pools.server
    };
    slice.ip(i).expect("a host of the slice")
}

#[tokio::test(flavor = "current_thread")]
async fn a_session_names_both_ends_over_ipv4() {
    let pools = Pools::sized("10.1.0.0/16", 3, "10.2.0.0/16", 3, None).expect("pools");
    let (client_ip, server_ip) = (host(&pools, "client", 2), host(&pools, "server", 1));
    let stack = stack_with(pools);
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(stack.connect_hosts(2, 1, PORT), stack.accept(PORT));
        let (client, mut server) = (client.expect("connect"), server.expect("accept"));
        let local = client.local_addr().expect("client local");
        assert_eq!(local.ip(), client_ip);
        assert!(local.port() >= 10_000, "an ephemeral port, got {local}");
        let remote = SocketAddr::new(server_ip, PORT);
        assert_eq!(client.peer_addr().expect("client peer"), remote);
        assert_eq!(server.local_addr().expect("server local"), remote);
        assert_eq!(server.peer_addr().expect("server peer"), local);
        // After an abort the session has no addresses, as a closed socket.
        server.abort();
        let error = server.peer_addr().expect_err("aborted");
        assert_eq!(error.kind(), ErrorKind::NotConnected);
        assert_eq!(
            server.local_addr().expect_err("aborted").kind(),
            ErrorKind::NotConnected
        );
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_session_names_both_ends_over_ipv6() {
    let pools = Pools::sized_v6("fd00:1::/64", 2, "fd00:2::/64", 2, None).expect("pools");
    let (client_ip, server_ip) = (host(&pools, "client", 1), host(&pools, "server", 0));
    assert!(matches!(client_ip, IpAddr::V6(_)));
    let stack = stack_with(pools);
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(
            stack.connect_addrs(client_ip, server_ip, PORT),
            stack.accept(PORT)
        );
        let (client, server) = (client.expect("connect"), server.expect("accept"));
        let local = client.local_addr().expect("client local");
        assert_eq!(local.ip(), client_ip);
        assert_eq!(server.peer_addr().expect("server peer"), local);
        assert_eq!(
            server.local_addr().expect("server local"),
            SocketAddr::new(server_ip, PORT)
        );
        assert_eq!(
            client.peer_addr().expect("client peer"),
            SocketAddr::new(server_ip, PORT)
        );
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn sessions_of_the_same_host_pair_have_distinct_peers() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let mut peers = Vec::new();
        let mut keep = Vec::new();
        for _ in 0..4 {
            let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
            let (client, server) = (client.expect("connect"), server.expect("accept"));
            assert_eq!(
                server.peer_addr().expect("peer"),
                client.local_addr().expect("local")
            );
            peers.push(server.peer_addr().expect("peer"));
            keep.push((client, server));
        }
        peers.sort();
        peers.dedup();
        assert_eq!(peers.len(), 4, "four sessions, four client ports");
        assert!(peers
            .iter()
            .all(|p| p.ip() == IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_port_can_be_listened_on_again_after_unlisten() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let error = stack.listen(PORT).expect_err("a second listen");
    assert_eq!(error.kind(), ErrorKind::AddrInUse);
    assert!(
        !stack.unlisten(PORT + 1),
        "a port that was never listened on"
    );
    assert!(stack.unlisten(PORT));
    assert!(!stack.unlisten(PORT), "the second unlisten finds nothing");
    stack.listen(PORT).expect("listen again");
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
        let (mut client, mut server) = (client.expect("connect"), server.expect("accept"));
        client.write_all(b"again").await.expect("write");
        let mut buf = [0u8; 5];
        server.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"again");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_port_without_a_listener_refuses_connections() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    assert!(stack.unlisten(PORT));
    let driver = start_driver(&stack);
    within(async {
        let error = stack.connect(PORT).await.expect_err("nobody listens");
        assert_eq!(error.kind(), ErrorKind::ConnectionRefused);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn unlisten_resets_connections_that_wait_and_fails_a_waiting_accept() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    stack.listen(PORT + 1).expect("listen the other port");
    let driver = start_driver(&stack);
    within(async {
        // Three connections complete their handshake and wait to be accepted.
        let mut waiting = Vec::new();
        for _ in 0..3 {
            waiting.push(stack.connect(PORT).await.expect("connect"));
        }
        // A session of the other port, accepted before the unlisten, is not
        // touched by it.
        let (mut kept_client, mut kept) = {
            let (c, s) = tokio::join!(stack.connect(PORT + 1), stack.accept(PORT + 1));
            (c.expect("connect"), s.expect("accept"))
        };
        // An accept that waits when the port goes away.
        let waiter = {
            let stack = stack.clone();
            tokio::spawn(async move { stack.accept(PORT + 1).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(stack.unlisten(PORT));
        assert!(stack.unlisten(PORT + 1));
        let error = waiter.await.expect("task").expect_err("the port went away");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);

        for (n, mut session) in waiting.into_iter().enumerate() {
            let mut byte = [0u8; 1];
            let error = session
                .read(&mut byte)
                .await
                .expect_err("a reset of a connection that was not accepted");
            assert_eq!(error.kind(), ErrorKind::ConnectionReset, "connection {n}");
        }
        // Accepted sessions keep running after the unlisten.
        kept_client.write_all(b"still here").await.expect("write");
        let mut buf = [0u8; 10];
        kept.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"still here");
    })
    .await;
    driver.abort();
}
