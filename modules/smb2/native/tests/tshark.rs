// SPDX-License-Identifier: GPL-3.0-only
//! The acceptance test of MOD-12: tshark parses every message that the
//! functions encode without an error.
//!
//! The test writes the messages of a whole session into a classic pcap file
//! as TCP segments between a client and a server on port 445, and runs tshark
//! over it. tshark must dissect every message as SMB2 without a malformed
//! packet and without an error or warning of its expert system, and the fields
//! that it decodes must equal the values that the test put in.
//!
//! The test skips itself when tshark is missing, unless the environment
//! variable `FPS_REQUIRE_CAPTURE_TOOLS` is `1`, which the CI job with tshark sets.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{session, Dir};

const MSS: usize = 1400;
const CLIENT: [u8; 4] = [10, 0, 0, 2];
const SERVER: [u8; 4] = [10, 0, 0, 1];
const CLIENT_PORT: u16 = 49_152;
const SERVER_PORT: u16 = 445;

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
}

fn scratch(name: &str) -> PathBuf {
    // Cargo gives integration tests a scratch directory inside the target directory.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fps-module-smb2");
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir.join(name)
}

fn write_capture(name: &str, messages: &[(Dir, Vec<u8>)]) -> PathBuf {
    let mut pcap = Pcap::new();
    pcap.handshake();
    for (dir, m) in messages {
        pcap.message(*dir, m);
    }
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

fn check_session(name: &str, dialects: &str, dialect_name: &str, read_size: usize) {
    let messages = session(dialects, dialect_name, read_size);
    let capture = write_capture(name, &messages);

    // Every message is dissected as SMB2, once per message.
    let smb = tshark(
        &capture,
        &[
            "-Y",
            "smb2",
            "-T",
            "fields",
            "-e",
            "smb2.cmd",
            "-e",
            "smb2.flags.response",
            "-e",
            "smb2.nt_status",
            "-e",
            "smb2.msg_id",
        ],
    );
    let rows = rows(&smb);
    assert_eq!(
        rows.len(),
        messages.len(),
        "tshark decoded {} of {} messages:\n{smb}",
        rows.len(),
        messages.len()
    );

    // No packet is malformed, and the expert system raises nothing above a note.
    let malformed = tshark(
        &capture,
        &["-Y", "_ws.malformed", "-T", "fields", "-e", "frame.number"],
    );
    assert_eq!(malformed.trim(), "", "malformed packets:\n{malformed}");
    let expert = tshark(
        &capture,
        &[
            "-T",
            "fields",
            "-e",
            "frame.number",
            "-e",
            "_ws.expert.severity",
            "-e",
            "_ws.expert.message",
            "-Y",
            "_ws.expert",
        ],
    );
    for row in self::rows(&expert) {
        // 0x00100000 comment, 0x00200000 chat, 0x00400000 note, 0x00600000 warning, 0x00800000 error.
        let severities: Vec<u32> = row[1].split(',').filter_map(|s| s.parse().ok()).collect();
        assert!(
            severities.iter().all(|s| *s <= 0x0040_0000),
            "expert warning or error in frame {}: {}",
            row[0],
            row.get(2).map_or("", String::as_str)
        );
    }

    // The commands, directions, and message ids that the test encoded.
    let want_cmds: Vec<&str> = [
        "0", "0", "1", "1", "1", "1", "3", "3", "5", "5", "8", "8", "5", "5", "9", "9", "6", "6",
        "5", "5", "14", "14", "14", "14", "5", "5", "5", "5", "6", "6", "4", "4", "2", "2",
    ]
    .to_vec();
    let got_cmds: Vec<&str> = rows.iter().map(|r| r[0].as_str()).collect();
    assert_eq!(got_cmds, want_cmds);
    let responses: Vec<&str> = rows.iter().map(|r| r[1].as_str()).collect();
    assert!(
        responses
            .iter()
            .enumerate()
            .all(|(i, r)| (*r == "True") == (i % 2 == 1)),
        "{responses:?}"
    );
    // Status of every response, in order: success except the staged ones.
    let statuses: Vec<&str> = rows.iter().map(|r| r[2].as_str()).collect();
    assert_eq!(statuses[3], "0xc0000016", "more processing required");
    assert_eq!(statuses[23], "0x80000006", "no more files");
    assert_eq!(statuses[25], "0xc0000034", "object name not found");
    assert_eq!(statuses[27], "0xc0000022", "access denied");
    // 17 responses, and four of them carry a status other than success.
    let successes = statuses
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|s| **s == "0x00000000")
        .count();
    assert_eq!(successes, 13, "{statuses:?}");

    // Field values that tshark decodes from the bytes.
    let fields = tshark(
        &capture,
        &[
            "-Y", "smb2.cmd == 3 && smb2.flags.response == 0 || smb2.cmd == 5 && smb2.flags.response == 0",
            "-T", "fields", "-e", "smb2.tree", "-e", "smb2.filename",
        ],
    );
    let r = self::rows(&fields);
    assert_eq!(r[0][0], "\\\\fpsserver\\share");
    let names: Vec<&str> = r
        .iter()
        .skip(1)
        .map(|c| c.get(1).map_or("", String::as_str))
        .collect();
    assert_eq!(
        names,
        [
            "docs\\report.bin",
            "upload\\new.txt",
            "",
            "docs\\missing.txt",
            "secret.txt"
        ]
    );

    // The NTLM exchange: tshark reads the three message types and the user.
    let ntlm = tshark(
        &capture,
        &[
            "-Y",
            "ntlmssp",
            "-T",
            "fields",
            "-e",
            "ntlmssp.messagetype",
            "-e",
            "ntlmssp.auth.username",
            "-e",
            "ntlmssp.auth.domain",
            "-e",
            "ntlmssp.auth.hostname",
            "-e",
            "ntlmssp.challenge.target_name",
        ],
    );
    let n = self::rows(&ntlm);
    assert_eq!(n.len(), 3, "{ntlm}");
    assert_eq!(n[0][0], "0x00000001");
    assert_eq!(n[1][0], "0x00000002");
    assert_eq!(n[1].get(4).map(String::as_str), Some("FPS"));
    assert_eq!(n[2][0], "0x00000003");
    assert_eq!(&n[2][1..4], ["alice", "FPS", "WS01"]);

    // The negotiated dialect, and the sizes of the data that moved.
    let selected = tshark(
        &capture,
        &[
            "-Y",
            "smb2.cmd == 0 && smb2.flags.response == 1",
            "-T",
            "fields",
            "-e",
            "smb2.dialect",
        ],
    );
    let want = match dialect_name {
        "2.0.2" => "0x0202",
        "2.1" => "0x0210",
        "3.0.2" => "0x0302",
        _ => "0x0311",
    };
    assert_eq!(selected.trim(), want);
    let sizes = tshark(
        &capture,
        &[
            "-Y",
            "smb2.cmd == 8 || smb2.cmd == 9",
            "-T",
            "fields",
            "-e",
            "smb2.read_length",
            "-e",
            "smb2.write_length",
        ],
    );
    let s = self::rows(&sizes);
    assert_eq!(s.len(), 4);
    assert_eq!(s[0][0], read_size.to_string());
    assert_eq!(s[2][1], "3000");
}

#[test]
fn tshark_parses_an_smb_3_1_1_session() {
    if skip_without_tshark() {
        return;
    }
    check_session("smb311.pcap", "2.0.2,2.1,3.0,3.0.2,3.1.1", "3.1.1", 200_000);
}

#[test]
fn tshark_parses_an_smb_2_1_session() {
    if skip_without_tshark() {
        return;
    }
    check_session("smb21.pcap", "2.0.2,2.1", "2.1", 8192);
}

#[test]
fn tshark_parses_an_smb_2_0_2_session() {
    if skip_without_tshark() {
        return;
    }
    check_session("smb202.pcap", "2.0.2", "2.0.2", 100);
}

#[test]
fn tshark_parses_an_smb_3_0_2_session() {
    if skip_without_tshark() {
        return;
    }
    check_session("smb302.pcap", "3.0,3.0.2", "3.0.2", 65_536);
}
