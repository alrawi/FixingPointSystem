// SPDX-License-Identifier: GPL-3.0-only
//! Exit codes of `fps-agent connect` for the startup kernel check and the
//! redial limit of decision D12 items 3 and 11.

use std::path::PathBuf;
use std::process::Command;

fn connect(extra: &[&str]) -> std::process::Output {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let modules = manifest.join("../../contracts/examples/modules");
    match Command::new(env!("CARGO_BIN_EXE_fps-agent"))
        .args(["connect", "--controller", "http://127.0.0.1:1"])
        .args(["--agent-id", "exit-codes", "--modules"])
        .arg(modules)
        .args(extra)
        .output()
    {
        Ok(out) => out,
        Err(e) => panic!("run fps-agent: {e}"),
    }
}

#[test]
fn an_old_kernel_stops_the_agent_before_it_dials() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/host-no-xdp");
    let out = connect(&["--root", &root.to_string_lossy()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("IP_LOCAL_PORT_RANGE"), "{stderr}");
    assert!(stderr.contains("5.4.0-150-custom"), "{stderr}");
    assert!(!stderr.contains("session lost"), "dialed anyway: {stderr}");
}

#[test]
fn an_unreachable_controller_ends_the_agent_with_exit_code_3_after_the_limit() {
    if fps_agent::require_kernel(&fps_agent::SystemRoot::host()).is_err() {
        // The host itself is too old, which the first test covers.
        return;
    }
    let out = connect(&["--redial-limit", "1"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("gave up redialing"), "{stderr}");
}

#[test]
fn a_bad_redial_limit_is_a_usage_error() {
    let out = connect(&["--redial-limit", "soon"]);
    assert_eq!(out.status.code(), Some(2));
}
