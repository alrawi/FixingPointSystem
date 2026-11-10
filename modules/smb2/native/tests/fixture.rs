// SPDX-License-Identifier: GPL-3.0-only
//! The committed fixture `fixtures/smb2.pcapng` holds 18 sessions, six of each
//! scenario, that `fixtures/capture.sh` records in the QA-01 harness. This test
//! scores it with the realism runner and the labels `fixtures/expected.yaml`:
//! nDPI must classify at least 99% of the sessions as SMB, and tshark must parse
//! every message of them without a malformed packet or an expert error.
//!
//! The test needs python3 with PyYAML, tshark 4.2 or newer, and ndpiReader, and
//! it skips itself when one is missing. `FPS_REQUIRE_CAPTURE_TOOLS=1` turns a
//! missing tool into a failure, as in the other tests of this crate.

use std::path::Path;
use std::process::Command;

/// The sessions per scenario of `capture.sh`.
const SESSIONS_PER_SCENARIO: usize = 6;
/// The scenarios of the manifest.
const SCENARIOS: usize = 3;

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
fn nearly_every_session_of_the_fixture_is_smb() {
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
    let report = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smb2.report.json");
    let result = Command::new("python3")
        .current_dir(&root)
        .args(["-m", "realism", "score", "--pcap"])
        .arg(module.join("fixtures/smb2.pcapng"))
        .arg("--labels")
        .arg(module.join("fixtures/expected.yaml"))
        .args(["--parsers", "tshark", "--output"])
        .arg(&report)
        .output()
        .expect("python3 starts");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "realism score failed: {stderr}");
    let text = std::fs::read_to_string(&report).expect("the report exists");
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).expect("the report is JSON");
    assert_eq!(doc["passed"].as_bool(), Some(true), "{text}");
    let sessions = SESSIONS_PER_SCENARIO * SCENARIOS;
    assert_eq!(
        doc["capture"]["sessions"].as_u64(),
        Some(sessions as u64),
        "{text}"
    );
    let conformance = doc["checks"]
        .as_sequence()
        .and_then(|checks| {
            checks
                .iter()
                .find(|c| c["check"].as_str() == Some("conformance"))
        })
        .expect("the report has a conformance check");
    let app = &conformance["applications"][0];
    assert_eq!(app["application"].as_str(), Some("smb2"), "{text}");
    assert_eq!(app["sessions"].as_u64(), Some(sessions as u64), "{text}");
    assert!(app["ratio"].as_f64().unwrap() >= 0.99, "{text}");
}
