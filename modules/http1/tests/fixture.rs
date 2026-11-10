// SPDX-License-Identifier: GPL-3.0-only
//! The committed fixture `fixtures/http1.pcapng` holds HTTP in the clear and
//! HTTP over TLS. nDPI classifies the TLS sessions as TLS, because it cannot
//! read inside them, so `python3 -m realism decrypted` decrypts the capture with
//! the keys that it embeds, scores the plaintext with nDPI, and lets tshark parse
//! the TLS sessions. This test runs it with the labels
//! `realism/examples/decrypted/http1.labels.yaml`.
//!
//! It needs python3 with PyYAML, tshark 4.2 or newer, and ndpiReader, and it
//! skips itself when one is missing. `FPS_REQUIRE_CAPTURE_TOOLS=1` turns a missing tool into
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

#[test]
fn nearly_every_decrypted_session_is_http() {
    let mut missing: Vec<&str> = ["python3", "tshark", "ndpiReader"]
        .into_iter()
        .filter(|tool| !available(tool))
        .collect();
    if !missing.contains(&"python3") && !python_has_yaml() {
        missing.push("PyYAML for python3");
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
    let report = out.join("http1.decrypted.json");
    let mut command = Command::new("python3");
    command
        .current_dir(&root)
        .args(["-m", "realism", "decrypted", "--pcap"])
        .arg(module.join("fixtures/http1.pcapng"))
        .arg("--labels")
        .arg(root.join("realism/examples/decrypted/http1.labels.yaml"))
        .args(["--sessions", "80", "--parsers", "tshark"])
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
    // The same fixture carries all four https scenarios, 20 sessions each.
    assert!(text.contains("\"tls_sessions\": 80"), "{text}");
}
