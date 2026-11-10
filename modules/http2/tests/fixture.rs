// SPDX-License-Identifier: GPL-3.0-only
//! The committed fixture `fixtures/http2.pcapng` holds HTTP/2 in the clear and
//! HTTP/2 over TLS. nDPI classifies the TLS sessions as TLS, because it cannot
//! read inside them, so `fixtures/decrypted_check.py` decrypts the capture with
//! the keys that it embeds, scores the plaintext with nDPI, and counts the HTTP
//! messages that tshark parses inside the TLS sessions. This test runs it.
//!
//! It needs python3, tshark 4.2 or newer, and an ndpiReader that knows HTTP/2
//! (`FPS_NDPI_READER` selects it; nDPI 4.2.0 has no HTTP/2 protocol), and it
//! skips itself when one is missing. `FPS_REQUIRE_CAPTURE_TOOLS=1` turns a missing tool into
//! a failure, as in the tests of the TLS module.

use std::path::Path;
use std::process::Command;

fn available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

/// Whether the ndpiReader lists the HTTP2 protocol in its help text.
fn knows_http2(reader: &str) -> bool {
    Command::new(reader)
        .arg("-h")
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
}

#[test]
fn nearly_every_decrypted_session_is_http2() {
    let reader = std::env::var("FPS_NDPI_READER").unwrap_or_else(|_| "ndpiReader".into());
    let mut missing: Vec<&str> = ["python3", "tshark"]
        .into_iter()
        .filter(|tool| !available(tool))
        .collect();
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
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let report = out.join("http2.decrypted.json");
    let result = Command::new("python3")
        .arg("-I")
        .arg(module.join("fixtures/decrypted_check.py"))
        .arg(module.join("fixtures/http2.pcapng"))
        .arg("--report")
        .arg(&report)
        .output()
        .expect("python3 starts");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "decrypted_check failed: {stderr}");
    let text = std::fs::read_to_string(&report).expect("the report exists");
    assert!(text.contains("\"passed\": true"), "{text}");
    // The fixture carries both https scenarios, 20 sessions each.
    assert!(text.contains("\"tls_sessions\": 40"), "{text}");
}
