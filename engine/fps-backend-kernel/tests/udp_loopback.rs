// SPDX-License-Identifier: GPL-3.0-only
//! Loopback component tests of the kernel UDP sessions.
//!
//! Every test uses its own addresses in 127.0.0.0/8, so the tests of this
//! file and of `loopback.rs` never share a listener address.

use std::collections::HashSet;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fps_backend_kernel::{
    bind_datagram_pool, ConnectConfig, Connector, DatagramAcceptOpener, DatagramConnectOpener,
    KernelDatagram, KernelDatagramListener, PortRange, SocketOptions,
};
use fps_flow::function::NoFunctions;
use fps_flow::template::Filler;
use fps_flow::Scenario;
use fps_interp::{session_rng, Limits, Session, SessionCtx, Side};
use fps_io::{DatagramIo, DatagramIoExt, MAX_DATAGRAM};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

fn listener(ip: IpAddr) -> KernelDatagramListener {
    KernelDatagramListener::bind(SocketAddr::new(ip, 0), &SocketOptions::default()).unwrap()
}

async fn recv(io: &mut KernelDatagram, buf: &mut [u8]) -> io::Result<usize> {
    timeout(WAIT, io.recv_datagram(buf))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

/// Accepts sessions forever. Each session answers every datagram with `R`
/// followed by the datagram until `answers` datagrams were answered, and
/// the server records each peer.
fn spawn_answer_server(
    mut listener: KernelDatagramListener,
    answers: usize,
    peers: Arc<Mutex<Vec<SocketAddr>>>,
) {
    tokio::spawn(async move {
        loop {
            let (mut io, peer) = listener.accept().await.unwrap();
            assert_eq!(io.peer_addr(), peer);
            peers.lock().unwrap().push(peer);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                let mut reply = Vec::with_capacity(2049);
                for _ in 0..answers {
                    let Ok(n) = recv(&mut io, &mut buf).await else {
                        return;
                    };
                    reply.clear();
                    reply.push(b'R');
                    reply.extend_from_slice(&buf[..n]);
                    if io.send_datagram(&reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_thousand_request_response_pairs_from_one_hundred_sources() {
    const SOURCES: u8 = 100;
    const PER_SOURCE: usize = 100;
    let server = listener(v4(20, 0, 1));
    let destination = server.local_addr();
    let peers = Arc::new(Mutex::new(Vec::new()));
    spawn_answer_server(server, 1, Arc::clone(&peers));
    let connector = Arc::new(Connector::new(ConnectConfig {
        local_ports: Some(PortRange::new(45000, 49999).unwrap()),
        ..ConnectConfig::default()
    }));

    let mut clients = Vec::new();
    for i in 1..=SOURCES {
        let connector = Arc::clone(&connector);
        clients.push(tokio::spawn(async move {
            let source = v4(21, 0, i);
            let mut buf = [0u8; 64];
            // The previous socket stays open until the next one is bound, so
            // the kernel cannot hand the next session the port of a session
            // that the server may still hold open, which would make the two
            // sessions one five tuple.
            let mut previous = None;
            for n in 0..PER_SOURCE {
                let mut io = connector
                    .connect_datagram(source, destination)
                    .map_err(io::Error::other)?;
                assert_eq!(io.local_addr().ip(), source);
                let request = format!("q {i} {n}");
                io.send_datagram(request.as_bytes()).await?;
                let len = recv(&mut io, &mut buf).await?;
                assert_eq!(&buf[..len], format!("Rq {i} {n}").as_bytes());
                previous = Some(io);
            }
            drop(previous);
            io::Result::Ok(())
        }));
    }
    for c in clients {
        c.await.unwrap().unwrap();
    }
    let peers = peers.lock().unwrap();
    assert_eq!(peers.len(), usize::from(SOURCES) * PER_SOURCE);
    let sources: HashSet<IpAddr> = peers.iter().map(SocketAddr::ip).collect();
    assert_eq!(sources.len(), usize::from(SOURCES));
    assert!(peers.iter().all(|p| (45000..=49999).contains(&p.port())));
}

#[tokio::test]
async fn one_session_carries_many_pairs_over_the_connected_server_socket() {
    let server = listener(v4(22, 0, 1));
    let destination = server.local_addr();
    spawn_answer_server(server, 1000, Arc::default());
    let connector = Connector::new(ConnectConfig::default());
    let mut io = connector
        .connect_datagram(v4(22, 0, 2), destination)
        .unwrap();
    let mut buf = [0u8; 32];
    for n in 0..1000u32 {
        io.send_datagram(&n.to_be_bytes()).await.unwrap();
        let len = recv(&mut io, &mut buf).await.unwrap();
        assert_eq!(len, 5);
        assert_eq!(&buf[1..5], &n.to_be_bytes());
    }
}

#[tokio::test]
async fn receives_truncate_and_return_the_full_length_on_both_sides() {
    let mut server = listener(v4(23, 0, 1));
    let connector = Connector::new(ConnectConfig::default());
    let mut client = connector
        .connect_datagram(v4(23, 0, 2), server.local_addr())
        .unwrap();
    let big: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    client.send_datagram(&big).await.unwrap();
    let (mut session, _) = timeout(WAIT, server.accept()).await.unwrap().unwrap();
    // The first datagram comes from the queue that the listener filled.
    let mut small = [0u8; 100];
    assert_eq!(recv(&mut session, &mut small).await.unwrap(), 3000);
    assert_eq!(&small[..], &big[..100]);
    // Later datagrams come from the kernel socket with MSG_TRUNC.
    client.send_datagram(&big[..2000]).await.unwrap();
    assert_eq!(recv(&mut session, &mut small).await.unwrap(), 2000);
    assert_eq!(&small[..], &big[..100]);
    session.send_datagram(&big[..1500]).await.unwrap();
    let mut tiny = [0u8; 10];
    assert_eq!(recv(&mut client, &mut tiny).await.unwrap(), 1500);
    assert_eq!(&tiny[..], &big[..10]);
    // A zero length datagram keeps its boundary.
    session.send_datagram(&[]).await.unwrap();
    assert_eq!(recv(&mut client, &mut tiny).await.unwrap(), 0);
}

#[tokio::test]
async fn datagrams_that_race_the_session_connect_reach_the_session() {
    let mut server = listener(v4(24, 0, 1));
    let connector = Connector::new(ConnectConfig::default());
    let mut client = connector
        .connect_datagram(v4(24, 0, 2), server.local_addr())
        .unwrap();
    let mut other = connector
        .connect_datagram(v4(24, 0, 3), server.local_addr())
        .unwrap();
    // All of them wait on the listener socket before the first accept.
    for n in 0..5u8 {
        client.send_datagram(&[n]).await.unwrap();
    }
    other.send_datagram(b"other").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut first, peer) = timeout(WAIT, server.accept()).await.unwrap().unwrap();
    assert_eq!(peer, client.local_addr());
    let mut buf = [0u8; 16];
    for n in 0..5u8 {
        assert_eq!(recv(&mut first, &mut buf).await.unwrap(), 1);
        assert_eq!(buf[0], n);
    }
    let (mut second, peer) = timeout(WAIT, server.accept()).await.unwrap().unwrap();
    assert_eq!(peer, other.local_addr());
    let n = recv(&mut second, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"other");
    assert_eq!(server.open_sessions(), 2);

    // Each session hears only its own peer.
    client.send_datagram(b"to first").await.unwrap();
    other.send_datagram(b"to second").await.unwrap();
    let n = recv(&mut first, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"to first");
    let n = recv(&mut second, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"to second");

    // After close, the next datagram of the peer opens a new session.
    first.close();
    assert_eq!(server.open_sessions(), 1);
    client.send_datagram(b"again").await.unwrap();
    let (mut third, peer) = timeout(WAIT, server.accept()).await.unwrap().unwrap();
    assert_eq!(peer, client.local_addr());
    let n = recv(&mut third, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"again");
}

#[tokio::test]
async fn a_closed_port_reports_connection_refused() {
    // Bind and drop a socket to find a port that nobody listens on.
    let port = std::net::UdpSocket::bind((Ipv4Addr::new(127, 25, 0, 1), 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let connector = Connector::new(ConnectConfig::default());
    let mut io = connector
        .connect_datagram(v4(25, 0, 2), SocketAddr::new(v4(25, 0, 1), port))
        .unwrap();
    io.send_datagram(b"hello").await.unwrap();
    let mut buf = [0u8; 8];
    let err = recv(&mut io, &mut buf).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
}

#[tokio::test]
async fn close_and_oversized_sends_fail_with_their_kinds() {
    let server = listener(v4(26, 0, 1));
    let connector = Connector::new(ConnectConfig::default());
    let mut io = connector
        .connect_datagram(v4(26, 0, 2), server.local_addr())
        .unwrap();
    let huge = vec![0u8; MAX_DATAGRAM + 1];
    let err = io.send_datagram(&huge).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    io.send_datagram(&huge[..MAX_DATAGRAM]).await.unwrap();
    io.close();
    io.close();
    assert!(io.udp_socket().is_none());
    let err = io.send_datagram(b"x").await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    let err = io.recv_datagram(&mut [0u8; 4]).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
}

#[tokio::test]
async fn address_family_mismatch_and_pool_binding() {
    let connector = Connector::new(ConnectConfig::default());
    let err = connector
        .connect_datagram(v4(27, 0, 2), "[::1]:53".parse().unwrap())
        .unwrap_err();
    assert!(err.to_string().contains("address families differ"), "{err}");
    let pool = bind_datagram_pool(
        [v4(27, 0, 10), v4(27, 0, 11)],
        0,
        &SocketOptions {
            ttl: Some(64),
            ..SocketOptions::default()
        },
    )
    .unwrap();
    assert_eq!(pool.len(), 2);
    assert_eq!(pool[0].local_addr().ip(), v4(27, 0, 10));
    assert_ne!(pool[1].local_addr().port(), 0);
    // A second listener on the same address and port joins the reuse port
    // group, as the session sockets do.
    let again = KernelDatagramListener::bind(pool[0].local_addr(), &SocketOptions::default());
    assert!(again.is_ok());
}

#[tokio::test]
async fn ttl_reaches_client_and_server_session_sockets() {
    let options = SocketOptions {
        ttl: Some(77),
        ..SocketOptions::default()
    };
    let mut server =
        KernelDatagramListener::bind(SocketAddr::new(v4(28, 0, 1), 0), &options).unwrap();
    let connector = Connector::new(ConnectConfig {
        socket: options,
        ..ConnectConfig::default()
    });
    let mut client = connector
        .connect_datagram(v4(28, 0, 2), server.local_addr())
        .unwrap();
    client.send_datagram(b"x").await.unwrap();
    let (session, _) = timeout(WAIT, server.accept()).await.unwrap().unwrap();
    for io in [&client, &session] {
        assert_eq!(io.udp_socket().unwrap().ttl().unwrap(), 77);
    }
}

/// The DNS lookup example of the scenario contract runs through the
/// interpreter over kernel UDP sockets.
const LOOKUP: &str = include_str!("../../../contracts/examples/modules/dns/scenarios/lookup.yaml");

#[tokio::test]
async fn the_dns_lookup_example_runs_over_kernel_sockets() {
    let mut s = Scenario::parse(LOOKUP).unwrap();
    s.fold_constants().unwrap();
    let flow = &s.flows[0];
    let filler = Filler::new(42);
    let (mut cf, mut sf) = (NoFunctions, NoFunctions);
    let session = |side, functions| {
        Session::new(SessionCtx {
            vars: &s.vars,
            filler: &filler,
            flow: &flow.id,
            side,
            session_id: 7,
            rng: session_rng(42, 7, side),
            functions,
            limits: Limits::default(),
        })
    };
    let mut client = session(Side::Client, &mut cf);
    let mut server = session(Side::Server, &mut sf);

    let mut listener = listener(v4(29, 0, 1));
    let port = listener.local_addr().port();
    let connector = Connector::new(ConnectConfig::default());
    // The scenario names port 53, so the openers replace it with the port
    // of the listener, as a port override of the runtime does.
    let mut connect = DatagramConnectOpener::new(&connector, v4(29, 0, 2))
        .with_server(v4(29, 0, 1))
        .with_port(port);
    let mut accept = DatagramAcceptOpener::new(&mut listener);
    let (c, sv) = tokio::join!(
        client.run_datagram_program(&mut connect, &flow.client, std::future::pending()),
        server.run_datagram_program(
            &mut accept,
            flow.server.as_ref().unwrap(),
            std::future::pending()
        ),
    );
    c.unwrap();
    sv.unwrap();
    assert_eq!(client.stats().bytes_tx, 34);
    assert_eq!(server.stats().bytes_rx, 34);
    assert_eq!(server.stats().bytes_tx, 50);
    assert_eq!(client.stats().bytes_rx, 50);
}
