// SPDX-License-Identifier: GPL-3.0-only
//! Capture and key log tests.
//!
//! Each test produces a capture of one TLS session with its key log, then
//! checks that the exported keys decrypt it. `tshark_decrypts_captures_with_exported_keys`
//! needs tshark, and `python_decrypts_a_tls13_capture_with_exported_keys` needs
//! Python with the `cryptography` package. A test skips itself when its tool is
//! missing, unless the environment variable `FPS_REQUIRE_CAPTURE_TOOLS` is set to
//! `1`, which CI jobs that install the tools use so a missing tool fails.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use common::{pcap, Dir, Log, Tap};
use fps_io::memory::duplex;
use fps_module_tls::{
    ClientProfile, LeafKey, NssKeyLog, RunAuthority, ServerProfile, TlsAcceptor, TlsConnector,
    TlsVersion,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const URI: &str = "/fps-tls-sample";
const REQUEST: &str =
    "GET /fps-tls-sample HTTP/1.1\r\nHost: tls.example.test\r\nUser-Agent: fps-tls-test\r\n\r\n";
const RESPONSE: &str =
    "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nContent-Type: text/plain\r\n\r\nhello tshark";

struct Sample {
    pcap: PathBuf,
    keys: PathBuf,
}

/// Runs one HTTP exchange over TLS and writes `<name>.pcap` and `<name>.keys`
/// into the target temp directory of cargo.
async fn sample(version: TlsVersion, name: &str) -> Sample {
    let keys = Arc::new(NssKeyLog::in_memory());
    let ca = RunAuthority::generate("fps capture authority").unwrap();
    let identity = ca.issue(&["tls.example.test"], LeafKey::EcdsaP256).unwrap();
    let client = ClientProfile {
        versions: vec![version],
        alpn: vec![b"http/1.1".to_vec()],
        key_log: Some(keys.clone()),
        ..ClientProfile::default()
    };
    let server = ServerProfile {
        versions: vec![version],
        key_log: Some(keys.clone()),
        ..ServerProfile::default()
    };
    let connector = TlsConnector::new(&client, ca.root_store().unwrap()).unwrap();
    let acceptor = TlsAcceptor::new(&server, &identity).unwrap();

    let log: Log = Arc::default();
    let (c, s) = duplex(64 * 1024);
    let (c, s) = (
        Tap::new(c, Dir::ClientToServer, &log),
        Tap::new(s, Dir::ServerToClient, &log),
    );
    let (c, s) = tokio::join!(connector.connect(c, "tls.example.test"), acceptor.accept(s));
    let (mut c, mut s) = (c.unwrap(), s.unwrap());
    let serve = async {
        let mut buf = vec![0u8; REQUEST.len()];
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, REQUEST.as_bytes());
        s.write_all(RESPONSE.as_bytes()).await.unwrap();
        s.shutdown().await.unwrap();
    };
    let ask = async {
        c.write_all(REQUEST.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        c.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, RESPONSE.as_bytes());
    };
    tokio::join!(serve, ask);

    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let sample = Sample {
        pcap: dir.join(format!("{name}.pcap")),
        keys: dir.join(format!("{name}.keys")),
    };
    std::fs::write(&sample.pcap, pcap(&log)).unwrap();
    std::fs::write(&sample.keys, keys.contents()).unwrap();
    sample
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

fn tshark(capture: &Path, keys: Option<&Path>, filter: &str, field: &str) -> String {
    let mut cmd = Command::new("tshark");
    cmd.arg("-r").arg(capture);
    if let Some(keys) = keys {
        cmd.arg("-o")
            .arg(format!("tls.keylog_file:{}", keys.display()));
    }
    cmd.args(["-Y", filter, "-T", "fields", "-e", field]);
    let out = cmd.output().expect("tshark starts");
    assert!(
        out.status.success(),
        "tshark failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
async fn tshark_decrypts_captures_with_exported_keys() {
    if skip_without("tshark", tool_available("tshark", &["--version"])) {
        return;
    }
    for (version, name) in [(TlsVersion::Tls13, "tls13"), (TlsVersion::Tls12, "tls12")] {
        let s = sample(version, name).await;
        // The capture is TCP on port 443, which tshark hands to its TLS dissector and,
        // after decryption, to its HTTP dissector.
        let request = tshark(&s.pcap, Some(&s.keys), "http.request", "http.request.uri");
        assert!(
            request.contains(URI),
            "{name}: tshark decrypted the request, got {request:?}"
        );
        let response = tshark(
            &s.pcap,
            Some(&s.keys),
            "http.response",
            "http.response.code",
        );
        assert!(
            response.contains("200"),
            "{name}: tshark decrypted the response, got {response:?}"
        );
        // Without the key log the same capture stays opaque.
        let blind = tshark(&s.pcap, None, "http.request", "http.request.uri");
        assert!(
            !blind.contains(URI),
            "{name}: the capture must not decode without keys"
        );
    }
}

#[tokio::test]
async fn python_decrypts_a_tls13_capture_with_exported_keys() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/decrypt_tls13.py");
    let ready = tool_available("python3", &["-c", "import cryptography"]);
    if skip_without("python3 with the cryptography package", ready) {
        return;
    }
    let s = sample(TlsVersion::Tls13, "tls13-python").await;
    let run = |keys: &Path| {
        Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg(&s.pcap)
            .arg(keys)
            .output()
            .unwrap()
    };

    let out = run(&s.keys);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "decryption failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains(URI) && text.contains("hello tshark"),
        "both directions decrypt, got {text}"
    );

    // A key log with one flipped digit in each secret decrypts nothing.
    let broken = std::fs::read_to_string(&s.keys)
        .unwrap()
        .lines()
        .map(|l| {
            let (head, secret) = l.rsplit_once(' ').unwrap();
            let flipped = if secret.starts_with('0') { "1" } else { "0" };
            format!("{head} {flipped}{}", &secret[1..])
        })
        .collect::<Vec<_>>()
        .join("\n");
    let broken_path = s.keys.with_extension("broken");
    std::fs::write(&broken_path, broken).unwrap();
    let out = run(&broken_path);
    assert!(!out.status.success(), "a wrong key log must not decrypt");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(URI));
}
