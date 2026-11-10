// SPDX-License-Identifier: GPL-3.0-only
//! The mailbox scenario with STARTTLS (MOD-20).
//!
//! The scenario runs with both sides over the in memory duplex and the TLS
//! profiles of `common/tls.rs`. The tests check on the dialogue, on the bytes
//! of the wire, and on the handshake reports that
//!
//! * the greeting and the first CAPABILITY answer list STARTTLS and
//!   LOGINDISABLED, the client sends `A002 STARTTLS`, the server answers `OK`,
//!   and only then does the handshake start,
//! * the CAPABILITY inside TLS lists AUTH=PLAIN and no STARTTLS, and the LOGIN,
//!   the SELECT, and the FETCH commands follow with the tags that count on,
//! * the handshake negotiates TLS 1.2 with the SNI of the scenario, no ALPN id,
//!   and the profile that the scenario names, and the ClientHello has the JA4
//!   of that profile, which is the value of the labels file of the fixture,
//! * the credentials and the messages do not cross the wire in the clear, and
//! * the key log of the client names the ClientHello random of the wire and
//!   holds the master secret that a decrypting reader needs.
//!
//! The JA4 test runs `modules/tls/tests/ja4_of_hello.py`, which needs only
//! `python3`.

mod common;

use std::path::Path;
use std::process::Command;

use common::tls::{SERVER_NAME, SERVER_TLS12};
use common::{all_scenarios_with, run, uses_tls, Run, TlsRun};
use fps_flow::program::{Op, TlsRole};
use fps_module_tls::browser;
use fps_module_tls::TlsVersion;

const SCENARIO: &str = "starttls_mailbox";
const PROFILE: &str = "safari-18.5";

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs the scenario and returns its run, which must have succeeded over TLS.
async fn run_tls(session_id: u64) -> (Run, TlsRun) {
    let all = all_scenarios_with(&[]);
    let (_, s) = all.iter().find(|(n, _)| n == SCENARIO).expect("scenario");
    let mut r = run(s, 42, session_id).await;
    assert!(r.client.is_ok(), "client: {:?}", r.client);
    assert!(r.server.is_ok(), "server: {:?}", r.server);
    let tls = r.tls.take().expect("a session over TLS");
    (r, tls)
}

#[test]
fn the_manifest_lists_the_scenario_and_it_upgrades_once_per_side() {
    let all = all_scenarios_with(&[]);
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names.last(), Some(&SCENARIO), "{names:?}");
    let (_, s) = all.iter().find(|(n, _)| n == SCENARIO).expect("listed");
    assert_eq!(
        s.vars.get("server_port"),
        Some(&fps_flow::Value::Int(143)),
        "the IMAP port"
    );
    // Only this scenario upgrades.
    for (name, scenario) in &all {
        assert_eq!(uses_tls(scenario), name == SCENARIO, "{name}");
    }
    let flow = &s.flows[0];
    let upgrades = |ops: &[Op]| {
        ops.iter()
            .filter_map(|op| match op {
                Op::UpgradeTls(u) => Some(u.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let client = upgrades(&flow.client.ops);
    assert_eq!(client.len(), 1);
    assert_eq!(client[0].profile, PROFILE);
    assert_eq!(client[0].server_name.as_deref(), Some(SERVER_NAME));
    assert!(client[0].role.is_none() || client[0].role == Some(TlsRole::Client));
    let server = upgrades(&flow.server.as_ref().expect("server side").ops);
    assert_eq!(server.len(), 1);
    assert_eq!(server[0].profile, SERVER_TLS12);
    // The variable that names the server and the SNI that the client sends agree.
    assert_eq!(
        s.vars.get("server_name"),
        Some(&fps_flow::Value::Str(SERVER_NAME.into()))
    );
    assert!(
        browser::find(PROFILE).is_some(),
        "{PROFILE} is not a browser profile"
    );
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("the dialogue is ASCII up to the literals")
}

/// The command lines of a client stream, which hold no literals in this scenario.
fn commands(bytes: &[u8]) -> Vec<&str> {
    text(bytes).split_terminator("\r\n").collect()
}

#[tokio::test(start_paused = true)]
async fn the_dialogue_upgrades_after_the_ok_and_logs_in_inside_tls() {
    for session_id in 1..=3 {
        let (r, tls) = run_tls(session_id).await;
        let (c_plain, c_secure) = r.c2s.split_at(tls.plain_before_c2s);
        let (s_plain, s_secure) = r.s2c.split_at(tls.plain_before_s2c);

        // Before the upgrade: CAPABILITY and STARTTLS.
        assert_eq!(commands(c_plain), ["A001 CAPABILITY", "A002 STARTTLS"]);
        let server = text(s_plain);
        let lines: Vec<&str> = server.split_terminator("\r\n").collect();
        assert!(lines[0].starts_with("* OK [CAPABILITY IMAP4rev1 "));
        assert!(
            lines[0].contains(" STARTTLS LOGINDISABLED]"),
            "the greeting offers STARTTLS: {}",
            lines[0]
        );
        assert!(lines[1].starts_with("* CAPABILITY IMAP4rev1 "));
        assert!(
            lines[1].ends_with(" STARTTLS LOGINDISABLED"),
            "{}",
            lines[1]
        );
        assert!(lines[2].starts_with("A001 OK Pre-login capabilities listed"));
        assert_eq!(lines[3], "A002 OK Begin TLS negotiation now.");
        assert_eq!(lines.len(), 4, "nothing follows the OK in the clear");

        // After the upgrade: the commands with tags that continue the count.
        let cmds = commands(c_secure);
        assert_eq!(cmds[0], "A003 CAPABILITY");
        assert!(cmds[1].starts_with("A004 LOGIN \""), "{}", cmds[1]);
        assert_eq!(cmds[2], "A005 SELECT \"INBOX\"");
        assert_eq!(cmds[3], "A006 FETCH 1:6 (UID FLAGS RFC822.SIZE)");
        assert!(cmds[4].starts_with("A007 FETCH "), "{}", cmds[4]);
        assert!(cmds[4].ends_with(" (UID FLAGS BODY.PEEK[])"), "{}", cmds[4]);
        assert!(cmds[5].starts_with("A008 FETCH "), "{}", cmds[5]);
        assert_eq!(cmds[6], "A009 LOGOUT");
        assert_eq!(cmds.len(), 7);
        assert!(
            !cmds.iter().any(|c| c.contains("STARTTLS")),
            "no second STARTTLS"
        );

        // The capability list inside TLS offers AUTH=PLAIN and no STARTTLS.
        let reply = text(
            &s_secure
                [..find(s_secure, b"A003 OK").expect("the capability answer") + b"A003 OK".len()],
        );
        assert!(reply.starts_with("* CAPABILITY IMAP4rev1 "), "{reply}");
        assert!(reply.contains(" AUTH=PLAIN\r\n"), "{reply}");
        assert!(!reply.contains("STARTTLS") && !reply.contains("LOGINDISABLED"));
        assert!(contains(s_secure, b"A004 OK [CAPABILITY "));
        assert!(contains(s_secure, b"A009 OK Logout completed"));
    }
}

/// The literals that the server sent in `FETCH` responses with `BODY[]`.
fn literals(s2c: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut rest = s2c;
    while let Some(at) = find(rest, b" BODY[] {") {
        let start = at + b" BODY[] {".len();
        let end = start + find(&rest[start..], b"}\r\n").expect("the end of the marker");
        let size: usize = text(&rest[start..end]).parse().expect("a literal size");
        let body = &rest[end + 3..end + 3 + size];
        out.push(body);
        rest = &rest[end + 3 + size..];
    }
    out
}

#[tokio::test(start_paused = true)]
async fn the_downloaded_messages_inside_tls_carry_their_attachments() {
    let mut sizes = Vec::new();
    for session_id in 1..=8 {
        let (r, _) = run_tls(session_id).await;
        let messages = literals(&r.s2c);
        assert_eq!(messages.len(), 2, "the variable fetches");
        for m in messages {
            let message = text(m);
            assert!(message.starts_with("Date: ") || message.starts_with("From: "));
            assert!(message.contains("\r\nSubject: Documents "));
            assert!(message.contains("Content-Type: multipart/mixed; boundary="));
            let kind = ["statement.pdf", "photo.jpg", "archive.zip"]
                .into_iter()
                .filter(|f| message.contains(&format!("filename=\"{f}\"")))
                .count();
            assert_eq!(kind, 1, "exactly one attachment");
            sizes.push(m.len());
        }
    }
    sizes.sort_unstable();
    sizes.dedup();
    assert!(sizes.len() >= 10, "message sizes do not vary: {sizes:?}");
}

#[tokio::test(start_paused = true)]
async fn the_handshake_negotiates_tls12_without_alpn() {
    for session_id in 1..=3 {
        let (_, tls) = run_tls(session_id).await;
        for report in [&tls.client, &tls.server] {
            assert_eq!(report.version, TlsVersion::Tls12);
            assert_eq!(report.alpn, None, "a mail server selects no ALPN id");
            assert_eq!(report.server_name.as_deref(), Some(SERVER_NAME));
            assert!(!report.resumed);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_wire_holds_the_hello_and_hides_the_credentials_and_messages() {
    let (r, tls) = run_tls(1).await;
    // A handshake record, TLS 1.0 in the record version, with a ClientHello inside.
    assert_eq!(&tls.wire_c2s[..3], &[0x16, 0x03, 0x01]);
    assert_eq!(
        tls.wire_c2s[5], 0x01,
        "the records start with a ClientHello"
    );
    assert!(contains(&tls.wire_c2s, SERVER_NAME.as_bytes()), "SNI");
    // The login line names the user and the password in the plaintext.
    let (_, secure) = r.c2s.split_at(tls.plain_before_c2s);
    let login = commands(secure)[1];
    let fields: Vec<&str> = login.split('"').collect();
    let (user, password) = (fields[1], fields[3]);
    assert!(user.ends_with("@example.test") && password.starts_with("Tst-"));
    // Every needle has six bytes or more, so a chance match in random records
    // is out of the question.
    for (wire, what) in [(&tls.wire_c2s, "client"), (&tls.wire_s2c, "server")] {
        for secret in [
            user,
            password,
            "A003 CAPABILITY",
            "LOGIN \"",
            "SELECT \"INBOX\"",
            "UID FLAGS",
            "BODY.PEEK",
            "Subject:",
            "Content-Type",
            "AUTH=PLAIN",
            "Logged in",
            "A009 LOGOUT",
            "Logout completed",
        ] {
            assert!(
                !contains(wire, secret.as_bytes()),
                "the {what} records leak {secret}"
            );
        }
    }
    // TLS 1.2 sends the certificate in the clear, so the name of the server
    // is in the records of the server.
    assert!(contains(&tls.wire_s2c, SERVER_NAME.as_bytes()));
    // The records carry more than the plaintext above them.
    assert!(tls.wire_c2s.len() > r.c2s.len() - tls.plain_before_c2s);
    assert!(tls.wire_s2c.len() > r.s2c.len() - tls.plain_before_s2c);
}

/// The ClientHello random, which the key log names in every line.
fn client_random(wire_c2s: &[u8]) -> &[u8] {
    // Record header 5, handshake header 4, legacy version 2.
    &wire_c2s[11..43]
}

#[tokio::test(start_paused = true)]
async fn the_key_log_names_the_hello_and_holds_the_master_secret() {
    let (_, tls) = run_tls(1).await;
    let random = hex(client_random(&tls.wire_c2s));
    let lines: Vec<&str> = tls.key_log.lines().collect();
    assert_eq!(lines.len(), 1, "TLS 1.2 logs one line: {lines:?}");
    let parts: Vec<&str> = lines[0].split(' ').collect();
    assert_eq!(parts[0], "CLIENT_RANDOM");
    assert_eq!(parts[1], random);
    assert_eq!(parts[2].len(), 96, "a master secret has 48 bytes");
    assert!(parts[2].bytes().all(|b| b.is_ascii_hexdigit()));
}

#[tokio::test(start_paused = true)]
async fn the_client_hello_has_the_ja4_of_its_profile() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = manifest.join("../../tls/tests/ja4_of_hello.py");
    let root = manifest.join("../../..");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir).expect("scratch directory");
    let (_, tls) = run_tls(1).await;
    let file = dir.join("imap_starttls.hello");
    std::fs::write(&file, &tls.wire_c2s).expect("write the wire bytes");
    let out = Command::new("python3")
        .arg("-I")
        .arg(&script)
        .arg(&root)
        .arg(&file)
        .output()
        .expect("python3 starts");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let field = |key: &str| {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    let expected = browser::find(PROFILE).expect("profile").expected_ja4;
    assert_eq!(field("ja4"), expected);
    // The profile is a browser profile, so it offers the ALPN ids of a
    // browser. Apple Mail sends none to a mail server, a gap that the README
    // names.
    assert_eq!(field("alpn"), "h2,http/1.1");
    assert_eq!(field("server_name"), SERVER_NAME);
}
