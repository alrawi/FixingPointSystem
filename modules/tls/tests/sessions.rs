// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the TLS module over in memory sessions and kernel sockets.

mod common;

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use common::{parse_client_hello, stream, Dir, Log, Tap};
use fps_backend_kernel::{ConnectConfig, Connector, KernelListener, ListenConfig};
use fps_io::memory::{duplex, MemoryIo};
use fps_io::SessionIo;
use fps_module_tls::{
    CipherSuite, ClientProfile, ClientResumption, Group, LeafKey, NssKeyLog, RunAuthority,
    ServerProfile, ServerResumption, Side, TlsAcceptor, TlsConnector, TlsError, TlsSession,
    TlsVersion,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The resumed flag and SNI that the server saw per session.
type Seen = (bool, Option<String>);

const NAME: &str = "www.example.test";

struct Rig {
    connector: TlsConnector,
    acceptor: TlsAcceptor,
}

fn rig(client: &ClientProfile, server: &ServerProfile) -> Rig {
    let ca = RunAuthority::generate("fps test authority").unwrap();
    let identity = ca.issue(&[NAME], LeafKey::EcdsaP256).unwrap();
    Rig {
        connector: TlsConnector::new(client, ca.root_store().unwrap()).unwrap(),
        acceptor: TlsAcceptor::new(server, &identity).unwrap(),
    }
}

type Pair = (
    Result<TlsSession<MemoryIo>, TlsError>,
    Result<TlsSession<MemoryIo>, TlsError>,
);

/// Runs one handshake over an in memory duplex and returns both outcomes.
async fn handshake(rig: &Rig) -> Pair {
    let (c, s) = duplex(64 * 1024);
    tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s))
}

/// Sends `request` from the client, echoes it as upper case on the server, and
/// returns what the client read back.
async fn round_trip(
    mut client: TlsSession<MemoryIo>,
    mut server: TlsSession<MemoryIo>,
    request: &[u8],
) -> Vec<u8> {
    let echo = async move {
        let mut buf = vec![0u8; request.len()];
        server.read_exact(&mut buf).await.unwrap();
        buf.make_ascii_uppercase();
        server.write_all(&buf).await.unwrap();
        server.shutdown().await.unwrap();
    };
    let ask = async move {
        client.write_all(request).await.unwrap();
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.unwrap();
        reply
    };
    tokio::join!(echo, ask).1
}

fn one(version: TlsVersion) -> (ClientProfile, ServerProfile) {
    (
        ClientProfile {
            versions: vec![version],
            ..ClientProfile::default()
        },
        ServerProfile {
            versions: vec![version],
            ..ServerProfile::default()
        },
    )
}

#[tokio::test]
async fn both_versions_complete_and_report_what_they_negotiated() {
    for (version, suite, group) in [
        (TlsVersion::Tls13, "TLS_AES_256_GCM_SHA384", "x25519"),
        (
            TlsVersion::Tls12,
            "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
            "x25519",
        ),
    ] {
        let (cp, sp) = one(version);
        let rig = rig(&cp, &sp);
        let (c, s) = handshake(&rig).await;
        let (c, s) = (c.unwrap(), s.unwrap());
        for (report, side) in [(c.report(), Side::Client), (s.report(), Side::Server)] {
            assert_eq!(report.side, side);
            assert_eq!(report.version, version);
            assert_eq!(report.cipher_suite, suite);
            assert_eq!(report.group.as_deref(), Some(group));
            assert_eq!(report.server_name.as_deref(), Some(NAME), "SNI on {side}");
            assert!(!report.resumed);
        }
        assert_eq!(round_trip(c, s, b"hello tls").await, b"HELLO TLS");
    }
}

#[tokio::test]
async fn client_hello_follows_the_profile() {
    let profile = ClientProfile {
        name: "ordered".into(),
        versions: vec![TlsVersion::Tls13, TlsVersion::Tls12],
        alpn: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        groups: vec![Group::Secp256r1, Group::X25519, Group::Secp384r1],
        cipher_suites: vec![
            CipherSuite::Tls13Chacha20Poly1305Sha256,
            CipherSuite::Tls13Aes128GcmSha256,
            CipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256,
            CipherSuite::Tls12EcdheEcdsaAes128GcmSha256,
            CipherSuite::Tls12EcdheRsaAes256GcmSha384,
        ],
        ..ClientProfile::default()
    };
    let rig = rig(&profile, &ServerProfile::default());
    let log: Log = Arc::default();
    let (c, s) = duplex(64 * 1024);
    let (c, s) = (
        Tap::new(c, Dir::ClientToServer, &log),
        Tap::new(s, Dir::ServerToClient, &log),
    );
    let (c, s) = tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s));
    let (c, _s) = (c.unwrap(), s.unwrap());

    let hello = parse_client_hello(&stream(&log, Dir::ClientToServer));
    let ciphers: Vec<u16> = hello
        .ciphers
        .iter()
        .copied()
        .filter(|c| *c != 0x00ff)
        .collect();
    assert_eq!(ciphers, vec![0x1303, 0x1301, 0xcca9, 0xc02b, 0xc030]);
    assert_eq!(hello.groups, vec![23, 29, 24]);
    assert_eq!(
        hello.key_share_groups,
        vec![23],
        "the first group supplies the key share"
    );
    assert_eq!(hello.alpn, vec!["h2", "http/1.1"]);
    assert_eq!(hello.versions, vec![0x0304, 0x0303]);
    assert_eq!(hello.sni.as_deref(), Some(NAME));
    assert_eq!(c.report().cipher_suite, "TLS_CHACHA20_POLY1305_SHA256");
    assert_eq!(c.report().group.as_deref(), Some("secp256r1"));
}

#[tokio::test]
async fn sni_can_be_left_out_and_ip_addresses_never_carry_it() {
    let no_sni = ClientProfile {
        send_sni: false,
        ..ClientProfile::default()
    };
    let rig_a = rig(&no_sni, &ServerProfile::default());
    let log: Log = Arc::default();
    let (c, s) = duplex(64 * 1024);
    let (c, s) = (
        Tap::new(c, Dir::ClientToServer, &log),
        Tap::new(s, Dir::ServerToClient, &log),
    );
    let (c, s) = tokio::join!(rig_a.connector.connect(c, NAME), rig_a.acceptor.accept(s));
    let (c, s) = (c.unwrap(), s.unwrap());
    assert_eq!(
        parse_client_hello(&stream(&log, Dir::ClientToServer)).sni,
        None
    );
    assert_eq!(c.report().server_name, None);
    assert_eq!(s.report().server_name, None);

    // An IP address name never travels as SNI, and the certificate lacks it, so
    // the handshake fails at verification, which proves the name is checked.
    let rig_b = rig(&ClientProfile::default(), &ServerProfile::default());
    let log: Log = Arc::default();
    let (c, s) = duplex(64 * 1024);
    let (c, s) = (
        Tap::new(c, Dir::ClientToServer, &log),
        Tap::new(s, Dir::ServerToClient, &log),
    );
    let (c, _s) = tokio::join!(
        rig_b.connector.connect(c, "192.0.2.7"),
        rig_b.acceptor.accept(s)
    );
    assert!(matches!(
        c,
        Err(TlsError::Handshake {
            side: Side::Client,
            ..
        })
    ));
    assert_eq!(
        parse_client_hello(&stream(&log, Dir::ClientToServer)).sni,
        None
    );
}

#[tokio::test]
async fn alpn_negotiates_the_server_choice_and_a_mismatch_fails_with_an_alert() {
    let client = ClientProfile {
        alpn: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        ..ClientProfile::default()
    };
    let server = ServerProfile {
        alpn: vec![b"http/1.1".to_vec()],
        ..ServerProfile::default()
    };
    let (c, s) = handshake(&rig(&client, &server)).await;
    assert_eq!(c.unwrap().report().alpn.as_deref(), Some(&b"http/1.1"[..]));
    assert_eq!(s.unwrap().report().alpn.as_deref(), Some(&b"http/1.1"[..]));

    let server = ServerProfile {
        alpn: vec![b"imap".to_vec()],
        ..ServerProfile::default()
    };
    let (c, s) = handshake(&rig(&client, &server)).await;
    let (c, s) = (c.map(|_| ()), s.map(|_| ()));
    let (
        Err(TlsError::Handshake { alert_received, .. }),
        Err(TlsError::Handshake { alert_sent, .. }),
    ) = (&c, &s)
    else {
        panic!("both sides must fail: {c:?} {s:?}");
    };
    assert_eq!(
        alert_received.as_deref(),
        Some("no_application_protocol"),
        "{c:?}"
    );
    assert_eq!(
        alert_sent.as_deref(),
        Some("no_application_protocol"),
        "{s:?}"
    );
}

#[tokio::test]
async fn server_cipher_order_can_override_the_client_order() {
    let client = ClientProfile {
        cipher_suites: vec![
            CipherSuite::Tls13Aes128GcmSha256,
            CipherSuite::Tls13Chacha20Poly1305Sha256,
        ],
        ..ClientProfile::default()
    };
    let mut server = ServerProfile {
        cipher_suites: vec![
            CipherSuite::Tls13Chacha20Poly1305Sha256,
            CipherSuite::Tls13Aes128GcmSha256,
        ],
        ..ServerProfile::default()
    };
    let (c, _s) = handshake(&rig(&client, &server)).await;
    assert_eq!(c.unwrap().report().cipher_suite, "TLS_AES_128_GCM_SHA256");
    server.use_server_cipher_order = true;
    let (c, _s) = handshake(&rig(&client, &server)).await;
    assert_eq!(
        c.unwrap().report().cipher_suite,
        "TLS_CHACHA20_POLY1305_SHA256"
    );
}

/// Runs `count` handshakes with a round trip each, so the client reads the
/// TLS 1.3 tickets, and returns the resumed flag of each client.
async fn resumed_flags(rig: &Rig, count: usize) -> (Vec<bool>, Vec<bool>) {
    let (mut client_flags, mut server_flags) = (Vec::new(), Vec::new());
    for _ in 0..count {
        let (c, s) = handshake(rig).await;
        let (c, s) = (c.unwrap(), s.unwrap());
        client_flags.push(c.report().resumed);
        server_flags.push(s.report().resumed);
        assert_eq!(round_trip(c, s, b"ping").await, b"PING");
    }
    (client_flags, server_flags)
}

#[tokio::test]
async fn resumption_follows_the_profiles_on_both_versions() {
    for version in [TlsVersion::Tls13, TlsVersion::Tls12] {
        let (cp, sp) = one(version);
        // Tickets on the server, store on the client.
        let (c, s) = resumed_flags(&rig(&cp, &sp), 3).await;
        assert_eq!(
            (c, s),
            (vec![false, true, true], vec![false, true, true]),
            "tickets {version:?}"
        );

        // A session cache on the server works the same way.
        let cached = ServerProfile {
            resumption: ServerResumption::SessionCache { sessions: 8 },
            ..sp.clone()
        };
        let (c, _) = resumed_flags(&rig(&cp, &cached), 2).await;
        assert_eq!(c, vec![false, true], "session cache {version:?}");

        // A client without a store never resumes.
        let off = ClientProfile {
            resumption: ClientResumption::Disabled,
            ..cp.clone()
        };
        let (c, _) = resumed_flags(&rig(&off, &sp), 2).await;
        assert_eq!(c, vec![false, false], "client store off {version:?}");

        // A server without tickets or cache never resumes.
        let none = ServerProfile {
            resumption: ServerResumption::Disabled,
            ..sp.clone()
        };
        let (c, s) = resumed_flags(&rig(&cp, &none), 2).await;
        assert_eq!(
            (c, s),
            (vec![false, false], vec![false, false]),
            "server off {version:?}"
        );
    }
}

#[tokio::test]
async fn a_new_connector_starts_with_an_empty_resumption_store() {
    let (cp, sp) = one(TlsVersion::Tls13);
    let ca = RunAuthority::generate("fps test authority").unwrap();
    let identity = ca.issue(&[NAME], LeafKey::EcdsaP256).unwrap();
    let acceptor = TlsAcceptor::new(&sp, &identity).unwrap();
    let mut flags = Vec::new();
    for _ in 0..2 {
        let connector = TlsConnector::new(&cp, ca.root_store().unwrap()).unwrap();
        let (c, s) = duplex(64 * 1024);
        let (c, s) = tokio::join!(connector.connect(c, NAME), acceptor.accept(s));
        let (c, s) = (c.unwrap(), s.unwrap());
        flags.push(c.report().resumed);
        round_trip(c, s, b"x").await;
    }
    assert_eq!(flags, vec![false, false]);
}

#[tokio::test]
async fn failures_name_their_alerts() {
    // The client trusts another authority than the one that signed the server.
    let (cp, sp) = one(TlsVersion::Tls13);
    let other = RunAuthority::generate("other authority").unwrap();
    let ca = RunAuthority::generate("fps test authority").unwrap();
    let identity = ca.issue(&[NAME], LeafKey::EcdsaP256).unwrap();
    let connector = TlsConnector::new(&cp, other.root_store().unwrap()).unwrap();
    let acceptor = TlsAcceptor::new(&sp, &identity).unwrap();
    let (c, s) = duplex(64 * 1024);
    let (c, s) = tokio::join!(connector.connect(c, NAME), acceptor.accept(s));
    let Err(TlsError::Handshake {
        side,
        peer,
        alert_sent,
        source,
        ..
    }) = c
    else {
        panic!("client must fail")
    };
    assert_eq!(
        (side, peer.as_str(), alert_sent.as_deref()),
        (Side::Client, NAME, Some("unknown_ca"))
    );
    assert!(
        source.get_ref().is_some_and(|e| e.is::<rustls::Error>()),
        "the rustls error stays reachable"
    );
    let Err(TlsError::Handshake {
        side,
        alert_received,
        ..
    }) = s
    else {
        panic!("server must fail")
    };
    assert_eq!(
        (side, alert_received.as_deref()),
        (Side::Server, Some("unknown_ca"))
    );
    assert_eq!(
        TlsError::Config(String::new()).failure_reason(),
        "tls_failure"
    );

    // A client that offers only TLS 1.2 meets a server that offers only TLS 1.3.
    let (c12, _) = one(TlsVersion::Tls12);
    let (_, s13) = one(TlsVersion::Tls13);
    let (c, s) = handshake(&rig(&c12, &s13)).await;
    assert!(matches!(c, Err(TlsError::Handshake { .. })));
    assert!(matches!(s, Err(TlsError::Handshake { .. })));

    // A transport that ends in the middle of the handshake is an error, not a hang.
    let (c, s) = duplex(1024);
    drop(s);
    let rig = rig(&ClientProfile::default(), &ServerProfile::default());
    let err = rig.connector.connect(c, NAME).await.unwrap_err();
    assert!(matches!(err, TlsError::Handshake { .. }));
    let io_err: io::Error = err.into();
    assert!(matches!(
        io_err.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    ));
}

#[tokio::test]
async fn bad_profiles_are_configuration_errors() {
    let ca = RunAuthority::generate("fps test authority").unwrap();
    let none = ClientProfile {
        versions: vec![],
        ..ClientProfile::default()
    };
    assert!(matches!(
        TlsConnector::new(&none, ca.root_store().unwrap()),
        Err(TlsError::Config(_))
    ));
    // A TLS 1.3 only profile with only TLS 1.2 suites has nothing to offer.
    let empty = ClientProfile {
        versions: vec![TlsVersion::Tls13],
        cipher_suites: vec![CipherSuite::Tls12EcdheEcdsaAes128GcmSha256],
        ..ClientProfile::default()
    };
    assert!(matches!(
        TlsConnector::new(&empty, ca.root_store().unwrap()),
        Err(TlsError::Config(_))
    ));
    let connector = TlsConnector::new(&ClientProfile::default(), ca.root_store().unwrap()).unwrap();
    let (c, _s) = duplex(1024);
    assert!(matches!(
        connector.connect(c, "bad name!").await,
        Err(TlsError::Config(_))
    ));
}

#[tokio::test]
async fn key_log_lines_match_the_handshake() {
    for (version, labels) in [
        (
            TlsVersion::Tls13,
            vec![
                "CLIENT_HANDSHAKE_TRAFFIC_SECRET",
                "SERVER_HANDSHAKE_TRAFFIC_SECRET",
                "CLIENT_TRAFFIC_SECRET_0",
                "SERVER_TRAFFIC_SECRET_0",
                "EXPORTER_SECRET",
            ],
        ),
        (TlsVersion::Tls12, vec!["CLIENT_RANDOM"]),
    ] {
        let keys = Arc::new(NssKeyLog::in_memory());
        let (mut cp, mut sp) = one(version);
        cp.key_log = Some(keys.clone());
        sp.key_log = Some(keys.clone());
        let rig = rig(&cp, &sp);
        let log: Log = Arc::default();
        let (c, s) = duplex(64 * 1024);
        let (c, s) = (
            Tap::new(c, Dir::ClientToServer, &log),
            Tap::new(s, Dir::ServerToClient, &log),
        );
        let (c, s) = tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s));
        drop((c.unwrap(), s.unwrap()));

        let random: String = parse_client_hello(&stream(&log, Dir::ClientToServer))
            .random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let contents = keys.contents();
        let lines: Vec<Vec<&str>> = contents.lines().map(|l| l.split(' ').collect()).collect();
        // Client and server both log, so each label shows up for each side.
        for label in &labels {
            let hits: Vec<_> = lines.iter().filter(|l| l[0] == *label).collect();
            assert!(!hits.is_empty(), "{version:?} logs {label}");
            for l in hits {
                assert_eq!(l.len(), 3);
                assert_eq!(l[1], random, "{label} names the ClientHello random");
                assert!(
                    l[2].len() >= 64
                        && l[2].len() % 2 == 0
                        && l[2].bytes().all(|b| b.is_ascii_hexdigit())
                );
            }
        }
        assert!(keys.take_error().is_none());
    }
}

#[tokio::test]
async fn large_transfers_survive_one_byte_reads_and_back_pressure() {
    let (cp, sp) = one(TlsVersion::Tls13);
    let rig = rig(&cp, &sp);
    // A tiny pipe makes every read and write partial.
    let (c, s) = duplex(37);
    let (c, s) = tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s));
    let (c, s) = (c.unwrap(), s.unwrap());
    let request: Vec<u8> = (0..200_000u32).map(|i| b'a' + (i % 26) as u8).collect();
    let reply = round_trip(c, s, &request).await;
    assert_eq!(reply.len(), request.len());
    assert!(reply
        .iter()
        .zip(&request)
        .all(|(r, q)| *r == q.to_ascii_uppercase()));
}

#[tokio::test]
async fn shutdown_sends_close_notify_and_a_silent_end_reads_as_end_of_stream() {
    let (cp, sp) = one(TlsVersion::Tls13);
    let rig = rig(&cp, &sp);
    let (c, s) = handshake(&rig).await;
    let (mut c, mut s) = (c.unwrap(), s.unwrap());
    c.write_all(b"bye").await.unwrap();
    c.shutdown().await.unwrap();
    let mut got = Vec::new();
    s.read_to_end(&mut got).await.unwrap();
    assert_eq!(got, b"bye");

    // Dropping a session ends the stream without close_notify.
    let (c, s) = handshake(&rig).await;
    let (c, mut s) = (c.unwrap(), s.unwrap());
    drop(c);
    let mut got = Vec::new();
    assert_eq!(s.read_to_end(&mut got).await.unwrap(), 0);
}

#[tokio::test]
async fn abort_resets_the_transport_under_the_session() {
    let (cp, sp) = one(TlsVersion::Tls12);
    let (c, s) = handshake(&rig(&cp, &sp)).await;
    let (mut c, mut s) = (c.unwrap(), s.unwrap());
    c.abort();
    let mut buf = [0u8; 8];
    let err = s.read(&mut buf).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    let err = c.write_all(b"x").await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
}

#[tokio::test]
async fn sessions_run_over_kernel_tcp_with_shared_resumption() {
    let (cp, sp) = one(TlsVersion::Tls13);
    let rig = Arc::new(rig(&cp, &sp));
    let addr = |last| SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 44, 0, last)), 0);
    let listener = KernelListener::bind(addr(1), &ListenConfig::default()).unwrap();
    let target = listener.local_addr();
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();

    let server = {
        let (rig, seen) = (Arc::clone(&rig), Arc::clone(&seen));
        tokio::spawn(async move {
            for _ in 0..2 {
                let (io, _peer) = listener.accept().await.unwrap();
                let mut tls = rig.acceptor.accept(io).await.unwrap();
                seen.lock()
                    .unwrap()
                    .push((tls.report().resumed, tls.report().server_name.clone()));
                let mut buf = [0u8; 5];
                tls.read_exact(&mut buf).await.unwrap();
                tls.write_all(&buf).await.unwrap();
                tls.shutdown().await.unwrap();
            }
        })
    };

    let connector = Connector::new(ConnectConfig::default());
    let mut client_flags = Vec::new();
    for _ in 0..2 {
        let io = connector.connect(addr(2).ip(), target).await.unwrap();
        let mut tls = rig.connector.connect(io, NAME).await.unwrap();
        client_flags.push(tls.report().resumed);
        tls.write_all(b"hello").await.unwrap();
        let mut reply = Vec::new();
        tls.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"hello");
    }
    server.await.unwrap();
    assert_eq!(client_flags, vec![false, true]);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            (false, Some(NAME.to_owned())),
            (true, Some(NAME.to_owned()))
        ]
    );
}
