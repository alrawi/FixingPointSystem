// SPDX-License-Identifier: GPL-3.0-only
//! Helpers of the capture tests: a manual clock, a minimal pcapng block
//! reader, and a packet parser that checks every checksum.
//!
//! The reader and the parser share no code with the crate, so they check the
//! writer against the format and not against itself.

#![allow(dead_code)] // each test binary uses a different part

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use fps_capture::CaptureClock;

/// A clock that moves 10 microseconds on every read, so runs repeat exactly.
#[derive(Debug, Default)]
pub struct StepClock(AtomicU64);

impl CaptureClock for StepClock {
    fn now_nanos(&self) -> u64 {
        1_700_000_000_000_000_000 + self.0.fetch_add(10_000, Ordering::Relaxed)
    }
}

/// A directory under the target temp directory that starts empty.
pub fn fresh_dir(name: &str) -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The blocks of one pcapng file.
#[derive(Debug, Default)]
pub struct Pcapng {
    pub section_headers: usize,
    pub interfaces: Vec<(u16, u32, Option<u8>)>,
    pub packets: Vec<(u64, Vec<u8>)>,
    /// Decryption secrets blocks as (secrets type, data), in file order.
    pub secrets: Vec<(u32, Vec<u8>)>,
    /// Block types in file order.
    pub order: Vec<u32>,
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads every block of a little endian pcapng file and checks its framing.
pub fn read_pcapng(path: &Path) -> Pcapng {
    let raw = std::fs::read(path).unwrap();
    let mut out = Pcapng::default();
    let mut at = 0;
    while at < raw.len() {
        assert!(
            raw.len() - at >= 12,
            "{}: truncated block header at {at}",
            path.display()
        );
        let kind = u32le(&raw, at);
        let total = u32le(&raw, at + 4) as usize;
        assert_eq!(total % 4, 0, "block length is a multiple of 4");
        assert!(
            total >= 12 && at + total <= raw.len(),
            "{}: block at {at} overruns the file",
            path.display()
        );
        assert_eq!(
            u32le(&raw, at + total - 4) as usize,
            total,
            "trailing length matches"
        );
        let body = &raw[at + 8..at + total - 4];
        if at == 0 {
            assert_eq!(kind, 0x0A0D_0D0A, "the file starts with a section header");
        }
        match kind {
            0x0A0D_0D0A => {
                assert_eq!(
                    u32le(body, 0),
                    0x1A2B_3C4D,
                    "little endian byte order magic"
                );
                assert_eq!((u16le(body, 4), u16le(body, 6)), (1, 0), "version 1.0");
                out.section_headers += 1;
            }
            0x0000_0001 => {
                // linktype, reserved, snaplen, then options.
                let mut tsresol = None;
                let mut o = 8;
                while o + 4 <= body.len() {
                    let (code, len) = (u16le(body, o), usize::from(u16le(body, o + 2)));
                    if code == 0 {
                        break;
                    }
                    if code == 9 {
                        tsresol = Some(body[o + 4]);
                    }
                    o += 4 + len.div_ceil(4) * 4;
                }
                out.interfaces
                    .push((u16le(body, 0), u32le(body, 4), tsresol));
            }
            0x0000_0006 => {
                assert_eq!(u32le(body, 0), 0, "packets use interface 0");
                let ts = (u64::from(u32le(body, 4)) << 32) | u64::from(u32le(body, 8));
                let (cap, orig) = (u32le(body, 12) as usize, u32le(body, 16) as usize);
                assert_eq!(cap, orig, "no packet is cut short");
                out.packets.push((ts, body[20..20 + cap].to_vec()));
            }
            0x0000_000A => {
                let (kind, len) = (u32le(body, 0), u32le(body, 4) as usize);
                out.secrets.push((kind, body[8..8 + len].to_vec()));
            }
            other => panic!("unexpected block type {other:#x}"),
        }
        out.order.push(kind);
        at += total;
    }
    assert_eq!(out.section_headers, 1);
    out
}

/// One parsed Ethernet frame.
#[derive(Debug, Clone)]
pub struct Packet {
    pub ts: u64,
    pub src: IpAddr,
    pub dst: IpAddr,
    pub sport: u16,
    pub dport: u16,
    pub udp: bool,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub payload: Vec<u8>,
}

pub const SYN: u8 = 0x02;
pub const ACK: u8 = 0x10;
pub const FIN: u8 = 0x01;
pub const RST: u8 = 0x04;

fn sum16(parts: &[&[u8]]) -> u16 {
    let bytes: Vec<u8> = parts.concat();
    let mut sum: u32 = 0;
    for pair in bytes.chunks(2) {
        sum += u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Parses an Ethernet frame with IPv4 or IPv6 and TCP or UDP, and checks the
/// IPv4 header checksum, the transport checksum, and every length field.
pub fn parse_frame(ts: u64, f: &[u8]) -> Packet {
    let (src, dst, proto, l4, pseudo): (IpAddr, IpAddr, u8, &[u8], Vec<u8>) = match [f[12], f[13]] {
        [0x08, 0x00] => {
            assert_eq!(f[14], 0x45);
            assert_eq!(sum16(&[&f[14..34]]), 0, "IPv4 header checksum");
            let total = usize::from(u16::from_be_bytes([f[16], f[17]]));
            assert_eq!(14 + total, f.len(), "IPv4 total length");
            let s: [u8; 4] = f[26..30].try_into().unwrap();
            let d: [u8; 4] = f[30..34].try_into().unwrap();
            let (proto, l4) = (f[23], &f[34..]);
            let pseudo = [
                &s[..],
                &d[..],
                &[0, proto],
                &(l4.len() as u16).to_be_bytes(),
            ]
            .concat();
            (s.into(), d.into(), proto, l4, pseudo)
        }
        [0x86, 0xdd] => {
            assert_eq!(f[14] >> 4, 6);
            let plen = usize::from(u16::from_be_bytes([f[18], f[19]]));
            assert_eq!(54 + plen, f.len(), "IPv6 payload length");
            let s: [u8; 16] = f[22..38].try_into().unwrap();
            let d: [u8; 16] = f[38..54].try_into().unwrap();
            let (proto, l4) = (f[20], &f[54..]);
            let pseudo = [
                &s[..],
                &d[..],
                &(l4.len() as u32).to_be_bytes(),
                &[0, 0, 0, proto],
            ]
            .concat();
            (s.into(), d.into(), proto, l4, pseudo)
        }
        other => panic!("unexpected ethertype {other:?}"),
    };
    assert_eq!(sum16(&[&pseudo, l4]), 0, "transport checksum");
    let (sport, dport) = (
        u16::from_be_bytes([l4[0], l4[1]]),
        u16::from_be_bytes([l4[2], l4[3]]),
    );
    match proto {
        6 => {
            let off = usize::from(l4[12] >> 4) * 4;
            Packet {
                ts,
                src,
                dst,
                sport,
                dport,
                udp: false,
                seq: u32::from_be_bytes(l4[4..8].try_into().unwrap()),
                ack: u32::from_be_bytes(l4[8..12].try_into().unwrap()),
                flags: l4[13],
                payload: l4[off..].to_vec(),
            }
        }
        17 => {
            assert_eq!(
                usize::from(u16::from_be_bytes([l4[4], l4[5]])),
                l4.len(),
                "UDP length"
            );
            Packet {
                ts,
                src,
                dst,
                sport,
                dport,
                udp: true,
                seq: 0,
                ack: 0,
                flags: 0,
                payload: l4[8..].to_vec(),
            }
        }
        other => panic!("unexpected protocol {other}"),
    }
}

/// All packets of a file, parsed, with non decreasing timestamps checked.
pub fn packets(file: &Pcapng) -> Vec<Packet> {
    assert_eq!(
        file.interfaces,
        vec![(1, 262_144, Some(9))],
        "one Ethernet interface with nanosecond timestamps"
    );
    let parsed: Vec<Packet> = file
        .packets
        .iter()
        .map(|(ts, f)| parse_frame(*ts, f))
        .collect();
    assert!(
        parsed.windows(2).all(|w| w[0].ts <= w[1].ts),
        "timestamps do not go back"
    );
    parsed
}

/// One reassembled TCP connection.
#[derive(Debug, Default)]
pub struct Conn {
    pub to_server: Vec<u8>,
    pub to_client: Vec<u8>,
    pub saw_syn: bool,
    pub saw_syn_ack: bool,
    pub fins: usize,
    pub rst: bool,
    pub packets: usize,
}

/// Reassembles TCP connections by their client port, which the tests keep
/// unique per session. The reassembly fails when a segment leaves a gap or overlaps.
pub fn reassemble(packets: &[Packet], server_port: u16) -> BTreeMap<u16, Conn> {
    let mut conns: BTreeMap<u16, Conn> = BTreeMap::new();
    let mut next: BTreeMap<(u16, bool), u32> = BTreeMap::new();
    for p in packets.iter().filter(|p| !p.udp) {
        let from_client = p.dport == server_port;
        let port = if from_client { p.sport } else { p.dport };
        let c = conns.entry(port).or_default();
        c.packets += 1;
        let key = (port, from_client);
        if p.flags & SYN != 0 {
            if from_client {
                c.saw_syn = true;
            } else {
                c.saw_syn_ack = p.flags & ACK != 0;
            }
            next.insert(key, p.seq.wrapping_add(1));
            continue;
        }
        if p.flags & RST != 0 {
            c.rst = true;
            continue;
        }
        let expect = *next.get(&key).expect("data after the handshake");
        assert_eq!(p.seq, expect, "port {port}: segments are contiguous");
        let mut advance = p.payload.len() as u32;
        if p.flags & FIN != 0 {
            c.fins += 1;
            advance += 1;
        }
        next.insert(key, expect.wrapping_add(advance));
        if from_client {
            c.to_server.extend_from_slice(&p.payload);
        } else {
            c.to_client.extend_from_slice(&p.payload);
        }
    }
    conns
}

pub fn tool_available(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Returns true when the test must stop because its tool is missing, and
/// fails instead when `FPS_REQUIRE_CAPTURE_TOOLS` is `1`.
pub fn skip_without(tool: &str, available: bool) -> bool {
    if available {
        return false;
    }
    assert!(
        std::env::var("FPS_REQUIRE_CAPTURE_TOOLS").as_deref() != Ok("1"),
        "{tool} is required by FPS_REQUIRE_CAPTURE_TOOLS=1 but is not available"
    );
    eprintln!("skipping: {tool} is not available");
    true
}
