// SPDX-License-Identifier: GPL-3.0-only
//! `fixtures/tshark-tls-ports.sh` is the tshark that `fixtures/capture.sh` gives to
//! `python3 -m realism decrypted`. The decrypted check counts the TLS sessions of the
//! capture as the streams with a ClientHello, and tshark finds a ClientHello on the
//! ports 8443 and 8444 by its heuristic only, which a ClientHello whose first TCP
//! segment is shorter than a record header escapes: the module job of the realism
//! workflow once recorded 39 of 40 TLS sessions (MOD-23). The script makes the ports
//! TLS with Decode As for the capture, and leaves every other file alone, because the
//! decrypted view keeps those ports and holds HTTP/2 in the clear on them.
//!
//! The first tests run the script against a stand-in for tshark that prints its
//! arguments, so they need nothing but bash. The last one reads the committed fixture
//! with the real tshark and skips itself without one.
//! `FPS_REQUIRE_CAPTURE_TOOLS=1` turns the missing tool into a failure.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn wrapper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/tshark-tls-ports.sh")
}

/// Writes a tshark that prints one argument per line and exits with 7.
fn stand_in(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::write(
        &path,
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\"\nexit 7\n",
    )
    .expect("write stand-in");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stand-in");
    path
}

fn run(stand_in: &Path, decode: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(wrapper());
    command
        .env("FPS_TSHARK_REAL", stand_in)
        .env("FPS_TLS_PORTS", "8443 8444")
        .env_remove("FPS_TLS_DECODE_PCAP")
        .args(args);
    if let Some(file) = decode {
        command.env("FPS_TLS_DECODE_PCAP", file);
    }
    command.output().expect("bash starts")
}

fn printed(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_capture_is_read_with_its_ports_as_tls() {
    let peer = stand_in("tshark-stand-in-capture");
    let out = run(
        &peer,
        Some("/run/http2.pcapng"),
        &["-r", "/run/http2.pcapng", "-Y", "tls.handshake.type == 1"],
    );
    assert_eq!(
        out.status.code(),
        Some(7),
        "the exit code of tshark passes through"
    );
    assert_eq!(
        printed(&out),
        [
            "-d",
            "tcp.port==8443,tls",
            "-d",
            "tcp.port==8444,tls",
            "-r",
            "/run/http2.pcapng",
            "-Y",
            "tls.handshake.type == 1",
        ]
    );
}

#[test]
fn another_file_is_read_unchanged() {
    // The decrypted view keeps the ports and holds HTTP/2 in the clear on them.
    let peer = stand_in("tshark-stand-in-view");
    let args = ["-r", "/run/http2.decrypted.pcap", "-2", "-q"];
    let out = run(&peer, Some("/run/http2.pcapng"), &args);
    assert_eq!(printed(&out), args);
}

#[test]
fn without_a_capture_to_decode_nothing_is_added() {
    let peer = stand_in("tshark-stand-in-none");
    let args = ["--version"];
    let out = run(&peer, None, &args);
    assert_eq!(printed(&out), args);
}

fn available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

#[test]
fn the_committed_fixture_lists_its_40_tls_streams() {
    if !available("tshark") {
        assert!(
            std::env::var_os("FPS_REQUIRE_CAPTURE_TOOLS").is_none(),
            "tshark not installed"
        );
        eprintln!("skipped: tshark not installed");
        return;
    }
    let capture =
        fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/http2.pcapng"))
            .expect("the fixture exists");
    let out = Command::new(wrapper())
        .env("FPS_TLS_PORTS", "8443 8444")
        .env("FPS_TLS_DECODE_PCAP", &capture)
        .arg("-r")
        .arg(&capture)
        .args([
            "-Y",
            "tls.handshake.type == 1",
            "-T",
            "fields",
            "-e",
            "tcp.stream",
        ])
        .output()
        .expect("bash starts");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut streams = printed(&out);
    streams.sort();
    streams.dedup();
    assert_eq!(streams.len(), 40, "{streams:?}");
}

/// Counts the streams with a ClientHello that `program` lists in `capture`.
fn hello_streams(program: &Path, capture: &Path, decode: bool) -> usize {
    let mut command = Command::new(program);
    command.env("FPS_TLS_PORTS", "8443 8444");
    if decode {
        command.env("FPS_TLS_DECODE_PCAP", capture);
    }
    let out = command
        .arg("-r")
        .arg(capture)
        .args([
            "-Y",
            "tls.handshake.type == 1",
            "-T",
            "fields",
            "-e",
            "tcp.stream",
        ])
        .output()
        .expect("tshark starts");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut streams = printed(&out);
    streams.sort();
    streams.dedup();
    streams.len()
}

#[test]
fn a_client_hello_that_starts_in_a_short_segment_still_counts() {
    let missing: Vec<&str> = ["tshark", "editcap", "python3"]
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
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests");
    let fixture = tests.join("../fixtures/http2.pcapng");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let classic = out.join("http2.classic.pcap");
    let split = out.join("http2.split.pcap");
    let converted = Command::new("editcap")
        .args(["-F", "pcap"])
        .arg(&fixture)
        .arg(&classic)
        .output()
        .expect("editcap starts");
    assert!(
        converted.status.success(),
        "{}",
        String::from_utf8_lossy(&converted.stderr)
    );
    // One byte of the first ClientHello in the first segment, the rest in the second.
    let cut = Command::new("python3")
        .arg("-I")
        .arg(tests.join("split_client_hello.py"))
        .arg(&classic)
        .arg(&split)
        .arg("1")
        .output()
        .expect("python3 starts");
    assert!(
        cut.status.success(),
        "{}",
        String::from_utf8_lossy(&cut.stderr)
    );

    // The heuristic of tshark may miss the split ClientHello (tshark 4.2 does), so the
    // count without Decode As is for the log only.
    let plain = hello_streams(Path::new("tshark"), &split, false);
    eprintln!("tshark without Decode As lists {plain} of 40 TLS streams");
    assert_eq!(hello_streams(&wrapper(), &split, true), 40);
}
