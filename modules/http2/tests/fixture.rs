// SPDX-License-Identifier: GPL-3.0-only
//! The committed fixture `fixtures/http2.pcapng` holds HTTP/2 in the clear and
//! HTTP/2 over TLS. nDPI classifies the TLS sessions as TLS, because it cannot
//! read inside them, so `python3 -m realism decrypted` decrypts the capture with
//! the keys that it embeds, scores the plaintext with nDPI, and lets tshark parse
//! the TLS sessions. This test runs it with the labels
//! `realism/examples/decrypted/http2.labels.yaml`.
//!
//! It needs python3 with PyYAML, tshark 4.2 or newer, and an ndpiReader that
//! knows HTTP/2 (`FPS_NDPI_READER` selects it; nDPI 4.2.0 has no HTTP/2
//! protocol), and it skips itself when one is missing. `FPS_REQUIRE_CAPTURE_TOOLS=1` turns a missing tool into
//! a failure, as in the tests of the TLS module.

use std::path::Path;
use std::process::Command;

fn available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

/// Whether python3 can import PyYAML, which the labels files of the realism runner need.
fn python_has_yaml() -> bool {
    Command::new("python3")
        .args(["-c", "import yaml"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Whether the ndpiReader lists the HTTP2 protocol, in its help text (nDPI 4.2)
/// or with `-H` (newer nDPI).
fn knows_http2(reader: &str) -> bool {
    ["-h", "-H"].iter().any(|flag| {
        Command::new(reader)
            .arg(flag)
            .output()
            .map(|out| {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                text.lines()
                    .any(|line| line.split_whitespace().any(|word| word == "HTTP2"))
            })
            .unwrap_or(false)
    })
}

#[test]
fn nearly_every_decrypted_session_is_http2() {
    let reader = std::env::var("FPS_NDPI_READER").unwrap_or_else(|_| "ndpiReader".into());
    let mut missing: Vec<&str> = ["python3", "tshark"]
        .into_iter()
        .filter(|tool| !available(tool))
        .collect();
    if !missing.contains(&"python3") && !python_has_yaml() {
        missing.push("PyYAML for python3");
    }
    if !knows_http2(&reader) {
        missing.push("an ndpiReader that knows HTTP/2");
    }
    if !missing.is_empty() {
        let message = format!("{} not installed", missing.join(", "));
        assert!(
            std::env::var_os("FPS_REQUIRE_CAPTURE_TOOLS").is_none(),
            "{message}"
        );
        eprintln!("skipped: {message}");
        return;
    }
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    // The runner is importable from the repository root, so the command runs there.
    let root = module.join("../..");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let report = out.join("http2.decrypted.json");
    let mut command = Command::new("python3");
    command
        .current_dir(&root)
        .args(["-m", "realism", "decrypted", "--pcap"])
        .arg(module.join("fixtures/http2.pcapng"))
        .arg("--labels")
        .arg(root.join("realism/examples/decrypted/http2.labels.yaml"))
        .args(["--sessions", "40", "--parsers", "tshark"])
        .arg("--output")
        .arg(&report);
    let result = command.output().expect("python3 starts");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "realism decrypted failed: {stderr}"
    );
    let text = std::fs::read_to_string(&report).expect("the report exists");
    assert!(text.contains("\"passed\": true"), "{text}");
    // The fixture carries both https scenarios, 20 sessions each.
    assert!(text.contains("\"tls_sessions\": 40"), "{text}");
}
