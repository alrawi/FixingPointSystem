// SPDX-License-Identifier: GPL-3.0-only
//! Helpers of the TLS component tests: a recording session, a ClientHello
//! parser, and a classic pcap writer.

#![allow(dead_code)] // each test binary uses a different part

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_io::SessionIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Direction of recorded bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    ClientToServer,
    ServerToClient,
}

/// Bytes in the order in which the two sides wrote them.
pub type Log = Arc<Mutex<Vec<(Dir, Vec<u8>)>>>;

/// Wraps a session and records every byte that this end writes.
pub struct Tap<IO> {
    io: IO,
    dir: Dir,
    log: Log,
}

impl<IO> Tap<IO> {
    pub fn new(io: IO, dir: Dir, log: &Log) -> Self {
        Self {
            io,
            dir,
            log: Arc::clone(log),
        }
    }
}

impl<IO: AsyncRead + Unpin> AsyncRead for Tap<IO> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl<IO: AsyncWrite + Unpin> AsyncWrite for Tap<IO> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let r = Pin::new(&mut self.io).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &r {
            let (dir, data) = (self.dir, buf[..*n].to_vec());
            self.log.lock().unwrap().push((dir, data));
        }
        r
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

impl<IO: SessionIo> SessionIo for Tap<IO> {
    fn abort(&mut self) {
        self.io.abort();
    }
}

/// Returns the concatenated bytes of one direction.
pub fn stream(log: &Log, dir: Dir) -> Vec<u8> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|(d, _)| *d == dir)
        .flat_map(|(_, b)| b.clone())
        .collect()
}

/// The fields of a ClientHello that the profile tests check.
#[derive(Debug, Default)]
pub struct Hello {
    pub random: Vec<u8>,
    pub ciphers: Vec<u16>,
    pub sni: Option<String>,
    pub groups: Vec<u16>,
    pub key_share_groups: Vec<u16>,
    pub alpn: Vec<String>,
    pub versions: Vec<u16>,
}

struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        head
    }
    fn u8(&mut self) -> usize {
        usize::from(self.take(1)[0])
    }
    fn u16(&mut self) -> usize {
        let b = self.take(2);
        usize::from(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u24(&mut self) -> usize {
        let b = self.take(3);
        (usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2])
    }
}

fn u16s(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

/// Parses the first record of a client stream as a ClientHello.
pub fn parse_client_hello(stream: &[u8]) -> Hello {
    let mut rec = Cursor(stream);
    assert_eq!(rec.u8(), 22, "first record is a handshake record");
    rec.take(2);
    let len = rec.u16();
    let mut hs = Cursor(rec.take(len));
    assert_eq!(hs.u8(), 1, "handshake message is a ClientHello");
    let len = hs.u24();
    let mut ch = Cursor(hs.take(len));
    ch.take(2);
    let mut hello = Hello {
        random: ch.take(32).to_vec(),
        ..Hello::default()
    };
    let sid = ch.u8();
    ch.take(sid);
    let n = ch.u16();
    hello.ciphers = u16s(ch.take(n));
    let n = ch.u8();
    ch.take(n);
    let n = ch.u16();
    let mut ext = Cursor(ch.take(n));
    while !ext.0.is_empty() {
        let kind = ext.u16();
        let n = ext.u16();
        let mut body = Cursor(ext.take(n));
        match kind {
            0 => {
                body.u16();
                assert_eq!(body.u8(), 0, "host_name type");
                let n = body.u16();
                hello.sni = Some(String::from_utf8(body.take(n).to_vec()).unwrap());
            }
            10 => {
                let n = body.u16();
                hello.groups = u16s(body.take(n));
            }
            16 => {
                body.u16();
                while !body.0.is_empty() {
                    let n = body.u8();
                    hello
                        .alpn
                        .push(String::from_utf8(body.take(n).to_vec()).unwrap());
                }
            }
            43 => {
                let n = body.u8();
                hello.versions = u16s(body.take(n));
            }
            51 => {
                body.u16();
                while !body.0.is_empty() {
                    hello
                        .key_share_groups
                        .push(u16::try_from(body.u16()).unwrap());
                    let n = body.u16();
                    body.take(n);
                }
            }
            _ => {}
        }
    }
    hello
}

fn checksum(parts: &[&[u8]]) -> u16 {
    let mut sum: u32 = 0;
    let mut odd: Option<u8> = None;
    for byte in parts.iter().flat_map(|p| p.iter()) {
        match odd.take() {
            Some(high) => sum += u32::from(u16::from_be_bytes([high, *byte])),
            None => odd = Some(*byte),
        }
    }
    if let Some(high) = odd {
        sum += u32::from(u16::from_be_bytes([high, 0]));
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

const CLIENT_IP: [u8; 4] = [10, 0, 0, 1];
const SERVER_IP: [u8; 4] = [10, 0, 0, 2];
const CLIENT_PORT: u16 = 50_000;
const SERVER_PORT: u16 = 443;
const MSS: usize = 1400;

struct Pcap {
    out: Vec<u8>,
    micros: u64,
}

impl Pcap {
    fn frame(&mut self, from_client: bool, seq: u32, ack: u32, flags: u8, payload: &[u8]) {
        let (src, dst, sport, dport) = if from_client {
            (CLIENT_IP, SERVER_IP, CLIENT_PORT, SERVER_PORT)
        } else {
            (SERVER_IP, CLIENT_IP, SERVER_PORT, CLIENT_PORT)
        };
        let mut tcp = Vec::with_capacity(20 + payload.len());
        tcp.extend_from_slice(&sport.to_be_bytes());
        tcp.extend_from_slice(&dport.to_be_bytes());
        tcp.extend_from_slice(&seq.to_be_bytes());
        tcp.extend_from_slice(&ack.to_be_bytes());
        tcp.extend_from_slice(&[0x50, flags]);
        tcp.extend_from_slice(&65_535u16.to_be_bytes());
        tcp.extend_from_slice(&[0, 0, 0, 0]);
        tcp.extend_from_slice(payload);
        let pseudo = [
            &src[..],
            &dst[..],
            &[0, 6],
            &u16::try_from(tcp.len()).unwrap().to_be_bytes()[..],
        ]
        .concat();
        let sum = checksum(&[&pseudo, &tcp]);
        tcp[16..18].copy_from_slice(&sum.to_be_bytes());

        let mut ip = vec![0x45, 0];
        ip.extend_from_slice(&u16::try_from(20 + tcp.len()).unwrap().to_be_bytes());
        ip.extend_from_slice(&[0, 0, 0x40, 0, 64, 6, 0, 0]);
        ip.extend_from_slice(&src);
        ip.extend_from_slice(&dst);
        let sum = checksum(&[&ip]);
        ip[10..12].copy_from_slice(&sum.to_be_bytes());

        let mut eth = Vec::with_capacity(14 + ip.len() + tcp.len());
        let (dmac, smac) = if from_client { (2u8, 1u8) } else { (1u8, 2u8) };
        eth.extend_from_slice(&[0x02, 0, 0, 0, 0, dmac, 0x02, 0, 0, 0, 0, smac, 0x08, 0x00]);
        eth.extend_from_slice(&ip);
        eth.extend_from_slice(&tcp);

        self.micros += 100;
        let len = u32::try_from(eth.len()).unwrap();
        self.out.extend_from_slice(
            &u32::try_from(1_700_000_000 + self.micros / 1_000_000)
                .unwrap()
                .to_le_bytes(),
        );
        self.out.extend_from_slice(
            &u32::try_from(self.micros % 1_000_000)
                .unwrap()
                .to_le_bytes(),
        );
        self.out.extend_from_slice(&len.to_le_bytes());
        self.out.extend_from_slice(&len.to_le_bytes());
        self.out.extend_from_slice(&eth);
    }
}

/// Renders the recorded bytes as a classic pcap file of one TCP connection
/// from 10.0.0.1:50000 to 10.0.0.2:443, with a three way handshake and one
/// segment of at most 1400 bytes per chunk, in the order of the log.
pub fn pcap(log: &Log) -> Vec<u8> {
    let mut p = Pcap {
        out: Vec::new(),
        micros: 0,
    };
    p.out
        .extend_from_slice(&[0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0]);
    p.out.extend_from_slice(&[0; 8]);
    p.out.extend_from_slice(&65_535u32.to_le_bytes());
    p.out.extend_from_slice(&1u32.to_le_bytes());
    let (mut c_seq, mut s_seq) = (1_000u32, 5_000u32);
    p.frame(true, c_seq, 0, 0x02, &[]);
    c_seq += 1;
    p.frame(false, s_seq, c_seq, 0x12, &[]);
    s_seq += 1;
    p.frame(true, c_seq, s_seq, 0x10, &[]);
    for (dir, data) in log.lock().unwrap().iter() {
        for chunk in data.chunks(MSS) {
            let len = u32::try_from(chunk.len()).unwrap();
            match dir {
                Dir::ClientToServer => {
                    p.frame(true, c_seq, s_seq, 0x18, chunk);
                    c_seq += len;
                }
                Dir::ServerToClient => {
                    p.frame(false, s_seq, c_seq, 0x18, chunk);
                    s_seq += len;
                }
            }
        }
    }
    p.out
}
