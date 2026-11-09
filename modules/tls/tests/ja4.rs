// SPDX-License-Identifier: GPL-3.0-only
//! JA4 tests of the browser profiles (TRN-05).
//!
//! Each test runs a real handshake with the module, records the bytes of the
//! ClientHello, and computes the JA4 fingerprint from those bytes with
//! `realism/ja4.py`, which follows the FoxIO specification. The fingerprint must
//! equal `expected_ja4` of the profile. `tshark_agrees_with_the_expected_ja4`
//! also reads the fingerprint from tshark 4.2 or newer. The Python half needs
//! only `python3`. A test skips a missing tool unless `FPS_REQUIRE_CAPTURE_TOOLS`
//! is `1`, which the `mod-tls` workflow sets after it installs tshark.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use common::{pcap, Dir, Log, Tap};
use fps_io::memory::duplex;
use fps_module_tls::browser::{BrowserProfile, BROWSER_PROFILES};
use fps_module_tls::{LeafKey, RunAuthority, ServerProfile, TlsAcceptor, TlsConnector};

const SERVER_NAME: &str = "tls.example.test";

/// Runs one handshake of `profile` and returns the recorded bytes of both sides.
async fn handshake(profile: &BrowserProfile) -> Log {
    let ca = RunAuthority::generate("fps ja4 authority").unwrap();
    let identity = ca.issue(&[SERVER_NAME], LeafKey::EcdsaP256).unwrap();
    let connector = TlsConnector::new(&profile.client_profile(), ca.root_store().unwrap()).unwrap();
    let acceptor = TlsAcceptor::new(&ServerProfile::default(), &identity).unwrap();
    let log: Log = Arc::default();
    let (c, s) = duplex(64 * 1024);
    let (c, s) = (
        Tap::new(c, Dir::ClientToServer, &log),
        Tap::new(s, Dir::ServerToClient, &log),
    );
    let (c, s) = tokio::join!(connector.connect(c, SERVER_NAME), acceptor.accept(s));
    c.unwrap_or_else(|e| panic!("{}: client handshake failed: {e}", profile.name));
    s.unwrap_or_else(|e| panic!("{}: server handshake failed: {e}", profile.name));
    log
}

/// The bytes that the client wrote before the first server byte.
fn client_hello(log: &Log) -> Vec<u8> {
    let log = log.lock().unwrap();
    let mut bytes = Vec::new();
    for (dir, data) in log.iter() {
        if *dir == Dir::ServerToClient {
            break;
        }
        bytes.extend_from_slice(data);
    }
    bytes
}

fn tool_available(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Returns true when the test must stop because its tool is missing.
fn skip_without(tool: &str, available: bool) -> bool {
    if available {
        return false;
    }
    assert!(
        std::env::var("FPS_REQUIRE_CAPTURE_TOOLS").as_deref() != Ok("1"),
        "{tool} is required by FPS_REQUIRE_CAPTURE_TOOLS=1 but is not available"
    );
    eprintln!("skipping: {tool} is not available");
    true
}

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `ja4_of_hello.py` over the ClientHello bytes and returns its fields.
fn python_fields(name: &str, hello: &[u8]) -> BTreeMap<String, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = manifest.join("../tests/ja4_of_hello.py");
    let root = manifest.join("../../..");
    let file = out_dir().join(format!("{name}.hello"));
    std::fs::write(&file, hello).unwrap();
    let out = Command::new("python3")
        .arg("-I")
        .arg(&script)
        .arg(&root)
        .arg(&file)
        .output()
        .expect("python3 starts");
    assert!(
        out.status.success(),
        "{name}: ja4_of_hello.py failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn hex_list(values: impl IntoIterator<Item = u16>) -> String {
    values
        .into_iter()
        .map(|v| format!("{v:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[tokio::test]
async fn every_browser_profile_produces_its_expected_ja4() {
    if skip_without("python3", tool_available("python3", &["--version"])) {
        return;
    }
    for profile in &BROWSER_PROFILES {
        let log = handshake(profile).await;
        let fields = python_fields(profile.name, &client_hello(&log));
        assert_eq!(
            fields["ja4"], profile.expected_ja4,
            "{}: the ClientHello has JA4 {} but the profile expects {} (the {} has {})",
            profile.name, fields["ja4"], profile.expected_ja4, profile.browser, profile.browser_ja4
        );
        // The profile never claims the fingerprint of the real browser, because
        // the gap list says rustls cannot reproduce it.
        assert_ne!(fields["ja4"], profile.browser_ja4, "{}", profile.name);
        assert_eq!(fields["server_name"], SERVER_NAME);
    }
}

#[tokio::test]
async fn profile_orders_reach_the_wire() {
    if skip_without("python3", tool_available("python3", &["--version"])) {
        return;
    }
    for profile in &BROWSER_PROFILES {
        let client = profile.client_profile();
        let log = handshake(profile).await;
        let fields = python_fields(&format!("{}-order", profile.name), &client_hello(&log));
        // The suites follow the profile, then the renegotiation signal that rustls
        // appends when TLS 1.2 is on.
        let suites = client
            .cipher_suites
            .iter()
            .map(|s| iana_code(s.as_str()))
            .chain([0x00ff]);
        assert_eq!(fields["ciphers"], hex_list(suites), "{}", profile.name);
        assert_eq!(
            fields["sig_algs"],
            hex_list(client.sig_schemes.iter().map(|s| s.code())),
            "{}",
            profile.name
        );
        assert_eq!(fields["alpn"], "h2,http/1.1", "{}", profile.name);
    }
}

#[tokio::test]
async fn profiles_that_share_a_client_hello_share_a_ja4() {
    // Edge and Chrome use one list of suites and schemes, so rustls cannot tell
    // them apart on the wire, and neither can JA4. The wiki page says so.
    let chrome = fps_module_tls::browser::find("chrome-136").unwrap();
    let edge = fps_module_tls::browser::find("edge-134").unwrap();
    assert_eq!(chrome.expected_ja4, edge.expected_ja4);
    assert_ne!(chrome.browser_ja4, edge.browser_ja4);
}

#[tokio::test]
async fn tshark_agrees_with_the_expected_ja4() {
    if skip_without(
        "tshark 4.2 or newer",
        tool_available("tshark", &["--version"]),
    ) {
        return;
    }
    for profile in &BROWSER_PROFILES {
        let log = handshake(profile).await;
        let path = out_dir().join(format!("{}.pcap", profile.name));
        std::fs::write(&path, pcap(&log)).unwrap();
        let out = Command::new("tshark")
            .arg("-r")
            .arg(&path)
            .args([
                "-Y",
                "tls.handshake.type == 1",
                "-T",
                "fields",
                "-e",
                "tls.handshake.ja4",
            ])
            .output()
            .expect("tshark starts");
        assert!(
            out.status.success(),
            "tshark failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            text.trim(),
            profile.expected_ja4,
            "{}: tshark computes another JA4 than the profile expects",
            profile.name
        );
    }
}

/// The IANA code of a suite name that the module uses.
fn iana_code(name: &str) -> u16 {
    match name {
        "TLS_AES_128_GCM_SHA256" => 0x1301,
        "TLS_AES_256_GCM_SHA384" => 0x1302,
        "TLS_CHACHA20_POLY1305_SHA256" => 0x1303,
        "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" => 0xc02b,
        "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" => 0xc02c,
        "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" => 0xc02f,
        "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" => 0xc030,
        "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" => 0xcca8,
        "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" => 0xcca9,
        other => panic!("no code for {other}"),
    }
}
