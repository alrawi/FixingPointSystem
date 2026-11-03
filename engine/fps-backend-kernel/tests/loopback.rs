// SPDX-License-Identifier: GPL-3.0-only
//! Loopback component tests of the kernel backend.
//!
//! Every test uses its own addresses in 127.0.0.0/8, so TIME_WAIT entries of
//! one test never collide with the sessions of another.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fps_backend_kernel::{
    bind_pool, ConnectConfig, Connector, KernelListener, Linger, ListenConfig, OsProfile,
    PortRange, SocketOptions, BUILTIN_PROFILES,
};
use fps_io::SessionIo;
use socket2::SockRef;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

/// Accepts sessions forever, echoes the first 8 bytes of each, and waits for
/// end of stream. It records every peer address.
fn spawn_echo_server(listener: KernelListener, peers: Arc<Mutex<Vec<SocketAddr>>>) {
    tokio::spawn(async move {
        loop {
            let (mut io, peer) = listener.accept().await.unwrap();
            peers.lock().unwrap().push(peer);
            tokio::spawn(async move {
                let mut buf = [0u8; 8];
                if io.read_exact(&mut buf).await.is_err() {
                    return;
                }
                if io.write_all(&buf).await.is_err() {
                    return;
                }
                let mut rest = Vec::new();
                let _ = io.read_to_end(&mut rest).await;
            });
        }
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_thousand_sessions_from_one_hundred_sources_without_address_errors() {
    const SESSIONS: usize = 10_000;
    const IN_FLIGHT: usize = 512;
    let servers = bind_pool(
        [v4(10, 0, 1), v4(10, 0, 2)],
        0,
        &ListenConfig {
            backlog: 4096,
            ..ListenConfig::default()
        },
    )
    .unwrap();
    let destinations: Vec<SocketAddr> = servers.iter().map(|l| l.local_addr()).collect();
    let peers = Arc::new(Mutex::new(Vec::new()));
    for listener in servers {
        spawn_echo_server(listener, Arc::clone(&peers));
    }
    let sources: Vec<IpAddr> = (1..=100).map(|i| v4(11, 0, i)).collect();
    let range = PortRange::new(40000, 44999).unwrap();
    let connector = Arc::new(Connector::new(ConnectConfig {
        local_ports: Some(range),
        ..ConnectConfig::default()
    }));

    let next = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(Mutex::new(Vec::new()));
    let mut workers = Vec::new();
    for _ in 0..IN_FLIGHT {
        let (connector, next, errors) = (connector.clone(), next.clone(), errors.clone());
        let (sources, destinations) = (sources.clone(), destinations.clone());
        workers.push(tokio::spawn(async move {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= SESSIONS {
                    return;
                }
                let source = sources[i % sources.len()];
                let destination = destinations[(i / sources.len()) % destinations.len()];
                let mut io = match connector.connect(source, destination).await {
                    Ok(io) => io,
                    Err(e) => {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("{e}: {:?}", e.io_kind()));
                        continue;
                    }
                };
                let local = io.local_addr().unwrap();
                assert_eq!(local.ip(), source);
                assert!(range.contains(local.port()), "port {}", local.port());
                let id = (i as u64).to_be_bytes();
                io.write_all(&id).await.unwrap();
                let mut echo = [0u8; 8];
                io.read_exact(&mut echo).await.unwrap();
                assert_eq!(echo, id);
                io.shutdown().await.unwrap();
                let mut rest = Vec::new();
                io.read_to_end(&mut rest).await.unwrap();
                assert!(rest.is_empty());
            }
        }));
    }
    for w in workers {
        w.await.unwrap();
    }

    let errors = errors.lock().unwrap();
    assert!(
        errors.is_empty(),
        "{} errors, first {:?}",
        errors.len(),
        errors.first()
    );
    let peers = peers.lock().unwrap();
    assert_eq!(peers.len(), SESSIONS);
    let mut per_source: HashMap<IpAddr, usize> = HashMap::new();
    for p in peers.iter() {
        *per_source.entry(p.ip()).or_default() += 1;
    }
    assert_eq!(per_source.len(), 100);
    assert!(per_source.values().all(|&n| n == SESSIONS / 100));
}

#[tokio::test]
async fn port_range_limits_sessions_per_destination_and_shares_ports_across_destinations() {
    let servers = bind_pool([v4(12, 0, 1), v4(12, 0, 2)], 0, &ListenConfig::default()).unwrap();
    let (a, b) = (servers[0].local_addr(), servers[1].local_addr());
    let range = PortRange::new(41000, 41003).unwrap();
    let connector = Connector::new(ConnectConfig {
        local_ports: Some(range),
        ..ConnectConfig::default()
    });
    let source = v4(12, 1, 1);

    let mut held = Vec::new();
    let mut ports_a = HashSet::new();
    for _ in 0..range.len() {
        let io = connector.connect(source, a).await.unwrap();
        ports_a.insert(io.local_addr().unwrap().port());
        held.push(io);
    }
    assert_eq!(ports_a.len(), 4);
    assert!(ports_a.iter().all(|p| range.contains(*p)));

    let err = connector.connect(source, a).await.unwrap_err();
    assert_eq!(
        err.io_kind(),
        Some(io::ErrorKind::AddrNotAvailable),
        "{err}"
    );

    // IP_BIND_ADDRESS_NO_PORT lets the same four ports serve a second destination.
    let mut ports_b = HashSet::new();
    for _ in 0..range.len() {
        let io = connector.connect(source, b).await.unwrap();
        ports_b.insert(io.local_addr().unwrap().port());
        held.push(io);
    }
    assert_eq!(ports_a, ports_b);
    drop(servers);
}

#[tokio::test]
async fn client_abort_resets_the_server_and_disables_the_stream() {
    let listener =
        KernelListener::bind(SocketAddr::new(v4(13, 0, 1), 0), &ListenConfig::default()).unwrap();
    let connector = Connector::new(ConnectConfig::default());
    let mut client = connector
        .connect(v4(13, 1, 1), listener.local_addr())
        .await
        .unwrap();
    let (mut server, peer) = listener.accept().await.unwrap();
    assert_eq!(peer, client.local_addr().unwrap());

    client.write_all(b"hello").await.unwrap();
    let mut buf = [0u8; 5];
    server.read_exact(&mut buf).await.unwrap();
    server.write_all(b"unread by the client").await.unwrap();

    client.abort();
    client.abort();
    assert!(client.tcp_stream().is_none());
    let err = client.write_all(b"x").await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    let err = client.read(&mut buf).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    assert_eq!(
        client.local_addr().unwrap_err().kind(),
        io::ErrorKind::NotConnected
    );

    let err = server.read(&mut buf).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
}

#[tokio::test]
async fn server_abort_resets_the_client() {
    let listener =
        KernelListener::bind(SocketAddr::new(v4(14, 0, 1), 0), &ListenConfig::default()).unwrap();
    let connector = Connector::new(ConnectConfig::default());
    let mut client = connector
        .connect(v4(14, 1, 1), listener.local_addr())
        .await
        .unwrap();
    let (mut server, _) = listener.accept().await.unwrap();
    server.abort();
    let mut buf = [0u8; 1];
    let err = client.read(&mut buf).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
}

#[tokio::test]
async fn shutdown_sends_end_of_stream_and_keeps_the_other_direction_open() {
    let listener =
        KernelListener::bind(SocketAddr::new(v4(15, 0, 1), 0), &ListenConfig::default()).unwrap();
    let connector = Connector::new(ConnectConfig::default());
    let mut client = connector
        .connect(v4(15, 1, 1), listener.local_addr())
        .await
        .unwrap();
    let (mut server, _) = listener.accept().await.unwrap();
    client.write_all(b"last").await.unwrap();
    client.shutdown().await.unwrap();
    let mut got = Vec::new();
    server.read_to_end(&mut got).await.unwrap();
    assert_eq!(got, b"last");
    server.write_all(b"reply").await.unwrap();
    let mut buf = [0u8; 5];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"reply");
}

#[tokio::test]
async fn socket_options_reach_client_and_accepted_sockets() {
    let socket = SocketOptions {
        send_buffer: Some(64 * 1024),
        recv_buffer: Some(32 * 1024),
        nodelay: false,
        linger: Linger::Abort,
        ..SocketOptions::default()
    };
    let listener = KernelListener::bind(
        SocketAddr::new(v4(16, 0, 1), 0),
        &ListenConfig {
            backlog: 16,
            reuse_address: true,
            socket: socket.clone(),
        },
    )
    .unwrap();
    let connector = Connector::new(ConnectConfig {
        socket: socket.clone(),
        ..ConnectConfig::default()
    });
    let client = connector
        .connect(v4(16, 1, 1), listener.local_addr())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    for (side, io) in [("client", &client), ("server", &server)] {
        let sock = SockRef::from(io.tcp_stream().unwrap());
        assert!(!sock.tcp_nodelay().unwrap(), "{side} nodelay");
        assert_eq!(
            sock.linger().unwrap(),
            Some(Duration::ZERO),
            "{side} linger"
        );
        // Linux doubles the requested size for bookkeeping.
        assert!(
            sock.send_buffer_size().unwrap() >= 64 * 1024,
            "{side} sndbuf"
        );
        assert!(
            sock.recv_buffer_size().unwrap() >= 32 * 1024,
            "{side} rcvbuf"
        );
    }

    let defaults = Connector::new(ConnectConfig::default())
        .connect(v4(16, 1, 2), listener.local_addr())
        .await
        .unwrap();
    let sock = SockRef::from(defaults.tcp_stream().unwrap());
    assert!(sock.tcp_nodelay().unwrap());
    assert_eq!(sock.linger().unwrap(), None);
}

#[tokio::test]
async fn os_profiles_reach_client_and_accepted_sockets() {
    for (i, name) in BUILTIN_PROFILES.into_iter().enumerate() {
        let profile = OsProfile::builtin(name).unwrap();
        let socket = profile.socket_options(SocketOptions::default());
        let octet = u8::try_from(i).unwrap() + 1;
        let listener = KernelListener::bind(
            SocketAddr::new(v4(19, 0, octet), 0),
            &ListenConfig {
                socket: socket.clone(),
                ..ListenConfig::default()
            },
        )
        .unwrap();
        let connector = Connector::new(ConnectConfig {
            socket,
            ..ConnectConfig::default()
        });
        let client = connector
            .connect(v4(19, 1, octet), listener.local_addr())
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        for (side, io) in [("client", &client), ("server", &server)] {
            let sock = SockRef::from(io.tcp_stream().unwrap());
            assert_eq!(
                sock.ttl_v4().unwrap(),
                u32::from(profile.socket.ttl),
                "{name} {side} ttl"
            );
            // Loopback allows an MSS near 64 KiB, so the profile MSS decides.
            // Connected sockets report it less the timestamp option.
            let mss = sock.tcp_mss().unwrap();
            assert!(
                (1400..=u32::from(profile.socket.mss)).contains(&mss),
                "{name} {side} mss {mss}"
            );
        }
    }
}

#[tokio::test]
async fn refused_connect_names_source_destination_and_step() {
    // Bind and drop a listener to find a port with nothing listening.
    let port = KernelListener::bind(SocketAddr::new(v4(17, 0, 1), 0), &ListenConfig::default())
        .unwrap()
        .local_addr();
    let err = Connector::new(ConnectConfig::default())
        .connect(v4(17, 1, 1), port)
        .await
        .unwrap_err();
    assert_eq!(err.io_kind(), Some(io::ErrorKind::ConnectionRefused));
    assert_eq!(
        err.to_string(),
        format!("connect from 127.17.1.1 to {port}: connect failed")
    );
}

#[tokio::test]
async fn mismatched_address_families_fail_before_any_system_call() {
    let err = Connector::new(ConnectConfig::default())
        .connect("::1".parse().unwrap(), "127.0.0.1:80".parse().unwrap())
        .await
        .unwrap_err();
    assert!(err.io_kind().is_none());
    assert!(err.to_string().contains("address families differ"));
}

#[tokio::test]
async fn listener_reports_the_bound_address_in_bind_errors() {
    let first =
        KernelListener::bind(SocketAddr::new(v4(18, 0, 1), 0), &ListenConfig::default()).unwrap();
    let err = KernelListener::bind(
        first.local_addr(),
        &ListenConfig {
            reuse_address: false,
            ..ListenConfig::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.io_kind(), Some(io::ErrorKind::AddrInUse));
    assert_eq!(
        err.to_string(),
        format!("listen on {}: bind failed", first.local_addr())
    );
}

#[tokio::test]
async fn ipv6_loopback_session_uses_the_configured_port_range() {
    let listener = match KernelListener::bind("[::1]:0".parse().unwrap(), &ListenConfig::default())
    {
        Ok(l) => l,
        Err(e) => {
            eprintln!("skipped because IPv6 loopback is unavailable: {e}");
            return;
        }
    };
    let range = PortRange::new(42000, 42099).unwrap();
    let mut client = Connector::new(ConnectConfig {
        local_ports: Some(range),
        ..ConnectConfig::default()
    })
    .connect("::1".parse().unwrap(), listener.local_addr())
    .await
    .unwrap();
    assert!(range.contains(client.local_addr().unwrap().port()));
    let (mut server, _) = listener.accept().await.unwrap();
    client.write_all(b"v6").await.unwrap();
    let mut buf = [0u8; 2];
    server.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"v6");
}
