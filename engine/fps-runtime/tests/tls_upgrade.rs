// SPDX-License-Identifier: GPL-3.0-only
//! The `tls_upgrade` action over the kernel backend (ENG-20).
//!
//! A loopback SMTP exchange starts in the clear (greeting, EHLO, STARTTLS,
//! `220`), upgrades both sides to TLS 1.3, and continues with a second EHLO
//! and a mail transaction over TLS. A capture records the session, and the
//! tests check on the captured bytes that
//!
//! * the first half is plaintext and the second half is not, so the secrets of
//!   the mail never cross the wire in the clear, and
//! * the key log that the TLS profiles exported decrypts the second half, which
//!   a decrypter of the test does with `ring` and no help from the module under
//!   test, and a key log with flipped digits decrypts nothing.
//!
//! Run it with `cargo test -p fps-runtime --test tls_upgrade`.

#[path = "../../fps-capture/tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use common::{fresh_dir, packets, read_pcapng, reassemble, Conn, StepClock};
use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_capture::{Capture, CaptureConfig, RingConfig, SamplePolicy};
use fps_flow::Scenario;
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_interp::Side;
use fps_module_tls::{
    ClientProfile, LeafKey, NssKeyLog, RunAuthority, ServerProfile, TlsAcceptor, TlsConnector,
    TlsVersion,
};
use fps_runtime::{
    ClientConfig, FlowRef, Role, Runtime, RuntimeConfig, ServerConfig, SourceHosts, TlsProfiles,
    WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::{metric, SeriesValue, Snapshot};
use ring::{aead, hkdf};

const SEED: u64 = 20_261_110;
const SESSIONS: u64 = 2;

/// The SMTP exchange with STARTTLS. Both sides upgrade after the `220`.
const STARTTLS: &str = r#"
scenario: smtp_starttls
flows:
  - id: submission
    module: smtp
    client:
      - connect: { port: 25 }
      - expect: { pattern: "220 mx.example.test ESMTP\r\n", timeout_ms: 5000 }
      - send: "EHLO client.example.test\r\n"
      - expect: { pattern: "250 STARTTLS\r\n", timeout_ms: 5000 }
      - send: "STARTTLS\r\n"
      - expect: { pattern: "220 2.0.0 Ready to start TLS\r\n", timeout_ms: 5000 }
      - tls_upgrade: { profile: mail, server_name: mx.example.test }
      - send: "EHLO client.example.test\r\n"
      - expect: { pattern: "250 AUTH PLAIN\r\n", timeout_ms: 5000 }
      - send: "MAIL FROM:<alice@example.test>\r\n"
      - expect: { pattern: "250 2.1.0 Ok\r\n", timeout_ms: 5000 }
      - send: "QUIT\r\n"
      - expect: { pattern: "221 2.0.0 Bye\r\n", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: 25 }
      - send: "220 mx.example.test ESMTP\r\n"
      - expect: { pattern: "EHLO client.example.test\r\n", timeout_ms: 5000 }
      - send: "250-mx.example.test\r\n250 STARTTLS\r\n"
      - expect: { pattern: "STARTTLS\r\n", timeout_ms: 5000 }
      - send: "220 2.0.0 Ready to start TLS\r\n"
      - tls_upgrade: { profile: mail }
      - expect: { pattern: "EHLO client.example.test\r\n", timeout_ms: 5000 }
      - send: "250-mx.example.test\r\n250 AUTH PLAIN\r\n"
      - expect: { pattern: "MAIL FROM:<alice@example.test>\r\n", timeout_ms: 5000 }
      - send: "250 2.1.0 Ok\r\n"
      - expect: { pattern: "QUIT\r\n", timeout_ms: 5000 }
      - send: "221 2.0.0 Bye\r\n"
      - close: fin
"#;

/// What the client sends in the clear, and what the server sends in the clear.
const CLIENT_CLEAR: &str = "EHLO client.example.test\r\nSTARTTLS\r\n";
const SERVER_CLEAR: &str = "220 mx.example.test ESMTP\r\n250-mx.example.test\r\n250 STARTTLS\r\n220 2.0.0 Ready to start TLS\r\n";

fn compile(text: &str) -> Arc<Scenario> {
    let mut s = Scenario::parse(text).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

/// TLS 1.3 profiles of one run, which export their secrets to `keys`.
fn profiles(trust_server: bool, keys: &Arc<NssKeyLog>) -> (Arc<TlsProfiles>, Arc<TlsProfiles>) {
    let ca = RunAuthority::generate("fps starttls authority").unwrap();
    let identity = ca.issue(&["mx.example.test"], LeafKey::EcdsaP256).unwrap();
    let client = ClientProfile {
        name: "mail".into(),
        versions: vec![TlsVersion::Tls13],
        key_log: Some(keys.clone()),
        ..ClientProfile::default()
    };
    let server = ServerProfile {
        name: "mail".into(),
        versions: vec![TlsVersion::Tls13],
        key_log: Some(keys.clone()),
        ..ServerProfile::default()
    };
    // A client that trusts another authority rejects the server certificate.
    let roots = if trust_server {
        ca.root_store().unwrap()
    } else {
        RunAuthority::generate("another authority")
            .unwrap()
            .root_store()
            .unwrap()
    };
    let connector = TlsConnector::new(&client, roots).unwrap();
    let acceptor = TlsAcceptor::new(&server, &identity).unwrap();
    (
        Arc::new(TlsProfiles::new().with_client("mail", connector)),
        Arc::new(TlsProfiles::new().with_server("mail", acceptor)),
    )
}

fn start_server(
    scenario: &Arc<Scenario>,
    ip: IpAddr,
    tls: Arc<TlsProfiles>,
) -> fps_runtime::RuntimeHandle {
    let worker = WorkerConfig {
        id: 10,
        core: None,
        role: Role::Server(ServerConfig {
            flow: FlowRef::new(Arc::clone(scenario), 0),
            listen: vec![SocketAddr::new(ip, 0)],
            config: ListenConfig::default(),
        }),
    };
    Runtime::start(RuntimeConfig::new(SEED, vec![worker]).with_tls(tls)).unwrap()
}

/// A client worker that starts [`SESSIONS`] sessions in about a second, from
/// sources in `127.<pool>.0.0/24`.
fn client_config(
    scenario: &Arc<Scenario>,
    server: SocketAddr,
    pool: u8,
    tls: Option<Arc<TlsProfiles>>,
) -> RuntimeConfig {
    let prefix = format!("127.{pool}.0.0/24");
    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some(&prefix),
        count: 16,
        ..PoolSpec::default()
    })
    .unwrap();
    let range = hosts.range();
    let profile = LoadProfile::new(vec![Phase::new(
        "steady",
        Duration::from_secs(1),
        Shape::Hold,
        1.0,
        1.0,
    )])
    .unwrap();
    let mut schedule =
        ScheduleConfig::new(Objective::SessionsPerSecond(SESSIONS as f64), profile, SEED);
    schedule.arrival = Arrival::Constant;
    schedule.stream = stream_id(0, 0);
    let worker = WorkerConfig {
        id: 0,
        core: None,
        role: Role::Client(Box::new(ClientConfig {
            flow: FlowRef::new(Arc::clone(scenario), 0),
            schedule,
            servers: vec![server.ip()],
            port: Some(server.port()),
            sources: SourceHosts {
                selector: HostSelector::new("clients", range, Selection::RoundRobin, SEED, 0)
                    .unwrap(),
                pool: hosts,
                ipv6: false,
            },
            connect: ConnectConfig::default(),
        })),
    };
    let mut config = RuntimeConfig::new(SEED, vec![worker]);
    config.snapshot_interval = Duration::from_millis(100);
    config.tls = tls;
    config
}

fn start_capture(name: &str) -> (Capture, std::path::PathBuf) {
    let dir = fresh_dir(name);
    let mut ring = RingConfig::new(&dir);
    ring.prefix = "rt".into();
    let mut config = CaptureConfig::new(ring);
    config.sample = SamplePolicy::EveryNth(1);
    config.seed = SEED;
    (
        Capture::start(config, Arc::new(StepClock::default())).unwrap(),
        dir,
    )
}

/// Sessions of `snapshot` that failed with `reason`.
fn failed_with(snapshot: &Snapshot, reason: &str) -> u64 {
    snapshot
        .iter()
        .filter(|(k, _)| {
            k.metric() == metric::SESSIONS_FAILED
                && k.labels()
                    .iter()
                    .any(|(l, v)| *l == "reason" && v == reason)
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(c) => *c,
            _ => 0,
        })
        .sum()
}

/// Reassembles every TCP connection of the packets, keyed by the client port.
fn connections(packets: &[common::Packet], server: SocketAddr) -> BTreeMap<u16, Conn> {
    reassemble(packets, server.port())
}

// Independent decryption of a TLS 1.3 record stream with an NSS key log. ----

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// HKDF-Expand-Label of RFC 8446 section 7.1 with an empty context.
fn expand_label(alg: hkdf::Algorithm, secret: &[u8], label: &str, len: usize) -> Vec<u8> {
    let prk = hkdf::Prk::new_less_safe(alg, secret);
    let full = format!("tls13 {label}");
    let length = u16::try_from(len).unwrap().to_be_bytes();
    let label_len = [u8::try_from(full.len()).unwrap()];
    let info: [&[u8]; 4] = [&length, &label_len, full.as_bytes(), &[0]];
    let okm = prk.expand(&info, Len(len)).unwrap();
    let mut out = vec![0u8; len];
    okm.fill(&mut out).unwrap();
    out
}

/// Looks up the secrets of the handshake whose ClientHello carries `random`.
fn secrets_of(keylog: &str, random: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let random = hex(random);
    keylog
        .lines()
        .filter_map(|l| {
            let mut parts = l.split(' ');
            let (label, r, secret) = (parts.next()?, parts.next()?, parts.next()?);
            (r == random).then(|| (label.to_owned(), unhex(secret)))
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// Decrypts the application data records of one direction of a TLS 1.3
/// stream. The first records use the handshake keys, as the encrypted server
/// flight does, and the rest the application keys. Returns `None` when a
/// record fails with both key sets.
fn decrypt_direction(
    stream: &[u8],
    suite: u16,
    handshake_secret: &[u8],
    traffic_secret: &[u8],
) -> Option<Vec<u8>> {
    let (hash, algorithm, key_len) = match suite {
        0x1301 => (hkdf::HKDF_SHA256, &aead::AES_128_GCM, 16),
        0x1302 => (hkdf::HKDF_SHA384, &aead::AES_256_GCM, 32),
        0x1303 => (hkdf::HKDF_SHA256, &aead::CHACHA20_POLY1305, 32),
        other => panic!("unexpected cipher suite {other:#x}"),
    };
    let keys: Vec<(aead::LessSafeKey, Vec<u8>)> = [handshake_secret, traffic_secret]
        .iter()
        .map(|s| {
            let key = expand_label(hash, s, "key", key_len);
            let iv = expand_label(hash, s, "iv", 12);
            (
                aead::LessSafeKey::new(aead::UnboundKey::new(algorithm, &key).unwrap()),
                iv,
            )
        })
        .collect();
    let (mut stage, mut seq, mut pos, mut out) = (0usize, 0u64, 0usize, Vec::new());
    while pos + 5 <= stream.len() {
        let length = usize::from(u16::from_be_bytes([stream[pos + 3], stream[pos + 4]]));
        let header = &stream[pos..pos + 5];
        let body = &stream[pos + 5..pos + 5 + length];
        pos += 5 + length;
        if header[0] != 23 {
            continue;
        }
        let plain = loop {
            let (key, iv) = &keys[stage];
            let mut nonce = [0u8; 12];
            nonce.copy_from_slice(iv);
            for (n, s) in nonce[4..].iter_mut().zip(seq.to_be_bytes()) {
                *n ^= s;
            }
            let mut buf = body.to_vec();
            let opened = key.open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(header),
                &mut buf,
            );
            match opened {
                Ok(p) => break p.to_vec(),
                Err(_) if stage == 0 => {
                    stage = 1;
                    seq = 0;
                }
                Err(_) => return None,
            }
        };
        seq += 1;
        let end = plain.iter().rposition(|b| *b != 0)?;
        // The last byte of the inner plaintext is the real content type.
        if plain[end] == 23 {
            out.extend_from_slice(&plain[..end]);
        }
    }
    Some(out)
}

/// Decrypts both directions of the TLS part of a connection. The client part
/// starts at its ClientHello and the server part at its ServerHello.
fn decrypt_connection(
    keylog: &str,
    client_tls: &[u8],
    server_tls: &[u8],
) -> Option<(Vec<u8>, Vec<u8>)> {
    // ClientHello: record header (5), handshake header (4), version (2), random (32).
    let random = client_tls.get(11..43)?;
    // ServerHello: the cipher suite follows the session id.
    let sid_len = usize::from(*server_tls.get(43)?);
    let suite = u16::from_be_bytes([
        *server_tls.get(44 + sid_len)?,
        *server_tls.get(45 + sid_len)?,
    ]);
    let secrets = secrets_of(keylog, random);
    let get = |name: &str| secrets.get(name).map(Vec::as_slice);
    let c2s = decrypt_direction(
        client_tls,
        suite,
        get("CLIENT_HANDSHAKE_TRAFFIC_SECRET")?,
        get("CLIENT_TRAFFIC_SECRET_0")?,
    )?;
    let s2c = decrypt_direction(
        server_tls,
        suite,
        get("SERVER_HANDSHAKE_TRAFFIC_SECRET")?,
        get("SERVER_TRAFFIC_SECRET_0")?,
    )?;
    Some((c2s, s2c))
}

/// Flips the first digit of every secret, which no key derivation survives.
fn broken(keylog: &str) -> String {
    keylog
        .lines()
        .map(|l| {
            let (head, secret) = l.rsplit_once(' ').unwrap();
            let flipped = if secret.starts_with('0') { "1" } else { "0" };
            format!("{head} {flipped}{}", &secret[1..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[test]
fn smtp_starttls_runs_its_second_half_over_tls_and_the_key_log_decrypts_it() {
    let scenario = compile(STARTTLS);
    let keys = Arc::new(NssKeyLog::in_memory());
    let (client_tls, server_tls) = profiles(true, &keys);
    let server = start_server(&scenario, v4(60, 0, 1), server_tls);
    let addr = server.listen_addrs()[0];
    let (capture, _dir) = start_capture("runtime-starttls");
    let client = Runtime::start(client_config(&scenario, addr, 61, Some(client_tls))).unwrap();
    client.capture_cell().set(capture.clone());
    let report = client.join();
    server.stop(Duration::from_secs(1));
    let server_report = server.join();

    // Every session completes on both sides.
    let c = report.collector.total(Side::Client).unwrap();
    let s = server_report.collector.total(Side::Server).unwrap();
    assert_eq!(c.counter_total(metric::SESSIONS_SUCCEEDED), SESSIONS);
    assert_eq!(c.counter_total(metric::SESSIONS_FAILED), 0);
    assert_eq!(s.counter_total(metric::SESSIONS_SUCCEEDED), SESSIONS);
    assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0);

    let files = capture.flush();
    let stats = capture.stop();
    assert_eq!(stats.sessions_written, SESSIONS, "{stats:?}");
    let packets = packets(&read_pcapng(&files[0]));
    let conns = connections(&packets, addr);
    assert_eq!(conns.len() as u64, SESSIONS);

    let keylog = keys.contents();
    assert!(keylog.contains("CLIENT_TRAFFIC_SECRET_0"), "{keylog}");
    for (port, conn) in &conns {
        assert!(conn.saw_syn && !conn.rst, "port {port}");
        // The first half is plaintext on the wire.
        assert!(
            conn.to_server.starts_with(CLIENT_CLEAR.as_bytes()),
            "port {port}: {:?}",
            String::from_utf8_lossy(&conn.to_server)
        );
        assert!(conn.to_client.starts_with(SERVER_CLEAR.as_bytes()));
        let client_tls = &conn.to_server[CLIENT_CLEAR.len()..];
        let server_tls = &conn.to_client[SERVER_CLEAR.len()..];
        // The second half is TLS records, a handshake first, and the mail
        // transaction appears nowhere on the wire.
        assert_eq!(client_tls[..3], [0x16, 0x03, 0x01], "ClientHello record");
        assert_eq!(server_tls[..3], [0x16, 0x03, 0x03], "ServerHello record");
        for stream in [&conn.to_server, &conn.to_client] {
            for secret in [
                "MAIL FROM",
                "alice@example.test",
                "AUTH PLAIN",
                "250 2.1.0",
                "221 2.0.0",
                "QUIT",
            ] {
                assert!(
                    !contains(stream, secret),
                    "port {port}: {secret} on the wire"
                );
            }
        }
        // The key log of the profiles decrypts the second half.
        let (c2s, s2c) = decrypt_connection(&keylog, client_tls, server_tls)
            .unwrap_or_else(|| panic!("port {port}: the key log does not decrypt the capture"));
        let c2s = String::from_utf8_lossy(&c2s);
        let s2c = String::from_utf8_lossy(&s2c);
        assert_eq!(
            c2s,
            "EHLO client.example.test\r\nMAIL FROM:<alice@example.test>\r\nQUIT\r\n"
        );
        assert_eq!(
            s2c,
            "250-mx.example.test\r\n250 AUTH PLAIN\r\n250 2.1.0 Ok\r\n221 2.0.0 Bye\r\n"
        );
        // A key log with wrong secrets decrypts nothing.
        assert!(decrypt_connection(&broken(&keylog), client_tls, server_tls).is_none());
    }
}

#[test]
fn an_untrusted_server_certificate_fails_the_client_with_tls_failure() {
    let scenario = compile(STARTTLS);
    let keys = Arc::new(NssKeyLog::in_memory());
    let (client_tls, server_tls) = profiles(false, &keys);
    let server = start_server(&scenario, v4(62, 0, 1), server_tls);
    let addr = server.listen_addrs()[0];
    let client = Runtime::start(client_config(&scenario, addr, 63, Some(client_tls))).unwrap();
    let report = client.join();
    server.stop(Duration::from_secs(1));
    let server_report = server.join();

    let c = report.collector.total(Side::Client).unwrap();
    assert_eq!(c.counter_total(metric::SESSIONS_SUCCEEDED), 0);
    assert_eq!(failed_with(&c, "tls_failure"), SESSIONS);
    // The server sees the alert of its peer, which is a TLS failure too.
    let s = server_report.collector.total(Side::Server).unwrap();
    assert_eq!(failed_with(&s, "tls_failure"), SESSIONS);
}

#[test]
fn a_flow_that_upgrades_without_profiles_fails_with_internal() {
    let scenario = compile(STARTTLS);
    let keys = Arc::new(NssKeyLog::in_memory());
    let (_, server_tls) = profiles(true, &keys);
    let server = start_server(&scenario, v4(64, 0, 1), server_tls);
    let addr = server.listen_addrs()[0];
    let client = Runtime::start(client_config(&scenario, addr, 65, None)).unwrap();
    let report = client.join();
    server.stop(Duration::from_secs(1));
    let _ = server.join();
    let c = report.collector.total(Side::Client).unwrap();
    assert_eq!(failed_with(&c, "internal"), SESSIONS);
}
