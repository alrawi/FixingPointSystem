// SPDX-License-Identifier: GPL-3.0-only
//! The submission scenario with STARTTLS (MOD-20).
//!
//! The scenario runs with both sides over the in memory duplex and the TLS
//! profiles of `common/tls.rs`. The tests check on the dialogue, on the bytes
//! of the wire, and on the handshake reports that
//!
//! * the first EHLO reply offers STARTTLS and no AUTH, the client asks for
//!   STARTTLS, the server answers 220, and only then does the handshake start,
//! * the second EHLO, which RFC 3207 requires after the upgrade, gets a reply
//!   that offers AUTH and no STARTTLS, and the AUTH and the message follow,
//! * the handshake negotiates TLS 1.3 with the SNI of the scenario, no ALPN id,
//!   and the profile that the scenario names, and the ClientHello has the JA4
//!   of that profile, which is the value of the labels file of the fixture,
//! * nothing of the mail transaction crosses the wire in the clear, and
//! * the key log of the client names the ClientHello random of the wire and
//!   holds the secrets that a decrypting reader needs.
//!
//! The JA4 test runs `modules/tls/tests/ja4_of_hello.py`, which needs only
//! `python3`.

mod common;

use std::path::Path;
use std::process::Command;

use common::tls::{SERVER_NAME, SERVER_TLS13};
use common::{all_scenarios_with, run, uses_tls, Run, TlsRun};
use fps_flow::program::{Op, TlsRole};
use fps_module_tls::browser;
use fps_module_tls::TlsVersion;

const SCENARIO: &str = "starttls_submission";
const PROFILE: &str = "firefox-139";

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("the dialogue is ASCII")
}

/// Runs the scenario and returns its run, which must have succeeded over TLS.
async fn run_tls(session_id: u64) -> (Run, TlsRun) {
    let all = all_scenarios_with(&[]);
    let (_, s) = all.iter().find(|(n, _)| n == SCENARIO).expect("scenario");
    let mut r = run(s, 42, session_id).await;
    assert!(r.client.is_ok(), "client: {:?}", r.client);
    assert!(r.server.is_ok(), "server: {:?}", r.server);
    let tls = r.tls.take().expect("a session over TLS");
    (r, tls)
}

/// The lines of the replies, from the reply codes of the first line to the end.
fn lines(s: &str) -> Vec<&str> {
    s.split_terminator("\r\n").collect()
}

#[test]
fn the_manifest_lists_the_scenario_and_it_upgrades_once_per_side() {
    let all = all_scenarios_with(&[]);
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names.last(), Some(&SCENARIO), "{names:?}");
    let (_, s) = all.iter().find(|(n, _)| n == SCENARIO).expect("listed");
    assert!(uses_tls(s));
    assert_eq!(
        s.vars.get("server_port"),
        Some(&fps_flow::Value::Int(587)),
        "submission port"
    );
    // Only this scenario upgrades.
    for (name, scenario) in &all {
        assert_eq!(uses_tls(scenario), name == SCENARIO, "{name}");
    }
    let flow = &s.flows[0];
    let upgrades = |ops: &[Op]| {
        ops.iter()
            .filter_map(|op| match op {
                Op::UpgradeTls(u) => Some(u.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let client = upgrades(&flow.client.ops);
    assert_eq!(client.len(), 1);
    assert_eq!(client[0].profile, PROFILE);
    assert_eq!(client[0].server_name.as_deref(), Some(SERVER_NAME));
    assert!(client[0].role.is_none() || client[0].role == Some(TlsRole::Client));
    let server = upgrades(&flow.server.as_ref().expect("server side").ops);
    assert_eq!(server.len(), 1);
    assert_eq!(server[0].profile, SERVER_TLS13);
    // The variable that names the server and the SNI that the client sends agree.
    assert_eq!(
        s.vars.get("server_name"),
        Some(&fps_flow::Value::Str(SERVER_NAME.into()))
    );
    assert!(
        browser::find(PROFILE).is_some(),
        "{PROFILE} is not a browser profile"
    );
}

#[tokio::test(start_paused = true)]
async fn the_dialogue_upgrades_after_the_220_and_repeats_ehlo() {
    for session_id in 1..=3 {
        let (r, tls) = run_tls(session_id).await;
        let (c_before, s_before) = (tls.plain_before_c2s, tls.plain_before_s2c);
        let (c_plain, c_secure) = r.c2s.split_at(c_before);
        let (s_plain, s_secure) = r.s2c.split_at(s_before);

        // Before the upgrade: EHLO, STARTTLS, and the replies that lead to 220.
        let client = lines(text(c_plain));
        assert_eq!(client.len(), 2, "{client:?}");
        assert!(client[0].starts_with("EHLO "), "{client:?}");
        assert_eq!(client[1], "STARTTLS");
        let server = lines(text(s_plain));
        assert!(server[0].starts_with("220 submit.example.test ESMTP"));
        assert_eq!(*server.last().unwrap(), "220 2.0.0 Ready to start TLS");
        let first_ehlo = &server[1..server.len() - 1];
        assert!(first_ehlo[0].starts_with("250-"), "{first_ehlo:?}");
        assert!(first_ehlo.last().unwrap().starts_with("250 "));
        assert!(
            first_ehlo.contains(&"250-STARTTLS"),
            "the first EHLO reply offers STARTTLS: {first_ehlo:?}"
        );
        assert!(
            !first_ehlo.iter().any(|l| l.contains("AUTH")),
            "no AUTH before TLS: {first_ehlo:?}"
        );

        // After the upgrade: EHLO again, with AUTH and without STARTTLS.
        assert!(c_secure.starts_with(b"EHLO laptop-23.example.test\r\nAUTH PLAIN "));
        let reply = text(s_secure);
        let end = reply.find("\r\n235 ").expect("the 235 reply");
        let second_ehlo = lines(&reply[..end + 2]);
        assert!(second_ehlo
            .iter()
            .any(|l| l.starts_with("250-AUTH PLAIN LOGIN")));
        assert!(!second_ehlo.iter().any(|l| l.contains("STARTTLS")));
        assert!(second_ehlo.last().unwrap().starts_with("250 "));

        // The transaction, the message, and the farewell.
        let secure = text(c_secure);
        for needle in ["MAIL FROM:<", "RCPT TO:<", "DATA\r\n", "QUIT\r\n"] {
            assert!(secure.contains(needle), "{needle}");
        }
        assert_eq!(secure.matches("RCPT TO:<").count(), 2);
        assert!(reply.ends_with("221 2.0.0 Bye\r\n"));
    }
}

fn unbase64(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=' && c != b'\r' && c != b'\n') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[tokio::test(start_paused = true)]
async fn the_message_inside_tls_carries_a_pdf_attachment() {
    let mut sizes = Vec::new();
    for session_id in 1..=6 {
        let (r, _) = run_tls(session_id).await;
        let data = find(&r.c2s, b"DATA\r\n").expect("DATA") + 6;
        let end = find(&r.c2s[data..], b"\r\n.\r\n").expect("the closing dot line") + data;
        let message = text(&r.c2s[data..end]);
        assert!(message.contains("Subject: Invoice "));
        assert!(message.contains("Content-Type: multipart/mixed; boundary="));
        assert!(message.contains("filename=\"invoice.pdf\""));
        assert!(message.contains("Content-Type: application/pdf"));
        assert!(
            message.contains("\r\n..\r\nThe line above holds a single dot.\r\n"),
            "the dot line is stuffed"
        );
        let (_, part) = message
            .split_once("Content-Type: application/pdf")
            .expect("the attachment");
        assert!(part.contains("Content-Transfer-Encoding: base64\r\n"));
        let (_, encoded) = part.split_once("\r\n\r\n").expect("a blank line");
        let encoded = encoded.split("\r\n--").next().unwrap();
        let bytes = unbase64(encoded);
        assert!(bytes.starts_with(b"%PDF-"), "{:02x?}", &bytes[..8]);
        sizes.push(bytes.len());
    }
    sizes.sort_unstable();
    sizes.dedup();
    assert!(sizes.len() >= 4, "attachment sizes do not vary: {sizes:?}");
    assert!(sizes[0] >= 24_000 && *sizes.last().unwrap() <= 160_000 + 2048);
}

#[tokio::test(start_paused = true)]
async fn the_handshake_negotiates_tls13_without_alpn() {
    for session_id in 1..=3 {
        let (_, tls) = run_tls(session_id).await;
        for report in [&tls.client, &tls.server] {
            assert_eq!(report.version, TlsVersion::Tls13);
            assert_eq!(report.alpn, None, "a mail server selects no ALPN id");
            assert_eq!(report.server_name.as_deref(), Some(SERVER_NAME));
            assert!(!report.resumed);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_wire_holds_the_hello_and_hides_the_mail_transaction() {
    let (r, tls) = run_tls(1).await;
    // A handshake record, TLS 1.0 in the record version, with a ClientHello inside.
    assert_eq!(&tls.wire_c2s[..3], &[0x16, 0x03, 0x01]);
    assert_eq!(
        tls.wire_c2s[5], 0x01,
        "the records start with a ClientHello"
    );
    assert!(contains(&tls.wire_c2s, SERVER_NAME.as_bytes()), "SNI");
    for (wire, what) in [(&tls.wire_c2s, "client"), (&tls.wire_s2c, "server")] {
        // Every needle has six bytes or more, so a chance match in random
        // records is out of the question.
        for secret in [
            "EHLO laptop",
            "AUTH PLAIN",
            "MAIL FROM",
            "RCPT TO",
            "DATA\r\n",
            "invoice.pdf",
            "Subject:",
            "Content-Type",
            "235 2.7.0",
            "250 2.1.",
            "354 End",
            "QUIT\r\n",
        ] {
            assert!(
                !contains(wire, secret.as_bytes()),
                "the {what} records leak {secret}"
            );
        }
    }
    // TLS 1.3 encrypts the certificate, so the name of the server is not in
    // the records of the server.
    assert!(!contains(&tls.wire_s2c, SERVER_NAME.as_bytes()));
    // The records carry more than the plaintext above them.
    assert!(tls.wire_c2s.len() > r.c2s.len() - tls.plain_before_c2s);
    assert!(tls.wire_s2c.len() > r.s2c.len() - tls.plain_before_s2c);
}

/// The ClientHello random, which the key log names in every line.
fn client_random(wire_c2s: &[u8]) -> &[u8] {
    // Record header 5, handshake header 4, legacy version 2.
    &wire_c2s[11..43]
}

#[tokio::test(start_paused = true)]
async fn the_key_log_names_the_hello_and_holds_the_tls13_secrets() {
    let (_, tls) = run_tls(1).await;
    let random = hex(client_random(&tls.wire_c2s));
    let labels: Vec<&str> = tls
        .key_log
        .lines()
        .map(|line| {
            let mut parts = line.split(' ');
            let label = parts.next().expect("label");
            assert_eq!(parts.next(), Some(random.as_str()), "{line}");
            let secret = parts.next().expect("secret");
            assert!(secret.len() >= 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()));
            label
        })
        .collect();
    for label in [
        "CLIENT_HANDSHAKE_TRAFFIC_SECRET",
        "SERVER_HANDSHAKE_TRAFFIC_SECRET",
        "CLIENT_TRAFFIC_SECRET_0",
        "SERVER_TRAFFIC_SECRET_0",
    ] {
        assert!(labels.contains(&label), "no {label}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_client_hello_has_the_ja4_of_its_profile() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = manifest.join("../../tls/tests/ja4_of_hello.py");
    let root = manifest.join("../../..");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir).expect("scratch directory");
    let (_, tls) = run_tls(1).await;
    let file = dir.join("smtp_starttls.hello");
    std::fs::write(&file, &tls.wire_c2s).expect("write the wire bytes");
    let out = Command::new("python3")
        .arg("-I")
        .arg(&script)
        .arg(&root)
        .arg(&file)
        .output()
        .expect("python3 starts");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let field = |key: &str| {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    let expected = browser::find(PROFILE).expect("profile").expected_ja4;
    assert_eq!(field("ja4"), expected);
    // The profile is a browser profile, so it offers the ALPN ids of a
    // browser. Thunderbird sends none to a mail server, a gap that the
    // README names.
    assert_eq!(field("alpn"), "h2,http/1.1");
    assert_eq!(field("server_name"), SERVER_NAME);
}
