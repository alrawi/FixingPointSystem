// SPDX-License-Identifier: GPL-3.0-only
//! The https scenarios over TLS (MOD-04).
//!
//! Each scenario runs with both sides over the in memory duplex and the TLS
//! profiles of `common/tls.rs`, which are the browser profiles of the TLS
//! module on the client side and one acceptor per version on the server side.
//! The tests check on the bytes of the wire and on the handshake reports that
//!
//! * the scenario negotiates the version, the ALPN id `http/1.1`, and the SNI
//!   that it names, with the profile that it names,
//! * the ClientHello has the JA4 that the profile expects, which is the value
//!   of the labels file of the fixture,
//! * nothing of the HTTP exchange crosses the wire in the clear, and a TLS 1.2
//!   server shows its certificate while a TLS 1.3 server hides it, and
//! * the key log of the client names the ClientHello random of the wire and
//!   holds the secrets that a decrypting reader needs.
//!
//! The JA4 test runs `modules/tls/tests/ja4_of_hello.py`, which needs only
//! `python3`.

mod common;

use std::path::Path;
use std::process::Command;

use common::tls::{ALPN_HTTP11, SERVER_NAME, SERVER_TLS12, SERVER_TLS13};
use common::{run, scenarios, Run, TlsRun};
use fps_flow::program::{Op, TlsUpgrade};
use fps_module_tls::browser;
use fps_module_tls::TlsVersion;

/// What one https scenario must negotiate.
struct Expect {
    scenario: &'static str,
    profile: &'static str,
    version: TlsVersion,
    port: i64,
}

const EXPECTED: [Expect; 4] = [
    Expect {
        scenario: "https_fetch",
        profile: "chrome-136",
        version: TlsVersion::Tls13,
        port: 8443,
    },
    Expect {
        scenario: "https_keepalive_browse",
        profile: "firefox-139",
        version: TlsVersion::Tls13,
        port: 8444,
    },
    Expect {
        scenario: "https_fetch_tls12",
        profile: "safari-18.5",
        version: TlsVersion::Tls12,
        port: 8445,
    },
    Expect {
        scenario: "https_keepalive_browse_tls12",
        profile: "edge-134",
        version: TlsVersion::Tls12,
        port: 8446,
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
    let all = scenarios();
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
        // Each side upgrades once. The client names the browser profile and
        // the SNI, and the server names the profile of its version.
        let client = upgrade(&flow.client.ops);
        assert_eq!(client.len(), 1, "{}", e.scenario);
        assert_eq!(client[0].profile, e.profile, "{}", e.scenario);
        assert_eq!(client[0].server_name.as_deref(), Some(SERVER_NAME));
        let server = upgrade(&flow.server.as_ref().expect("server side").ops);
        assert_eq!(server.len(), 1, "{}", e.scenario);
        let want = match e.version {
            TlsVersion::Tls13 => SERVER_TLS13,
            TlsVersion::Tls12 => SERVER_TLS12,
        };
        assert_eq!(server[0].profile, want, "{}", e.scenario);
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
async fn the_handshake_negotiates_version_alpn_and_sni() {
    for e in &EXPECTED {
        for session_id in 1..=3 {
            let (_, tls) = run_tls(e.scenario, session_id).await;
            for report in [&tls.client, &tls.server] {
                assert_eq!(report.version, e.version, "{}", e.scenario);
                assert_eq!(report.alpn.as_deref(), Some(ALPN_HTTP11), "{}", e.scenario);
                assert_eq!(report.server_name.as_deref(), Some(SERVER_NAME));
                assert!(!report.resumed, "{}", e.scenario);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_wire_is_encrypted_and_starts_with_the_client_hello() {
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
        for wire in [&tls.wire_c2s, &tls.wire_s2c] {
            assert!(!contains(wire, b"HTTP/1.1"), "{} leaks HTTP", e.scenario);
            assert!(!contains(wire, b"Host: "), "{} leaks a header", e.scenario);
        }
        // The plaintext above the TLS layer is the HTTP exchange.
        assert!(r.c2s.starts_with(b"GET "), "{}", e.scenario);
        assert!(r.s2c.starts_with(b"HTTP/1.1 200 "), "{}", e.scenario);
        assert!(tls.wire_c2s.len() > r.c2s.len() && tls.wire_s2c.len() > r.s2c.len());
        // TLS 1.2 sends the certificate in the clear and TLS 1.3 encrypts it.
        let certificate_visible = contains(&tls.wire_s2c, SERVER_NAME.as_bytes());
        assert_eq!(
            certificate_visible,
            e.version == TlsVersion::Tls12,
            "{}",
            e.scenario
        );
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
        let wanted: &[&str] = match e.version {
            TlsVersion::Tls13 => &[
                "CLIENT_HANDSHAKE_TRAFFIC_SECRET",
                "SERVER_HANDSHAKE_TRAFFIC_SECRET",
                "CLIENT_TRAFFIC_SECRET_0",
                "SERVER_TRAFFIC_SECRET_0",
            ],
            TlsVersion::Tls12 => &["CLIENT_RANDOM"],
        };
        for label in wanted {
            assert!(labels.contains(label), "{} lacks {label}", e.scenario);
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
        let file = dir.join(format!("{}.hello", e.scenario));
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
async fn the_scenarios_use_at_least_two_profiles_and_both_versions() {
    let profiles: std::collections::BTreeSet<_> = EXPECTED.iter().map(|e| e.profile).collect();
    assert!(profiles.len() >= 2);
    assert!(EXPECTED.iter().any(|e| e.version == TlsVersion::Tls12));
    assert!(EXPECTED.iter().any(|e| e.version == TlsVersion::Tls13));
    // Firefox has a JA4 of its own, and the other three share one, so the
    // scenarios cover both values that the labels file lists.
    let ja4: std::collections::BTreeSet<_> = EXPECTED
        .iter()
        .map(|e| browser::find(e.profile).expect("profile").expected_ja4)
        .collect();
    assert!(ja4.len() >= 2);
}
