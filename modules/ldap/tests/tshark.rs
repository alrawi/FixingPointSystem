// SPDX-License-Identifier: GPL-3.0-only
//! The acceptance test of the module: tshark parses every message that the
//! functions encode without an error.
//!
//! The test runs every scenario, writes the messages of the session into a
//! classic pcap file as TCP segments between a client and a server on port
//! 389, one message to a segment in the order of the message ids, and runs
//! tshark over it. tshark must dissect every message as LDAP without a
//! malformed packet and without an error or warning of its expert system, and
//! the fields that it decodes must equal the values that the messages carry.
//!
//! The test skips itself when tshark is missing, unless the environment
//! variable `FPS_REQUIRE_CAPTURE_TOOLS` is `1`, which the CI job with tshark sets.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{run, scenarios, SEED};
use fps_module_ldap::ber;
use fps_module_ldap::message::{Message, Op};

const CLIENT: [u8; 4] = [10, 0, 0, 2];
const SERVER: [u8; 4] = [10, 0, 0, 1];
const CLIENT_PORT: u16 = 49_152;
const SERVER_PORT: u16 = 389;
const MSS: usize = 1400;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    ToServer,
    ToClient,
}

fn tshark_available() -> bool {
    Command::new("tshark")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn skip_without_tshark() -> bool {
    if tshark_available() {
        return false;
    }
    assert!(
        std::env::var("FPS_REQUIRE_CAPTURE_TOOLS").as_deref() != Ok("1"),
        "tshark is required by FPS_REQUIRE_CAPTURE_TOOLS=1 but is not available"
    );
    eprintln!("skipping: tshark is not available");
    true
}

/// A classic pcap file with Ethernet frames, written one TCP segment at a time.
struct Pcap {
    bytes: Vec<u8>,
    time: u32,
    client_seq: u32,
    server_seq: u32,
    ip_id: u16,
}

impl Pcap {
    fn new() -> Pcap {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xA1B2_C3D4_u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&262_144u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        Pcap {
            bytes,
            time: 1_760_000_000,
            client_seq: 1000,
            server_seq: 5000,
            ip_id: 1,
        }
    }

    /// Appends one segment. `flags` are the TCP flags.
    fn segment(&mut self, dir: Dir, flags: u8, payload: &[u8]) {
        let (src, dst, sport, dport, seq, ack) = match dir {
            Dir::ToServer => (
                CLIENT,
                SERVER,
                CLIENT_PORT,
                SERVER_PORT,
                self.client_seq,
                self.server_seq,
            ),
            Dir::ToClient => (
                SERVER,
                CLIENT,
                SERVER_PORT,
                CLIENT_PORT,
                self.server_seq,
                self.client_seq,
            ),
        };
        // A SYN carries no acknowledgment.
        let ack = if flags == 0x02 { 0 } else { ack };
        let mut frame = Vec::new();
        // Ethernet, with two locally administered addresses.
        let (mac_src, mac_dst) = match dir {
            Dir::ToServer => ([2, 0, 0, 0, 0, 2], [2, 0, 0, 0, 0, 1]),
            Dir::ToClient => ([2, 0, 0, 0, 0, 1], [2, 0, 0, 0, 0, 2]),
        };
        frame.extend_from_slice(&mac_dst);
        frame.extend_from_slice(&mac_src);
        frame.extend_from_slice(&0x0800u16.to_be_bytes());
        // IPv4 without options. tshark does not check the checksums by default.
        let total = 20 + 20 + payload.len();
        frame.extend_from_slice(&[0x45, 0]);
        frame.extend_from_slice(&(total as u16).to_be_bytes());
        frame.extend_from_slice(&self.ip_id.to_be_bytes());
        self.ip_id = self.ip_id.wrapping_add(1);
        frame.extend_from_slice(&[0x40, 0, 64, 6, 0, 0]);
        frame.extend_from_slice(&src);
        frame.extend_from_slice(&dst);
        // TCP without options.
        frame.extend_from_slice(&sport.to_be_bytes());
        frame.extend_from_slice(&dport.to_be_bytes());
        frame.extend_from_slice(&seq.to_be_bytes());
        frame.extend_from_slice(&ack.to_be_bytes());
        frame.extend_from_slice(&[0x50, flags]);
        frame.extend_from_slice(&65_535u16.to_be_bytes());
        frame.extend_from_slice(&[0, 0, 0, 0]);
        frame.extend_from_slice(payload);
        self.time += 1;
        self.bytes.extend_from_slice(&self.time.to_le_bytes());
        self.bytes.extend_from_slice(&0u32.to_le_bytes());
        self.bytes
            .extend_from_slice(&(frame.len() as u32).to_le_bytes());
        self.bytes
            .extend_from_slice(&(frame.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(&frame);
        let advance = payload.len() as u32 + u32::from(flags & 0x03 != 0);
        match dir {
            Dir::ToServer => self.client_seq = self.client_seq.wrapping_add(advance),
            Dir::ToClient => self.server_seq = self.server_seq.wrapping_add(advance),
        }
    }

    fn handshake(&mut self) {
        self.segment(Dir::ToServer, 0x02, &[]);
        self.segment(Dir::ToClient, 0x12, &[]);
        self.segment(Dir::ToServer, 0x10, &[]);
    }

    /// Sends a message in segments of at most MSS octets and acknowledges it.
    fn message(&mut self, dir: Dir, data: &[u8]) {
        for chunk in data.chunks(MSS) {
            self.segment(dir, 0x18, chunk);
        }
        let other = match dir {
            Dir::ToServer => Dir::ToClient,
            Dir::ToClient => Dir::ToServer,
        };
        self.segment(other, 0x10, &[]);
    }

    /// Closes the connection from the client.
    fn finish(&mut self) {
        self.segment(Dir::ToServer, 0x11, &[]);
        self.segment(Dir::ToClient, 0x11, &[]);
        self.segment(Dir::ToServer, 0x10, &[]);
    }
}

/// Splits what one side wrote into the encoded messages with their ids.
fn split(wire: &[u8]) -> Vec<(i64, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < wire.len() {
        let (_, head, content) = ber::head(&wire[at..]).expect("a head");
        let end = at + head + content;
        let m = Message::decode(&wire[at..end]).expect("a message");
        out.push((m.id, wire[at..end].to_vec()));
        at = end;
    }
    out
}

/// The messages of one session in wire order: each request, then the messages
/// of the server that carry its id.
fn session_messages(c2s: &[u8], s2c: &[u8]) -> Vec<(Dir, Vec<u8>)> {
    let server = split(s2c);
    let mut out = Vec::new();
    for (id, request) in split(c2s) {
        out.push((Dir::ToServer, request));
        for (sid, m) in &server {
            if *sid == id {
                out.push((Dir::ToClient, m.clone()));
            }
        }
    }
    out
}

fn scratch(name: &str) -> PathBuf {
    // Cargo gives integration tests a scratch directory inside the target directory.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fps-module-ldap");
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir.join(name)
}

fn write_capture(name: &str, messages: &[(Dir, Vec<u8>)]) -> PathBuf {
    let mut pcap = Pcap::new();
    pcap.handshake();
    for (dir, m) in messages {
        pcap.message(*dir, m);
    }
    pcap.finish();
    let path = scratch(name);
    std::fs::write(&path, &pcap.bytes).expect("write the capture");
    path
}

fn tshark(capture: &Path, args: &[&str]) -> String {
    let out = Command::new("tshark")
        .arg("-r")
        .arg(capture)
        .args(["-o", "tcp.check_checksum:FALSE"])
        .args(args)
        .output()
        .expect("tshark starts");
    assert!(
        out.status.success(),
        "tshark failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Splits the output of `-T fields` into one vector of columns per line.
fn rows(output: &str) -> Vec<Vec<String>> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

/// The protocol operation numbers of the tshark field `ldap.protocolOp`.
fn op_number(op: &Op) -> &'static str {
    match op {
        Op::BindRequest { .. } => "0",
        Op::BindResponse(_) => "1",
        Op::UnbindRequest => "2",
        Op::SearchRequest(_) => "3",
        Op::SearchEntry(_) => "4",
        Op::SearchDone(_) => "5",
    }
}

fn check_session(name: &str, session_id: u64, messages: &[(Dir, Vec<u8>)]) {
    let capture = write_capture(&format!("{name}-{session_id}.pcap"), messages);

    // Every message is dissected as LDAP, once per message, in order.
    let out = tshark(
        &capture,
        &[
            "-Y",
            "ldap",
            "-T",
            "fields",
            "-e",
            "ldap.messageID",
            "-e",
            "ldap.protocolOp",
        ],
    );
    let got = rows(&out);
    assert_eq!(
        got.len(),
        messages.len(),
        "{name}: tshark decoded {} of {} messages:\n{out}",
        got.len(),
        messages.len()
    );
    for (row, (_, bytes)) in got.iter().zip(messages) {
        let m = Message::decode(bytes).expect("a message");
        assert_eq!(row[0], m.id.to_string(), "{name}: message id");
        assert_eq!(
            row[1],
            op_number(&m.op),
            "{name}: protocol operation of id {}",
            m.id
        );
    }

    // No malformed packet, and nothing at warning level or above in the expert
    // system of tshark.
    let bad = tshark(
        &capture,
        &[
            "-Y",
            "_ws.malformed || _ws.expert.severity >= 0x00600000",
            "-T",
            "fields",
            "-e",
            "frame.number",
            "-e",
            "_ws.expert.message",
        ],
    );
    assert!(
        bad.trim().is_empty(),
        "{name}: tshark reports problems:\n{bad}"
    );
}

fn check_fields(name: &str, session_id: u64, messages: &[(Dir, Vec<u8>)]) {
    let capture = write_capture(&format!("{name}-{session_id}.pcap"), messages);
    let value = |field: &str| -> Vec<String> {
        rows(&tshark(
            &capture,
            &[
                "-Y",
                "ldap",
                "-T",
                "fields",
                "-E",
                "aggregator=|",
                "-e",
                field,
            ],
        ))
        .into_iter()
        .flatten()
        .flat_map(|s| s.split('|').map(str::to_owned).collect::<Vec<_>>())
        .collect()
    };
    let mut bind_names = Vec::new();
    let mut bases = Vec::new();
    let mut codes = Vec::new();
    let mut entry_dns = Vec::new();
    for (_, bytes) in messages {
        match Message::decode(bytes).expect("a message").op {
            Op::BindRequest { name, .. } if !name.is_empty() => bind_names.push(name),
            Op::SearchRequest(s) => bases.push(s.base),
            Op::BindResponse(r) | Op::SearchDone(r) => codes.push(r.code.to_string()),
            Op::SearchEntry(e) => entry_dns.push(e.dn),
            _ => {}
        }
    }
    assert_eq!(value("ldap.name"), bind_names, "{name}: bind names");
    assert_eq!(
        value("ldap.baseObject")
            .into_iter()
            .filter(|b| !b.is_empty())
            .collect::<Vec<_>>(),
        bases
            .into_iter()
            .filter(|b| !b.is_empty())
            .collect::<Vec<_>>(),
        "{name}: search bases"
    );
    assert_eq!(value("ldap.resultCode"), codes, "{name}: result codes");
    assert_eq!(
        value("ldap.objectName")
            .into_iter()
            .filter(|b| !b.is_empty())
            .collect::<Vec<_>>(),
        entry_dns
            .into_iter()
            .filter(|b| !b.is_empty())
            .collect::<Vec<_>>(),
        "{name}: entry names"
    );
}

#[tokio::test(start_paused = true)]
async fn tshark_parses_every_message_of_every_scenario() {
    if skip_without_tshark() {
        return;
    }
    for (name, s) in scenarios() {
        for session_id in [1, 2, 3] {
            let r = run(&s, SEED, session_id).await;
            r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
            let conn = &r.conns_of("session")[0];
            let messages = session_messages(&conn.c2s(), &conn.s2c());
            assert!(messages.len() >= 5, "{name}: {} messages", messages.len());
            check_session(&name, session_id, &messages);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn tshark_reads_the_names_codes_and_entries() {
    if skip_without_tshark() {
        return;
    }
    for (name, s) in scenarios() {
        let r = run(&s, SEED, 1).await;
        r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
        let conn = &r.conns_of("session")[0];
        let messages = session_messages(&conn.c2s(), &conn.s2c());
        check_fields(&name, 1, &messages);
    }
}

#[tokio::test(start_paused = true)]
async fn tshark_parses_the_long_form_and_every_operation_with_odd_values() {
    if skip_without_tshark() {
        return;
    }
    use fps_flow::Value;
    // An entry above 127 octets uses the long length form of BER, and the
    // size limit and the errors exercise the other result codes.
    let long = "x".repeat(900);
    let ldif = format!(
        "dn: dc=lab,dc=test\nobjectClass: domain\ndc: lab\n\ndn: ou=People,dc=lab,dc=test\nobjectClass: organizationalUnit\nou: People\n\ndn: uid=solo,ou=People,dc=lab,dc=test\nobjectClass: inetOrgPerson\nuid: solo\ndescription: {long}\nuserPassword: Tst-solo-only-1\n"
    );
    let all = common::scenarios_with(&[
        ("directory", Value::Str(ldif)),
        ("search_base", Value::Str("dc=lab,dc=test".into())),
        ("user_attributes", Value::Str(String::new())),
    ]);
    let (name, s) = all.iter().find(|(n, _)| n == "bind_search").unwrap();
    let r = run(s, SEED, 1).await;
    r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
    let conn = &r.conns_of("session")[0];
    let messages = session_messages(&conn.c2s(), &conn.s2c());
    assert!(
        messages.iter().any(|(_, m)| m[1] == 0x82),
        "a long form message"
    );
    check_session("long", 1, &messages);
    // Failed bind, size limit, and no such object, as the server answers them.
    let failed = common::scenarios();
    let (_, s) = failed.iter().find(|(n, _)| n == "failed_bind").unwrap();
    let r = run(s, SEED, 1).await;
    let conn = &r.conns_of("session")[0];
    let messages = session_messages(&conn.c2s(), &conn.s2c());
    check_session("failed", 1, &messages);
}
