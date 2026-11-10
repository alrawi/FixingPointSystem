// SPDX-License-Identifier: GPL-3.0-only
//! The https scenarios of the http2 module, which run HTTP/2 over TLS 1.3 with
//! ALPN `h2`.
//!
//! Each scenario runs with both sides over the in memory duplex and the TLS
//! profiles of `common/tls.rs`, which are the browser profiles of the TLS
//! module on the client side and `server-tls13-h2` on the server side. The
//! tests check on the wire bytes and on the handshake reports that
//!
//! * the browser offers `h2` and `http/1.1`, the server selects `h2`, and both
//!   handshake reports name `h2`, TLS 1.3, and the SNI of the scenario,
//! * the ClientHello has the JA4 that the profile expects, which is the value
//!   of the labels file of the fixture,
//! * nothing of the HTTP/2 exchange crosses the wire in the clear, while the
//!   plaintext above TLS starts with the connection preface, and
//! * the key log of the client names the ClientHello random of the wire and
//!   holds the secrets that a decrypting reader needs.
//!
//! The JA4 test runs `modules/tls/tests/ja4_of_hello.py`, which needs only
//! `python3`.

mod common;

use std::path::Path;
use std::process::Command;

use common::frames::PREFACE;
use common::tls::{ALPN_H2, SERVER_NAME, SERVER_TLS13_H2};
use common::{run, scenarios, scenarios_with, tls_small, Run, TlsRun};
use fps_flow::program::{Op, TlsUpgrade};
use fps_module_tls::browser;
use fps_module_tls::TlsVersion;

/// What one https scenario must negotiate.
struct Expect {
    scenario: &'static str,
    profile: &'static str,
    port: i64,
}

const EXPECTED: [Expect; 2] = [
    Expect {
        scenario: "https_browse",
        profile: "chrome-136",
        port: 8443,
    },
    Expect {
        scenario: "https_upload",
        profile: "firefox-139",
        port: 8444,
    },
];

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs a scenario and returns its run, which must have succeeded over TLS.
async fn run_tls(name: &str, session_id: u64) -> (Run, TlsRun) {
    let all = scenarios_with(&tls_small());
    let (_, s) = all.iter().find(|(n, _)| n == name).expect("scenario");
    let mut r = run(s, 42, session_id).await;
    assert!(r.client.is_ok(), "{name} client: {:?}", r.client);
    assert!(r.server.is_ok(), "{name} server: {:?}", r.server);
    let tls = r.tls.take().expect("a session over TLS");
    (r, tls)
}

#[tokio::test(start_paused = true)]
async fn every_scenario_binds_its_profile_and_port() {
    let all = scenarios();
    for e in &EXPECTED {
        let (_, s) = all.iter().find(|(n, _)| n == e.scenario).expect("listed");
        assert_eq!(
            s.vars.get("server_port"),
            Some(&fps_flow::Value::Int(e.port)),
            "{}",
            e.scenario
        );
        assert!(
            browser::find(e.profile).is_some(),
            "{} names the unknown profile {}",
            e.scenario,
            e.profile
        );
        let flow = &s.flows[0];
        let upgrade = |ops: &[Op]| -> Vec<TlsUpgrade> {
            ops.iter()
                .filter_map(|op| match op {
                    Op::UpgradeTls(u) => Some(u.clone()),
                    _ => None,
                })
                .collect()
        };
        let client = upgrade(&flow.client.ops);
        assert_eq!(client.len(), 1, "{}", e.scenario);
        assert_eq!(client[0].profile, e.profile, "{}", e.scenario);
        assert_eq!(client[0].server_name.as_deref(), Some(SERVER_NAME));
        let server = upgrade(&flow.server.as_ref().expect("server side").ops);
        assert_eq!(server.len(), 1, "{}", e.scenario);
        assert_eq!(server[0].profile, SERVER_TLS13_H2, "{}", e.scenario);
        // The upgrade is the first action after the connection opens, so the
        // handshake is the first thing on the wire.
        assert!(matches!(flow.client.ops[0], Op::UpgradeTls(_)));
        assert!(matches!(
            flow.server.as_ref().expect("server").ops[0],
            Op::UpgradeTls(_)
        ));
    }
}

#[tokio::test(start_paused = true)]
async fn the_cleartext_scenarios_do_not_upgrade() {
    for (name, s) in scenarios().iter().filter(|(n, _)| n.starts_with("h2c_")) {
        assert!(!common::uses_tls(s), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_handshake_negotiates_h2_and_the_sni() {
    for e in &EXPECTED {
        for session_id in 1..=3 {
            let (_, tls) = run_tls(e.scenario, session_id).await;
            for report in [&tls.client, &tls.server] {
                assert_eq!(report.version, TlsVersion::Tls13, "{}", e.scenario);
                assert_eq!(report.alpn.as_deref(), Some(ALPN_H2), "{}", e.scenario);
                assert_eq!(report.server_name.as_deref(), Some(SERVER_NAME));
                assert!(!report.resumed, "{}", e.scenario);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_wire_is_encrypted_and_the_plaintext_is_http2() {
    for e in &EXPECTED {
        let (r, tls) = run_tls(e.scenario, 1).await;
        // A handshake record, TLS 1.0 in the record version, with a ClientHello inside.
        assert_eq!(&tls.wire_c2s[..3], &[0x16, 0x03, 0x01], "{}", e.scenario);
        assert_eq!(
            tls.wire_c2s[5], 0x01,
            "{} starts with a ClientHello",
            e.scenario
        );
        // The hello is the one place where the SNI and the ALPN ids are in the clear.
        assert!(
            contains(&tls.wire_c2s, SERVER_NAME.as_bytes()),
            "{}",
            e.scenario
        );
        assert!(
            contains(&tls.wire_c2s, b"\x02h2\x08http/1.1"),
            "{}",
            e.scenario
        );
        // The server hello of TLS 1.3 hides the certificate, so the SNI is in
        // the ClientHello only.
        assert!(
            !contains(&tls.wire_s2c, SERVER_NAME.as_bytes()),
            "{}",
            e.scenario
        );
        for wire in [&tls.wire_c2s, &tls.wire_s2c] {
            assert!(!contains(wire, PREFACE), "{} leaks the preface", e.scenario);
            assert!(!contains(wire, b"HTTP/2.0"), "{}", e.scenario);
            assert!(
                !contains(wire, b"www.example.test/"),
                "{} leaks a header",
                e.scenario
            );
        }
        // The plaintext above the TLS layer is the HTTP/2 exchange.
        assert!(r.c2s.starts_with(PREFACE), "{}", e.scenario);
        assert!(
            r.s2c.len() > 9 && r.s2c[3] == 4,
            "{}: the server starts with SETTINGS",
            e.scenario
        );
        assert!(tls.wire_c2s.len() > r.c2s.len() && tls.wire_s2c.len() > r.s2c.len());
    }
}

/// The ClientHello random, which the key log names in every line.
fn client_random(wire_c2s: &[u8]) -> &[u8] {
    // Record header 5, handshake header 4, legacy version 2.
    &wire_c2s[11..43]
}

#[tokio::test(start_paused = true)]
async fn the_key_log_names_the_hello_and_holds_the_secrets() {
    for e in &EXPECTED {
        let (_, tls) = run_tls(e.scenario, 1).await;
        let random = hex(client_random(&tls.wire_c2s));
        let labels: Vec<&str> = tls
            .key_log
            .lines()
            .map(|line| {
                let mut parts = line.split(' ');
                let label = parts.next().expect("label");
                assert_eq!(
                    parts.next(),
                    Some(random.as_str()),
                    "{}: {line}",
                    e.scenario
                );
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
            assert!(labels.contains(&label), "{} lacks {label}", e.scenario);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_client_hello_has_the_ja4_of_its_profile() {
    let python = Command::new("python3").arg("--version").output();
    if python.is_err() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = manifest.join("../../tls/tests/ja4_of_hello.py");
    let root = manifest.join("../../..");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir).expect("scratch directory");
    for e in &EXPECTED {
        let (_, tls) = run_tls(e.scenario, 1).await;
        let file = dir.join(format!("http2-{}.hello", e.scenario));
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
            "{}: {}",
            e.scenario,
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .unwrap_or_default()
                .to_owned()
        };
        let expected = browser::find(e.profile).expect("profile").expected_ja4;
        assert_eq!(field("ja4"), expected, "{} with {}", e.scenario, e.profile);
        assert_eq!(field("alpn"), "h2,http/1.1", "{}", e.scenario);
        assert_eq!(field("server_name"), SERVER_NAME, "{}", e.scenario);
    }
}

#[tokio::test(start_paused = true)]
async fn the_scenarios_use_two_profiles_with_their_own_ja4() {
    let ja4: std::collections::BTreeSet<_> = EXPECTED
        .iter()
        .map(|e| browser::find(e.profile).expect("profile").expected_ja4)
        .collect();
    assert_eq!(ja4.len(), 2);
}
