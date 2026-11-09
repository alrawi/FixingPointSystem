// SPDX-License-Identifier: GPL-3.0-only
//! The committed fixture `fixtures/http1.pcapng` holds HTTP in the clear and
//! HTTP over TLS. nDPI classifies the TLS sessions as TLS, because it cannot
//! read inside them, so `fixtures/decrypted_check.py` decrypts the capture with
//! the keys that it embeds, scores the plaintext with nDPI, and counts the HTTP
//! messages that tshark parses inside the TLS sessions. This test runs it.
//!
//! It needs python3, tshark 4.2 or newer, and ndpiReader, and it skips itself
//! when one is missing. `FPS_REQUIRE_CAPTURE_TOOLS=1` turns a missing tool into
//! a failure, as in the tests of the TLS module.

use std::path::Path;
use std::process::Command;

fn available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

#[test]
fn nearly_every_decrypted_session_is_http() {
    let missing: Vec<&str> = ["python3", "tshark", "ndpiReader"]
        .into_iter()
        .filter(|tool| !available(tool))
        .collect();
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
    let report = out.join("http1.decrypted.json");
    let result = Command::new("python3")
        .arg("-I")
        .arg(module.join("fixtures/decrypted_check.py"))
        .arg(module.join("fixtures/http1.pcapng"))
        .arg("--report")
        .arg(&report)
        .output()
        .expect("python3 starts");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "decrypted_check failed: {stderr}");
    let text = std::fs::read_to_string(&report).expect("the report exists");
    assert!(text.contains("\"passed\": true"), "{text}");
    // The same fixture carries all four https scenarios, 20 sessions each.
    assert!(text.contains("\"tls_sessions\": 80"), "{text}");
}
