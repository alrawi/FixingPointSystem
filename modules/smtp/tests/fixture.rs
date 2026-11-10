// SPDX-License-Identifier: GPL-3.0-only
//! The committed fixture `fixtures/smtp.pcapng` holds plain SMTP sessions and
//! the STARTTLS sessions of MOD-20. nDPI classifies the STARTTLS sessions as
//! SMTP from their plaintext start and never sees the message, and Zeek cannot
//! decrypt TLS, so `python3 -m realism decrypted` decrypts the capture with the
//! keys that it embeds, lets tshark parse the dialogue inside TLS, scores a
//! decrypted view with nDPI, and reads the view with Zeek when Zeek is
//! installed. This test runs it on the committed capture with the labels
//! `realism/examples/decrypted/smtp.labels.yaml`.
//!
//! It needs python3 with PyYAML, tshark 4.2 or newer, and ndpiReader, and it
//! skips itself when one is missing. `FPS_REQUIRE_CAPTURE_TOOLS=1` turns a missing tool into
//! a failure, as in the tests of the TLS module. Zeek is optional here. A job that
//! installs Zeek, such as the `module` job of the smtp workflow once it has the
//! step that the README names, runs the same command with `--zeek`.

use std::path::Path;
use std::process::Command;

/// The sessions per scenario of `capture.sh`, which is also the number of
/// STARTTLS sessions in the capture.
const SESSIONS: &str = "8";

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

fn zeek() -> Option<String> {
    std::env::var("FPS_ZEEK")
        .ok()
        .into_iter()
        .chain(["zeek".to_owned(), "/opt/zeek/bin/zeek".to_owned()])
        .find(|program| available(program))
}

#[test]
fn every_starttls_session_decrypts_and_scores() {
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
    let report = out.join("smtp.decrypted.json");
    let zeek = zeek();
    let parsers = if zeek.is_some() {
        "tshark,zeek"
    } else {
        "tshark"
    };
    let mut command = Command::new("python3");
    command
        .current_dir(&root)
        .args(["-m", "realism", "decrypted", "--pcap"])
        .arg(module.join("fixtures/smtp.pcapng"))
        .arg("--labels")
        .arg(root.join("realism/examples/decrypted/smtp.labels.yaml"))
        .args([
            "--sessions",
            SESSIONS,
            "--plaintext",
            "upgrade",
            "--view-port",
            "587",
            "--parsers",
            parsers,
        ])
        .arg("--output")
        .arg(&report);
    if let Some(program) = &zeek {
        command.args(["--zeek", program]);
    }
    let result = command.output().expect("python3 starts");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "realism decrypted failed: {stderr}"
    );
    let text = std::fs::read_to_string(&report).expect("the report exists");
    assert!(text.contains("\"passed\": true"), "{text}");
    assert!(text.contains("\"tls_sessions\": 8"), "{text}");
}
