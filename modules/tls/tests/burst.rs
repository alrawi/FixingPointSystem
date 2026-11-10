// SPDX-License-Identifier: GPL-3.0-only
//! TRN-15: plaintext backpressure of `TlsSession` under a burst, and the
//! handshakes of the mail client profile and the h2 and no ALPN server profiles.

mod common;

use common::{parse_client_hello, Dir, Log, Tap};
use fps_io::memory::{duplex, MemoryIo};
use fps_module_tls::mail::{
    mail_client, server_profile, server_tls13_h2, server_tls13_noalpn, MAIL_CLIENT,
    SERVER_TLS13_H2, SERVER_TLS13_NOALPN,
};
use fps_module_tls::{
    browser, ClientProfile, LeafKey, RunAuthority, ServerProfile, TlsAcceptor, TlsConnector,
    TlsError, TlsSession, TlsVersion,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const NAME: &str = "mail.example.test";
const MIB: usize = 1 << 20;

struct Rig {
    connector: TlsConnector,
    acceptor: TlsAcceptor,
}

fn rig(client: &ClientProfile, server: &ServerProfile) -> Rig {
    let ca = RunAuthority::generate("fps burst test authority").unwrap();
    let identity = ca.issue(&[NAME], LeafKey::EcdsaP256).unwrap();
    Rig {
        connector: TlsConnector::new(client, ca.root_store().unwrap()).unwrap(),
        acceptor: TlsAcceptor::new(server, &identity).unwrap(),
    }
}

/// A body whose every byte depends on its position and on `seed`, so a lost,
/// repeated, or swapped chunk shows up in the comparison.
fn body(seed: u8) -> Vec<u8> {
    (0..MIB)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add((i >> 8) as u8) ^ seed)
        .collect()
}

/// How the writer cuts the body into `write_all` calls. Rustls makes one record
/// of at most 16 KiB per call, so the pattern decides how many records a
/// ciphertext chunk completes.
#[derive(Debug, Clone, Copy)]
enum Cut {
    /// One call for the whole body.
    Whole,
    /// Alternating 16 KiB and one byte writes, which pack a full record and
    /// small ones into every chunk.
    FullAndTiny,
    /// A nine byte header and a 16 KiB payload, the shape of HTTP/2 frames.
    FrameShaped,
}

async fn send<IO: fps_io::SessionIo>(s: &mut TlsSession<IO>, data: &[u8], cut: Cut) {
    match cut {
        Cut::Whole => s.write_all(data).await.unwrap(),
        Cut::FullAndTiny => {
            let mut rest = data;
            let mut big = true;
            while !rest.is_empty() {
                let n = if big { 16 * 1024 } else { 1 }.min(rest.len());
                let (head, tail) = rest.split_at(n);
                s.write_all(head).await.unwrap();
                rest = tail;
                big = !big;
            }
        }
        Cut::FrameShaped => {
            let mut rest = data;
            while !rest.is_empty() {
                let n = 9.min(rest.len());
                let (head, tail) = rest.split_at(n);
                s.write_all(head).await.unwrap();
                let n = (16 * 1024).min(tail.len());
                let (head, tail) = tail.split_at(n);
                s.write_all(head).await.unwrap();
                rest = tail;
            }
        }
    }
    s.flush().await.unwrap();
}

/// Reads `len` bytes in reads of at most `step` bytes.
async fn take<IO: fps_io::SessionIo>(s: &mut TlsSession<IO>, len: usize, step: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    let mut at = 0;
    while at < len {
        let end = (at + step).min(len);
        let n = s.read(&mut out[at..end]).await.unwrap();
        assert!(n > 0, "the session ended after {at} of {len} bytes");
        at += n;
    }
    out
}

/// Moves one body from each side to the other. The pipe holds both bursts, so
/// each side writes its whole body before the other reads a byte, and a read
/// sees ciphertext chunks that complete several records at once.
async fn burst_both_ways(cut: Cut, step: usize, version: TlsVersion) {
    let client = ClientProfile {
        versions: vec![version],
        ..ClientProfile::default()
    };
    let server = ServerProfile {
        versions: vec![version],
        ..ServerProfile::default()
    };
    let rig = rig(&client, &server);
    let (c, s) = duplex(4 * MIB);
    let (c, s) = tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s));
    let (mut c, mut s) = (c.unwrap(), s.unwrap());

    let up = body(1);
    let down = body(2);
    send(&mut c, &up, cut).await;
    send(&mut s, &down, cut).await;
    let got_up = take(&mut s, MIB, step).await;
    let got_down = take(&mut c, MIB, step).await;
    assert!(got_up == up, "client to server body differs ({cut:?})");
    assert!(got_down == down, "server to client body differs ({cut:?})");

    // The session is still usable after the burst.
    c.write_all(b"after").await.unwrap();
    c.flush().await.unwrap();
    assert_eq!(take(&mut s, 5, 5).await, b"after");
}

#[tokio::test]
async fn one_mebibyte_in_one_burst_both_ways() {
    for version in [TlsVersion::Tls13, TlsVersion::Tls12] {
        burst_both_ways(Cut::Whole, MIB, version).await;
    }
}

#[tokio::test]
async fn bursts_of_full_and_tiny_records_survive_a_slow_reader() {
    for version in [TlsVersion::Tls13, TlsVersion::Tls12] {
        for step in [1_000, 4_096, MIB] {
            burst_both_ways(Cut::FullAndTiny, step, version).await;
        }
    }
}

#[tokio::test]
async fn frame_shaped_bursts_survive_a_slow_reader() {
    for step in [9, 1_000, MIB] {
        burst_both_ways(Cut::FrameShaped, step, TlsVersion::Tls13).await;
    }
}

#[tokio::test]
async fn a_burst_that_ends_with_close_notify_reads_to_the_end() {
    let rig = rig(&ClientProfile::default(), &ServerProfile::default());
    let (c, s) = duplex(4 * MIB);
    let (c, s) = tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s));
    let (mut c, mut s) = (c.unwrap(), s.unwrap());
    let down = body(3);
    send(&mut s, &down, Cut::FullAndTiny).await;
    s.shutdown().await.unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 1_000];
    loop {
        let n = c.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert!(got == down, "the body differs or the end came early");
}

type Handshake = (
    Result<TlsSession<Tap<MemoryIo>>, TlsError>,
    Result<TlsSession<MemoryIo>, TlsError>,
);

/// Runs a handshake and returns both outcomes and what the client sent.
async fn handshake(client: &ClientProfile, server: &ServerProfile) -> (Handshake, Vec<u8>) {
    let rig = rig(client, server);
    let log = Log::default();
    let (c, s) = duplex(64 * 1024);
    let c = Tap::new(c, Dir::ClientToServer, &log);
    let out = tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s));
    (out, common::stream(&log, Dir::ClientToServer))
}

fn alpn_of(client: &ClientProfile, server: &ServerProfile) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ((c, s), _) = rt.block_on(handshake(client, server));
    let (c, s) = (c.unwrap(), s.unwrap());
    assert_eq!(c.report().version, TlsVersion::Tls13);
    (c.report().alpn.clone(), s.report().alpn.clone())
}

fn chrome() -> ClientProfile {
    browser::find("chrome-136").unwrap().client_profile()
}

fn http11_only() -> ClientProfile {
    ClientProfile {
        alpn: vec![b"http/1.1".to_vec()],
        ..ClientProfile::default()
    }
}

#[test]
fn the_new_profiles_negotiate_the_expected_alpn() {
    let h2 = Some(b"h2".to_vec());
    let http11 = Some(b"http/1.1".to_vec());
    let cases: [(&str, ClientProfile, ServerProfile, Option<Vec<u8>>); 6] = [
        ("browser to h2", chrome(), server_tls13_h2(), h2.clone()),
        (
            "http/1.1 only to h2",
            http11_only(),
            server_tls13_h2(),
            http11,
        ),
        ("mail client to h2", mail_client(), server_tls13_h2(), None),
        ("browser to noalpn", chrome(), server_tls13_noalpn(), None),
        (
            "mail client to noalpn",
            mail_client(),
            server_tls13_noalpn(),
            None,
        ),
        (
            "firefox to h2",
            browser::find("firefox-139").unwrap().client_profile(),
            server_tls13_h2(),
            h2,
        ),
    ];
    for (what, client, server, want) in cases {
        let (c, s) = alpn_of(&client, &server);
        assert_eq!(c, want, "{what}: client report");
        assert_eq!(s, want, "{what}: server report");
    }
}

#[tokio::test]
async fn the_mail_client_sends_no_alpn_extension_and_the_browser_does() {
    let ((c, s), sent) = handshake(&mail_client(), &server_tls13_noalpn()).await;
    let (c, _s) = (c.unwrap(), s.unwrap());
    let hello = parse_client_hello(&sent);
    assert!(hello.alpn.is_empty(), "mail client: {:?}", hello.alpn);
    assert_eq!(hello.sni.as_deref(), Some(NAME));
    assert_eq!(c.report().server_name.as_deref(), Some(NAME));

    let ((c, s), sent) = handshake(&chrome(), &server_tls13_h2()).await;
    let (_c, _s) = (c.unwrap(), s.unwrap());
    assert_eq!(parse_client_hello(&sent).alpn, ["h2", "http/1.1"]);
}

#[tokio::test]
async fn the_server_profiles_accept_tls13_only_and_keep_their_names() {
    for name in [SERVER_TLS13_H2, SERVER_TLS13_NOALPN] {
        let server = server_profile(name).unwrap();
        assert_eq!(server.name, name);
        let tls12 = ClientProfile {
            versions: vec![TlsVersion::Tls12],
            ..mail_client()
        };
        let ((c, s), _) = handshake(&tls12, &server).await;
        assert!(c.is_err() && s.is_err(), "{name} must refuse TLS 1.2");
    }
    assert_eq!(mail_client().name, MAIL_CLIENT);
}

#[tokio::test]
async fn the_new_profiles_move_a_burst() {
    let (c, s) = duplex(4 * MIB);
    let rig = rig(&mail_client(), &server_tls13_noalpn());
    let (c, s) = tokio::join!(rig.connector.connect(c, NAME), rig.acceptor.accept(s));
    let (mut c, mut s) = (c.unwrap(), s.unwrap());
    let up = body(4);
    send(&mut c, &up, Cut::Whole).await;
    assert!(take(&mut s, MIB, MIB).await == up);
}
