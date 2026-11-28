// SPDX-License-Identifier: GPL-3.0-only
//! The datapath as the `Transport` of the workers, on in memory links (AGT-25).
//!
//! No root and no device are needed: the datapath runs the own stack over
//! `fps_backend_afxdp::memory::frame_pair`, whose two ends are its client arm
//! and its server arm, and the hosts are the pools of `Pools::counts`. The
//! tests check what the agent relies on when several server workers listen on
//! one port: one datapath listener per port, sessions sorted to the call
//! whose host they reached, and a port that stays open between two waiters.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use fps_agent::transport::DatapathTransport;
use fps_backend_afxdp::memory::frame_pair;
use fps_datapath::{Datapath, DatapathConfig, Lane};
use fps_io::{DatagramIo, DatagramIoExt};
use fps_runtime::transport::Transport;
use fps_tcp_own::Pools;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Rig {
    transport: DatapathTransport,
    datapath: Arc<Datapath>,
    clients: Vec<IpAddr>,
    servers: Vec<IpAddr>,
}

fn rig() -> Rig {
    let pools = Pools::counts(4, 4, None).expect("pools");
    let ip = |a: Option<u32>| IpAddr::V4(Ipv4Addr::from(a.expect("a host")));
    let clients = (0..4).map(|i| ip(pools.client.address(i))).collect();
    let servers = (0..4).map(|i| ip(pools.server.address(i))).collect();
    let (client, server) = frame_pair(512, 1600);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools,
    };
    let datapath = Arc::new(Datapath::start(vec![lane], DatapathConfig::default()).expect("start"));
    let transport = DatapathTransport::new(Arc::clone(&datapath)).expect("transport");
    Rig {
        transport,
        datapath,
        clients,
        servers,
    }
}

fn at(ip: IpAddr, port: u16) -> SocketAddr {
    SocketAddr::new(ip, port)
}

async fn within<T>(what: &str, f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), f)
        .await
        .unwrap_or_else(|_| panic!("{what} took longer than 5 seconds"))
}

#[tokio::test(flavor = "current_thread")]
async fn two_workers_listen_on_one_port_and_each_gets_the_sessions_of_its_hosts() {
    let r = rig();
    let t = &r.transport;
    // Two server workers with one host each, on one port, as the agent starts
    // them for two plan workers.
    let mut first = t.listen(&[at(r.servers[0], 8080)]).expect("first listen");
    let mut second = t.listen(&[at(r.servers[1], 8080)]).expect("second listen");
    assert_eq!(first.local_addrs(), vec![at(r.servers[0], 8080)]);
    assert_eq!(t.open_ports(), (1, 0), "one datapath listener for the port");

    // Client 0 dials the host of the second worker, client 1 the first.
    let (a, b) = tokio::join!(
        t.connect(r.clients[0], at(r.servers[1], 8080)),
        t.connect(r.clients[1], at(r.servers[0], 8080)),
    );
    let (mut a, mut b) = (a.expect("connect a"), b.expect("connect b"));
    assert_eq!(a.local_addr().map(|l| l.ip()), Some(r.clients[0]));
    assert_eq!(b.peer_addr(), Some(at(r.servers[0], 8080)));

    let on_second = within("second accept", second.accept())
        .await
        .expect("accept");
    let on_first = within("first accept", first.accept())
        .await
        .expect("accept");
    assert_eq!(on_second.local_addr(), Some(at(r.servers[1], 8080)));
    assert_eq!(on_second.peer_addr().map(|p| p.ip()), Some(r.clients[0]));
    assert_eq!(on_first.local_addr(), Some(at(r.servers[0], 8080)));
    assert_eq!(on_first.peer_addr().map(|p| p.ip()), Some(r.clients[1]));

    // The sessions carry bytes both ways.
    let (mut on_first, mut on_second) = (on_first, on_second);
    a.write_all(b"to second").await.expect("write");
    b.write_all(b"to first").await.expect("write");
    let mut buf = [0u8; 9];
    within("read", on_second.read_exact(&mut buf))
        .await
        .expect("read");
    assert_eq!(&buf, b"to second");
    let mut buf = [0u8; 8];
    within("read", on_first.read_exact(&mut buf))
        .await
        .expect("read");
    assert_eq!(&buf, b"to first");
    on_first.write_all(b"pong").await.expect("write");
    let mut buf = [0u8; 4];
    within("read", b.read_exact(&mut buf)).await.expect("read");
    assert_eq!(&buf, b"pong");

    // Nothing reached the other call.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), first.accept())
            .await
            .is_err()
    );
    drop((first, second, a, b, on_first, on_second));
    r.transport.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn a_call_for_several_hosts_takes_the_sessions_of_all_of_them() {
    let r = rig();
    let t = &r.transport;
    let mut listener = t
        .listen(&[at(r.servers[2], 9000), at(r.servers[3], 9000)])
        .expect("listen");
    for (i, host) in [r.servers[3], r.servers[2]].into_iter().enumerate() {
        let _client = t
            .connect(r.clients[i], at(host, 9000))
            .await
            .expect("connect");
        let s = within("accept", listener.accept()).await.expect("accept");
        assert_eq!(s.local_addr(), Some(at(host, 9000)));
    }
    r.transport.shutdown();
}

/// Bug #688: a client that dials before the server side of its flow listens
/// meets an open port with no call for its host. The session waits for the call
/// and is not reset.
#[tokio::test(flavor = "current_thread")]
async fn a_session_that_arrives_before_its_call_goes_to_the_call_that_comes_next() {
    let r = rig();
    let t = &r.transport;
    let _first = t.listen(&[at(r.servers[0], 8080)]).expect("listen");
    let mut client = t
        .connect(r.clients[0], at(r.servers[1], 8080))
        .await
        .expect("connect before the call");
    let mut late = t.listen(&[at(r.servers[1], 8080)]).expect("late listen");
    let mut server = within("accept", late.accept()).await.expect("accept");
    client.write_all(b"ping").await.expect("write");
    let mut buf = [0u8; 4];
    within("read", server.read_exact(&mut buf))
        .await
        .expect("read");
    assert_eq!(&buf, b"ping");
    r.transport.shutdown();
}

/// The shape of `dns/tcp_fallback` 100 times: a client dials right after the
/// previous leg ended, and the server side of the flow binds a moment later.
#[tokio::test(flavor = "current_thread")]
async fn a_hundred_flows_that_dial_before_their_call_all_complete() {
    let r = rig();
    let t = &r.transport;
    let _port = t.listen(&[at(r.servers[0], 53)]).expect("listen");
    for i in 0..100u16 {
        let mut client = t
            .connect(r.clients[0], at(r.servers[1], 53))
            .await
            .unwrap_or_else(|e| panic!("flow {i}: {e}"));
        tokio::task::yield_now().await;
        let mut call = t.listen(&[at(r.servers[1], 53)]).expect("listen");
        let mut server = within("accept", call.accept())
            .await
            .unwrap_or_else(|e| panic!("flow {i}: {e}"));
        client.write_all(b"q").await.expect("write");
        let mut buf = [0u8; 1];
        within("read", server.read_exact(&mut buf))
            .await
            .unwrap_or_else(|e| panic!("flow {i}: {e}"));
        server.write_all(b"a").await.expect("answer");
        within("read", client.read_exact(&mut buf))
            .await
            .unwrap_or_else(|e| panic!("flow {i}: {e}"));
        assert_eq!(&buf, b"a");
    }
    r.transport.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn a_session_for_a_host_that_nobody_listens_for_is_reset() {
    let r = rig();
    let t = &r.transport;
    let _first = t.listen(&[at(r.servers[0], 8080)]).expect("listen");
    // The stack finishes the handshake of the port, and the hub then refuses
    // the session, because no call holds its host. The reset can reach the
    // client before it saw the handshake end, so the connect fails, or after,
    // so the first read does.
    match t.connect(r.clients[0], at(r.servers[1], 8080)).await {
        Err(e) => assert_eq!(e.kind(), io::ErrorKind::ConnectionReset, "{e}"),
        Ok(mut client) => {
            let mut buf = [0u8; 1];
            let read = within("read", client.read(&mut buf)).await;
            assert!(
                matches!(read, Err(_) | Ok(0)),
                "the session is reset, got {read:?}"
            );
        }
    }
    r.transport.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn a_host_cannot_be_listened_on_twice_and_a_port_stays_open_for_the_next_waiter() {
    let r = rig();
    let t = &r.transport;
    let first = t.listen(&[at(r.servers[0], 8080)]).expect("listen");
    let e = t
        .listen(&[at(r.servers[1], 8080), at(r.servers[0], 8080)])
        .err()
        .expect("a second call for the same host");
    assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
    // The failed call took no host.
    let second = t.listen(&[at(r.servers[1], 8080)]).expect("listen");
    drop((first, second));
    // The last waiter left. A later waiter binds the port again at once, which
    // a second `Datapath::listen` of the port would refuse.
    let mut again = t.listen(&[at(r.servers[0], 8080)]).expect("listen again");
    assert_eq!(t.open_ports(), (1, 0));
    let _client = t
        .connect(r.clients[0], at(r.servers[0], 8080))
        .await
        .expect("connect");
    within("accept", again.accept()).await.expect("accept");
    r.transport.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn a_listen_on_port_zero_gets_a_port_that_closes_with_its_call() {
    let r = rig();
    let t = &r.transport;
    let mut listener = t.listen(&[at(r.servers[0], 0)]).expect("listen");
    let bound = listener.local_addrs();
    assert_eq!(bound.len(), 1);
    assert_eq!(bound[0].ip(), r.servers[0]);
    assert_ne!(bound[0].port(), 0, "the transport chose a port");
    assert_eq!(t.open_ports(), (1, 0));

    let _client = t.connect(r.clients[0], bound[0]).await.expect("connect");
    let s = within("accept", listener.accept()).await.expect("accept");
    assert_eq!(s.local_addr(), Some(bound[0]));

    // Two calls with port 0 never share a port.
    let other = t.listen(&[at(r.servers[0], 0)]).expect("listen");
    assert_ne!(other.local_addrs()[0].port(), bound[0].port());
    assert_eq!(t.open_ports(), (2, 0));
    drop((listener, other));
    assert_eq!(t.open_ports(), (0, 0), "a port of our choice closes");
    r.transport.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn udp_sessions_are_sorted_to_the_call_of_their_host() {
    let r = rig();
    let t = &r.transport;
    let mut first = t
        .listen_datagram(&[at(r.servers[0], 5300)])
        .expect("first listen");
    let mut second = t
        .listen_datagram(&[at(r.servers[1], 5300)])
        .expect("second listen");
    assert_eq!(t.open_ports(), (0, 1), "one datapath listener for the port");

    let mut client = t
        .connect_datagram(r.clients[2], at(r.servers[1], 5300))
        .expect("connect");
    assert_eq!(client.local_addr().ip(), r.clients[2]);
    assert_eq!(client.peer_addr(), at(r.servers[1], 5300));
    client.send_datagram(b"query").await.expect("send");

    let mut session = within("accept", second.accept()).await.expect("accept");
    assert_eq!(session.local_addr(), at(r.servers[1], 5300));
    assert_eq!(session.peer_addr(), client.local_addr());
    let mut buf = [0u8; 64];
    let n = within("recv", session.recv_datagram(&mut buf))
        .await
        .expect("recv");
    assert_eq!(&buf[..n], b"query");
    session.send_datagram(b"answer").await.expect("send");
    let n = within("recv", client.recv_datagram(&mut buf))
        .await
        .expect("recv");
    assert_eq!(&buf[..n], b"answer");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), first.accept())
            .await
            .is_err(),
        "the other call saw nothing"
    );

    // A datagram for a host that nobody listens for is dropped, and the port
    // stays served.
    let mut stray = t
        .connect_datagram(r.clients[0], at(r.servers[3], 5300))
        .expect("connect");
    stray.send_datagram(b"stray").await.expect("send");
    stray.close();
    let mut again = t
        .connect_datagram(r.clients[0], at(r.servers[0], 5300))
        .expect("connect");
    again.send_datagram(b"next").await.expect("send");
    let s = within("accept", first.accept()).await.expect("accept");
    assert_eq!(s.local_addr(), at(r.servers[0], 5300));
    r.transport.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_ends_the_listeners_and_refuses_new_ones() {
    let r = rig();
    let t = &r.transport;
    let mut listener = t.listen(&[at(r.servers[0], 8080)]).expect("listen");
    let mut udp = t
        .listen_datagram(&[at(r.servers[0], 5300)])
        .expect("listen");
    r.transport.shutdown();
    let e = within("accept", listener.accept())
        .await
        .expect_err("an error");
    assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
    let e = within("accept", udp.accept()).await.expect_err("an error");
    assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
    let e = t.listen(&[at(r.servers[1], 80)]).err().expect("refused");
    assert_eq!(e.kind(), io::ErrorKind::ConnectionAborted);
    r.transport.shutdown();
    drop((listener, udp));
    // The datapath is still ours to stop: the transport let go of it.
    let reports = Arc::try_unwrap(r.datapath).expect("no other handle").stop();
    assert!(reports.iter().all(|l| !l.panicked));
}

#[tokio::test(flavor = "current_thread")]
async fn a_call_must_name_one_port_and_at_least_one_address() {
    let r = rig();
    let t = &r.transport;
    let e = t.listen(&[]).err().expect("refused");
    assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    let e = t
        .listen(&[at(r.servers[0], 80), at(r.servers[1], 81)])
        .err()
        .expect("refused");
    assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    // A source that is not a client host of the datapath is an input error.
    let e = t
        .connect(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            at(r.servers[0], 80),
        )
        .await
        .expect_err("refused");
    assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    r.transport.shutdown();
}
