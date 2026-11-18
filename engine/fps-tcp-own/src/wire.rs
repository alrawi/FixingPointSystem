// SPDX-License-Identifier: GPL-3.0-only
//! The packet format of the own stack: an IPv4 header without options or an
//! IPv6 header without extension headers (TRN-30), a TCP header with the
//! options of [`Options`] (MSS, window scale, selective acknowledgment,
//! timestamps), and the payload. The Ethernet header belongs to
//! [`crate::net`].
//!
//! Both checksums of IPv4 and the TCP checksum of either family are computed
//! on send and verified on receive, so the measurements include their cost.
//! IPv6 has no header checksum, and its TCP checksum covers the pseudo header
//! of RFC 8200 section 8.1.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The length of the IPv4 header that the stack sends and accepts.
pub const IP_LEN: usize = 20;
/// The length of the IPv6 header that the stack sends and accepts. The stack
/// sends no extension header and drops a packet that has one.
pub const IP6_LEN: usize = 40;
/// The hop limit of the packets that the stack sends (the TTL of IPv4).
const HOP_LIMIT: u8 = 64;
/// The length of the TCP header without options.
pub const TCP_LEN: usize = 20;
/// The length of the MSS option.
pub const MSS_OPTION_LEN: usize = 4;
/// The most bytes that TCP options may take (a data offset of 15).
pub const OPTIONS_MAX: usize = 40;
/// The most SACK blocks that an option can carry (3 beside a timestamp).
pub const MAX_SACK_BLOCKS: usize = 4;
/// The largest window scale shift that RFC 7323 allows.
pub const MAX_WSCALE: u8 = 14;
/// The length of the timestamp option with its two padding bytes.
pub const TIMESTAMP_OPTION_LEN: usize = 12;

/// The options of a segment besides the MSS (RFC 7323, RFC 2018).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The window scale shift, on SYN segments only.
    pub wscale: Option<u8>,
    /// SACK permitted, on SYN segments only.
    pub sack_permitted: bool,
    /// The timestamp option: the value of the sender and the echo.
    pub timestamp: Option<(u32, u32)>,
    /// How many of `sack` are in use.
    pub sack_len: u8,
    /// The SACK blocks as (left edge, right edge), the right edge being the
    /// sequence number after the last byte.
    pub sack: [(u32, u32); MAX_SACK_BLOCKS],
}

impl Options {
    /// The SACK blocks in use.
    pub fn blocks(&self) -> &[(u32, u32)] {
        &self.sack[..usize::from(self.sack_len)]
    }

    /// Adds a SACK block, and returns false when the option is full.
    pub fn push_block(&mut self, left: u32, right: u32) -> bool {
        let max = if self.timestamp.is_some() {
            MAX_SACK_BLOCKS - 1
        } else {
            MAX_SACK_BLOCKS
        };
        if usize::from(self.sack_len) >= max {
            return false;
        }
        self.sack[usize::from(self.sack_len)] = (left, right);
        self.sack_len += 1;
        true
    }
}

/// Writes the options of `h` after the MSS option, in the order of the Linux
/// kernel: MSS, SACK permitted with the timestamp (or padding), window scale,
/// then SACK blocks. Returns the length, which is a multiple of 4.
fn write_options(h: &Header, buf: &mut [u8; OPTIONS_MAX]) -> usize {
    let syn = h.flags & flags::SYN != 0;
    let mut n = 0;
    if let Some(mss) = h.mss {
        buf[n..n + 2].copy_from_slice(&[2, 4]);
        buf[n + 2..n + 4].copy_from_slice(&mss.to_be_bytes());
        n += 4;
    }
    let sack_ok = syn && h.opts.sack_permitted;
    if let Some((value, echo)) = h.opts.timestamp {
        buf[n..n + 2].copy_from_slice(if sack_ok { &[4, 2] } else { &[1, 1] });
        buf[n + 2..n + 4].copy_from_slice(&[8, 10]);
        buf[n + 4..n + 8].copy_from_slice(&value.to_be_bytes());
        buf[n + 8..n + 12].copy_from_slice(&echo.to_be_bytes());
        n += TIMESTAMP_OPTION_LEN;
    } else if sack_ok {
        buf[n..n + 4].copy_from_slice(&[1, 1, 4, 2]);
        n += 4;
    }
    if let (true, Some(shift)) = (syn, h.opts.wscale) {
        buf[n..n + 4].copy_from_slice(&[1, 3, 3, shift.min(MAX_WSCALE)]);
        n += 4;
    }
    let max = if h.opts.timestamp.is_some() {
        MAX_SACK_BLOCKS - 1
    } else {
        MAX_SACK_BLOCKS
    };
    let blocks = &h.opts.blocks()[..h.opts.blocks().len().min(max)];
    if !blocks.is_empty() {
        buf[n..n + 4].copy_from_slice(&[1, 1, 5, (2 + 8 * blocks.len()) as u8]);
        n += 4;
        for (left, right) in blocks {
            buf[n..n + 4].copy_from_slice(&left.to_be_bytes());
            buf[n + 4..n + 8].copy_from_slice(&right.to_be_bytes());
            n += 8;
        }
    }
    n
}

/// The length of the TCP options that [`encode`] writes for `h`.
pub fn options_len(h: &Header) -> usize {
    let mut buf = [0u8; OPTIONS_MAX];
    write_options(h, &mut buf)
}

/// The TCP flag bits.
pub mod flags {
    /// No more data from the sender.
    pub const FIN: u8 = 0x01;
    /// Synchronize sequence numbers.
    pub const SYN: u8 = 0x02;
    /// Reset the connection.
    pub const RST: u8 = 0x04;
    /// Push, set on the last segment of a write.
    pub const PSH: u8 = 0x08;
    /// The acknowledgment number is valid.
    pub const ACK: u8 = 0x10;
}

/// The fields of one segment that the stack uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The source address. Both addresses are of one family.
    pub src_ip: IpAddr,
    /// The destination address.
    pub dst_ip: IpAddr,
    /// The TCP source port.
    pub src_port: u16,
    /// The TCP destination port.
    pub dst_port: u16,
    /// The sequence number of the first byte, or of the SYN.
    pub seq: u32,
    /// The next sequence number that the sender expects.
    pub ack: u32,
    /// The flag bits.
    pub flags: u8,
    /// The receive window, without scaling.
    pub window: u16,
    /// The MSS option, present on SYN segments.
    pub mss: Option<u16>,
    /// The other options.
    pub opts: Options,
}

/// A parsed frame, which borrows its payload from the frame.
#[derive(Clone, Copy, Debug)]
pub struct Parsed<'a> {
    /// The header fields.
    pub header: Header,
    /// The TCP payload.
    pub payload: &'a [u8],
}

/// Why a frame was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    /// The frame is shorter than its headers claim.
    #[error("the frame is too short")]
    Short,
    /// The IP version is neither 4 nor 6, or an IPv4 header has options.
    #[error("not an IPv4 header without options or an IPv6 header")]
    NotIp,
    /// The protocol is not TCP.
    #[error("the protocol is not TCP")]
    NotTcp,
    /// The total length field disagrees with the frame.
    #[error("the length field is wrong")]
    BadLength,
    /// The IPv4 header checksum failed.
    #[error("the IP header checksum failed")]
    BadIpChecksum,
    /// The TCP checksum failed.
    #[error("the TCP checksum failed")]
    BadTcpChecksum,
    /// The TCP data offset is below 5 or beyond the frame.
    #[error("the TCP data offset is wrong")]
    BadOffset,
}

/// Adds `data` to a one's complement sum, as 32 bit big endian words.
fn sum_words(mut sum: u64, data: &[u8]) -> u64 {
    let (words, tail) = data.as_chunks::<4>();
    for word in words {
        sum += u64::from(u32::from_be_bytes(*word));
    }
    if !tail.is_empty() {
        let mut word = [0u8; 4];
        word[..tail.len()].copy_from_slice(tail);
        sum += u64::from(u32::from_be_bytes(word));
    }
    sum
}

/// Folds a sum of 32 bit words to the 16 bit one's complement value.
fn fold(mut sum: u64) -> u16 {
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    sum as u16
}

/// The length of the IP header that the stack writes for packets from `ip`.
#[must_use]
pub fn ip_header_len(ip: IpAddr) -> usize {
    match ip {
        IpAddr::V4(_) => IP_LEN,
        IpAddr::V6(_) => IP6_LEN,
    }
}

/// An IPv4 address given as the number that `Ipv4Addr::from(u32)` reads.
#[must_use]
pub fn v4(ip: u32) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(ip))
}

fn v4_of(ip: IpAddr) -> u32 {
    match ip {
        IpAddr::V4(a) => u32::from(a),
        // A header of mixed families is a bug of the caller; it goes out as a
        // header of the family of its source.
        IpAddr::V6(_) => 0,
    }
}

fn v6_of(ip: IpAddr) -> Ipv6Addr {
    match ip {
        IpAddr::V6(a) => a,
        IpAddr::V4(_) => Ipv6Addr::UNSPECIFIED,
    }
}

fn pseudo_sum4(src_ip: u32, dst_ip: u32, tcp_len: usize) -> u64 {
    u64::from(src_ip >> 16)
        + u64::from(src_ip & 0xFFFF)
        + u64::from(dst_ip >> 16)
        + u64::from(dst_ip & 0xFFFF)
        + 6
        + tcp_len as u64
}

/// The pseudo header sum of RFC 8200 section 8.1: both addresses, the length
/// of the segment as 32 bits, and the next header 6.
fn pseudo_sum6(src: Ipv6Addr, dst: Ipv6Addr, tcp_len: usize) -> u64 {
    sum_words(0, &src.octets()) + sum_words(0, &dst.octets()) + 6 + tcp_len as u64
}

fn pseudo_sum(src: IpAddr, dst: IpAddr, tcp_len: usize) -> u64 {
    match src {
        IpAddr::V4(_) => pseudo_sum4(v4_of(src), v4_of(dst), tcp_len),
        IpAddr::V6(a) => pseudo_sum6(a, v6_of(dst), tcp_len),
    }
}

/// Appends the packet for `header` and the payload `(first, second)` to
/// `out`, as IPv4 when the source is an IPv4 address and as IPv6 when it is
/// an IPv6 address. The payload comes in two slices, because a ring buffer
/// hands out two.
pub fn encode(out: &mut Vec<u8>, header: &Header, payload: (&[u8], &[u8])) {
    let mut option_bytes = [0u8; OPTIONS_MAX];
    let options = write_options(header, &mut option_bytes);
    let tcp_len = TCP_LEN + options + payload.0.len() + payload.1.len();
    let ip_len = ip_header_len(header.src_ip);
    let total = ip_len + tcp_len;
    let base = out.len();
    out.reserve(total);
    match header.src_ip {
        IpAddr::V4(src) => {
            out.extend_from_slice(&[0x45, 0]);
            out.extend_from_slice(&(total as u16).to_be_bytes());
            out.extend_from_slice(&[0, 0, 0x40, 0, HOP_LIMIT, 6, 0, 0]);
            out.extend_from_slice(&src.octets());
            out.extend_from_slice(&v4_of(header.dst_ip).to_be_bytes());
            let ip_sum = !fold(sum_words(0, &out[base..base + IP_LEN]));
            out[base + 10..base + 12].copy_from_slice(&ip_sum.to_be_bytes());
        }
        IpAddr::V6(src) => {
            out.extend_from_slice(&[0x60, 0, 0, 0]);
            out.extend_from_slice(&(tcp_len as u16).to_be_bytes());
            out.extend_from_slice(&[6, HOP_LIMIT]);
            out.extend_from_slice(&src.octets());
            out.extend_from_slice(&v6_of(header.dst_ip).octets());
        }
    }
    out.extend_from_slice(&header.src_port.to_be_bytes());
    out.extend_from_slice(&header.dst_port.to_be_bytes());
    out.extend_from_slice(&header.seq.to_be_bytes());
    out.extend_from_slice(&header.ack.to_be_bytes());
    out.push((((TCP_LEN + options) / 4) as u8) << 4);
    out.push(header.flags);
    out.extend_from_slice(&header.window.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&option_bytes[..options]);
    out.extend_from_slice(payload.0);
    out.extend_from_slice(payload.1);
    let tcp_start = base + ip_len;
    let sum = pseudo_sum(header.src_ip, header.dst_ip, tcp_len) + sum_words(0, &out[tcp_start..]);
    let tcp_sum = !fold(sum);
    out[tcp_start + 16..tcp_start + 18].copy_from_slice(&tcp_sum.to_be_bytes());
}

fn be16(data: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([data[at], data[at + 1]])
}

fn be32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// Parses and verifies one Ethernet frame that carries IPv4 or IPv6 and TCP,
/// with or without an 802.1Q tag. The checks of [`parse`] apply to the packet.
pub fn parse_frame(frame: &[u8]) -> Result<Parsed<'_>, WireError> {
    let head = crate::net::decode_header(frame).map_err(|_| WireError::Short)?;
    if head.ethertype != crate::net::ETHERTYPE_IPV4 && head.ethertype != crate::net::ETHERTYPE_IPV6
    {
        return Err(WireError::NotIp);
    }
    parse(&frame[head.payload_at..])
}

/// Parses and verifies one IPv4 or IPv6 packet with a TCP segment.
pub fn parse(frame: &[u8]) -> Result<Parsed<'_>, WireError> {
    match frame.first().map(|b| b >> 4) {
        Some(4) => parse4(frame),
        Some(6) => parse6(frame),
        Some(_) => Err(WireError::NotIp),
        None => Err(WireError::Short),
    }
}

fn parse4(frame: &[u8]) -> Result<Parsed<'_>, WireError> {
    if frame.len() < IP_LEN + TCP_LEN {
        return Err(WireError::Short);
    }
    if frame[0] != 0x45 {
        return Err(WireError::NotIp);
    }
    if frame[9] != 6 {
        return Err(WireError::NotTcp);
    }
    let total = usize::from(be16(frame, 2));
    if total < IP_LEN + TCP_LEN || total > frame.len() {
        return Err(WireError::BadLength);
    }
    if fold(sum_words(0, &frame[..IP_LEN])) != 0xFFFF {
        return Err(WireError::BadIpChecksum);
    }
    let src = be32(frame, 12);
    let dst = be32(frame, 16);
    let tcp = &frame[IP_LEN..total];
    let sum = pseudo_sum4(src, dst, tcp.len()) + sum_words(0, tcp);
    if fold(sum) != 0xFFFF {
        return Err(WireError::BadTcpChecksum);
    }
    parse_tcp(v4(src), v4(dst), tcp)
}

fn parse6(frame: &[u8]) -> Result<Parsed<'_>, WireError> {
    if frame.len() < IP6_LEN + TCP_LEN {
        return Err(WireError::Short);
    }
    // An extension header in front of TCP is not read: the packet is not TCP
    // as far as the stack can tell.
    if frame[6] != 6 {
        return Err(WireError::NotTcp);
    }
    let payload = usize::from(be16(frame, 4));
    if payload < TCP_LEN || IP6_LEN + payload > frame.len() {
        return Err(WireError::BadLength);
    }
    let addr = |at: usize| {
        let mut octets = [0u8; 16];
        octets.copy_from_slice(&frame[at..at + 16]);
        Ipv6Addr::from(octets)
    };
    let (src, dst) = (addr(8), addr(24));
    let tcp = &frame[IP6_LEN..IP6_LEN + payload];
    let sum = pseudo_sum6(src, dst, tcp.len()) + sum_words(0, tcp);
    if fold(sum) != 0xFFFF {
        return Err(WireError::BadTcpChecksum);
    }
    parse_tcp(IpAddr::V6(src), IpAddr::V6(dst), tcp)
}

/// Reads the TCP header and options of a segment whose checksum passed.
fn parse_tcp(src_ip: IpAddr, dst_ip: IpAddr, tcp: &[u8]) -> Result<Parsed<'_>, WireError> {
    let offset = usize::from(tcp[12] >> 4) * 4;
    if offset < TCP_LEN || offset > tcp.len() {
        return Err(WireError::BadOffset);
    }
    let mut mss = None;
    let mut opts = Options::default();
    let mut options = &tcp[TCP_LEN..offset];
    while let Some(&kind) = options.first() {
        match kind {
            0 => break,
            1 => options = &options[1..],
            _ => {
                let len = usize::from(options.get(1).copied().unwrap_or(0));
                if len < 2 || len > options.len() {
                    return Err(WireError::BadOffset);
                }
                match (kind, len) {
                    (2, 4) => mss = Some(be16(options, 2)),
                    (3, 3) => opts.wscale = Some(options[2].min(MAX_WSCALE)),
                    (4, 2) => opts.sack_permitted = true,
                    (5, 10 | 18 | 26 | 34) => {
                        for block in options[2..len].as_chunks::<8>().0 {
                            opts.push_block(be32(block, 0), be32(block, 4));
                        }
                    }
                    (8, 10) => opts.timestamp = Some((be32(options, 2), be32(options, 6))),
                    _ => {}
                }
                options = &options[len..];
            }
        }
    }
    Ok(Parsed {
        header: Header {
            src_ip,
            dst_ip,
            src_port: be16(tcp, 0),
            dst_port: be16(tcp, 2),
            seq: be32(tcp, 4),
            ack: be32(tcp, 8),
            flags: tcp[13],
            window: be16(tcp, 14),
            mss,
            opts,
        },
        payload: &tcp[offset..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            src_ip: v4(0x0A00_0001),
            dst_ip: v4(0x0A00_0002),
            src_port: 40000,
            dst_port: 80,
            seq: 0xFFFF_FFF0,
            ack: 77,
            flags: flags::SYN | flags::ACK,
            window: 65535,
            mss: Some(1460),
            opts: Options::default(),
        }
    }

    #[test]
    fn a_frame_round_trips_with_a_split_odd_payload() {
        let mut out = Vec::new();
        encode(&mut out, &header(), (b"abc", b"defgh"));
        let parsed = parse(&out).expect("parse");
        assert_eq!(parsed.header, header());
        assert_eq!(parsed.payload, b"abcdefgh");
        assert_eq!(out.len(), IP_LEN + TCP_LEN + MSS_OPTION_LEN + 8);
    }

    #[test]
    fn a_frame_without_options_round_trips() {
        let mut h = header();
        h.mss = None;
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        assert_eq!(parse(&out).expect("parse").header, h);
        assert_eq!(out.len(), IP_LEN + TCP_LEN);
    }

    #[test]
    fn any_flipped_bit_is_caught_by_a_checksum() {
        let mut out = Vec::new();
        encode(&mut out, &header(), (b"payload", &[]));
        for byte in 0..out.len() {
            let mut bad = out.clone();
            bad[byte] ^= 0x10;
            assert!(parse(&bad).is_err(), "byte {byte} went unnoticed");
        }
    }

    #[test]
    fn short_and_foreign_frames_are_rejected() {
        let mut out = Vec::new();
        encode(&mut out, &header(), (&[], &[]));
        assert_eq!(parse(&out[..10]).unwrap_err(), WireError::Short);
        let mut udp = out.clone();
        udp[9] = 17;
        assert_eq!(parse(&udp).unwrap_err(), WireError::NotTcp);
        let mut v7 = out.clone();
        v7[0] = 0x70;
        assert_eq!(parse(&v7).unwrap_err(), WireError::NotIp);
        let mut options = out.clone();
        options[0] = 0x46;
        assert_eq!(parse(&options).unwrap_err(), WireError::NotIp);
        assert_eq!(
            parse(&out[..out.len() - 1]).unwrap_err(),
            WireError::BadLength
        );
    }

    fn all_options() -> Options {
        Options {
            wscale: Some(7),
            sack_permitted: true,
            timestamp: Some((0xAABB_CCDD, 0x1122_3344)),
            ..Options::default()
        }
    }

    /// Recomputes both checksums of a frame that a test edited.
    fn fix_checksums(frame: &mut [u8]) {
        frame[10..12].copy_from_slice(&[0, 0]);
        let ip = !fold(sum_words(0, &frame[..IP_LEN]));
        frame[10..12].copy_from_slice(&ip.to_be_bytes());
        frame[IP_LEN + 16..IP_LEN + 18].copy_from_slice(&[0, 0]);
        let (src, dst) = (be32(frame, 12), be32(frame, 16));
        let tcp_len = frame.len() - IP_LEN;
        let sum = pseudo_sum4(src, dst, tcp_len) + sum_words(0, &frame[IP_LEN..]);
        frame[IP_LEN + 16..IP_LEN + 18].copy_from_slice(&(!fold(sum)).to_be_bytes());
    }

    fn header6() -> Header {
        Header {
            src_ip: "fd00:1::1".parse().expect("address"),
            dst_ip: "fd00:2::2".parse().expect("address"),
            ..header()
        }
    }

    #[test]
    fn an_ipv6_frame_round_trips_with_options_and_a_split_payload() {
        let mut h = header6();
        h.flags = flags::SYN;
        h.opts = all_options();
        let mut out = Vec::new();
        encode(&mut out, &h, (b"abc", b"defgh"));
        assert_eq!(out[0] >> 4, 6);
        assert_eq!(out[6], 6, "next header is TCP");
        let tcp_len = out.len() - IP6_LEN;
        assert_eq!(usize::from(be16(&out, 4)), tcp_len);
        let parsed = parse(&out).expect("parse");
        assert_eq!(parsed.header, h);
        assert_eq!(parsed.payload, b"abcdefgh");
        assert_eq!(out.len(), IP6_LEN + TCP_LEN + options_len(&h) + 8);
    }

    #[test]
    fn an_ipv6_checksum_matches_a_hand_computed_value() {
        // A segment of 20 bytes without payload and options. The checksum is
        // the complement of the sum of the pseudo header and the segment.
        let mut h = header6();
        h.mss = None;
        h.flags = flags::ACK;
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        let tcp = &out[IP6_LEN..];
        let mut sum: u32 = 0;
        for pair in out[8..40].chunks(2) {
            sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
        }
        sum += 20 + 6;
        for (n, pair) in tcp.chunks(2).enumerate() {
            if n != 8 {
                sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
            }
        }
        while sum > 0xFFFF {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        assert_eq!(be16(tcp, 16), !(sum as u16));
    }

    #[test]
    fn a_flipped_bit_of_an_ipv6_frame_is_caught_except_in_unchecked_fields() {
        let mut out = Vec::new();
        encode(&mut out, &header6(), (b"payload", &[]));
        for byte in 0..out.len() {
            // The traffic class, the flow label, and the hop limit are in no
            // checksum.
            if (1..4).contains(&byte) || byte == 7 {
                continue;
            }
            let mut bad = out.clone();
            bad[byte] ^= 0x10;
            assert!(parse(&bad).is_err(), "byte {byte} went unnoticed");
        }
    }

    #[test]
    fn short_and_foreign_ipv6_frames_are_rejected() {
        let mut out = Vec::new();
        encode(&mut out, &header6(), (b"xy", &[]));
        assert_eq!(parse(&out[..30]).unwrap_err(), WireError::Short);
        assert_eq!(
            parse(&out[..out.len() - 1]).unwrap_err(),
            WireError::BadLength
        );
        let mut udp = out.clone();
        udp[6] = 17;
        assert_eq!(parse(&udp).unwrap_err(), WireError::NotTcp);
        // A hop by hop extension header in front of TCP.
        let mut ext = out.clone();
        ext[6] = 0;
        assert_eq!(parse(&ext).unwrap_err(), WireError::NotTcp);
        // Ethernet padding after the packet is not part of it.
        let mut padded = out.clone();
        padded.extend_from_slice(&[0; 20]);
        assert_eq!(parse(&padded).expect("parse").payload, b"xy");
        assert_eq!(parse(&[]).unwrap_err(), WireError::Short);
    }

    #[test]
    fn a_frame_of_each_family_parses_from_ethernet() {
        use crate::net::{encode_header, ETHERTYPE_IPV4, ETHERTYPE_IPV6};
        use fps_hosts::MacAddr;
        for (h, ethertype) in [(header(), ETHERTYPE_IPV4), (header6(), ETHERTYPE_IPV6)] {
            let mut frame = Vec::new();
            encode_header(
                &mut frame,
                MacAddr([2; 6]),
                MacAddr([4; 6]),
                Some(7),
                ethertype,
            );
            encode(&mut frame, &h, (b"hello", &[]));
            let parsed = parse_frame(&frame).expect("parse");
            assert_eq!(parsed.header, h);
            assert_eq!(parsed.payload, b"hello");
            let mut wrong = frame.clone();
            wrong[16..18].copy_from_slice(&0x0806u16.to_be_bytes());
            assert_eq!(parse_frame(&wrong).unwrap_err(), WireError::NotIp);
        }
    }

    #[test]
    fn a_syn_with_every_option_has_the_layout_of_the_kernel() {
        let mut h = header();
        h.flags = flags::SYN;
        h.opts = all_options();
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        let tcp = &out[IP_LEN..];
        // Data offset 10 words: 20 bytes of options.
        assert_eq!(tcp[12] >> 4, 10);
        assert_eq!(
            &tcp[TCP_LEN..],
            [
                2, 4, 0x05, 0xB4, // MSS 1460
                4, 2, 8, 10, 0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33,
                0x44, // SACK ok, timestamp
                1, 3, 3, 7, // window scale
            ]
        );
        assert_eq!(parse(&out).expect("parse").header, h);
        assert_eq!(options_len(&h), 20);
    }

    #[test]
    fn a_syn_without_a_timestamp_pads_the_sack_option() {
        let mut h = header();
        h.flags = flags::SYN;
        h.opts = Options {
            sack_permitted: true,
            ..Options::default()
        };
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        let at = IP_LEN + TCP_LEN;
        assert_eq!(&out[at..at + 8], [2, 4, 5, 0xB4, 1, 1, 4, 2]);
        assert_eq!(parse(&out).expect("parse").header, h);
    }

    #[test]
    fn a_data_segment_carries_a_timestamp_and_up_to_three_sack_blocks() {
        let mut h = header();
        h.flags = flags::ACK;
        h.mss = None;
        let mut o = Options {
            timestamp: Some((1, 2)),
            ..Options::default()
        };
        assert!(o.push_block(1000, 2000));
        assert!(o.push_block(3000, 4000));
        assert!(o.push_block(5000, 6000));
        // A fourth block does not fit beside the timestamp.
        assert!(!o.push_block(7000, 8000));
        h.opts = o;
        let mut out = Vec::new();
        encode(&mut out, &h, (b"xy", &[]));
        assert_eq!(options_len(&h), 12 + 4 + 24);
        let parsed = parse(&out).expect("parse");
        assert_eq!(parsed.header, h);
        assert_eq!(
            parsed.header.opts.blocks(),
            [(1000, 2000), (3000, 4000), (5000, 6000)]
        );
        assert_eq!(parsed.payload, b"xy");
    }

    #[test]
    fn four_blocks_fit_without_a_timestamp_and_the_options_stay_within_40_bytes() {
        let mut h = header();
        h.flags = flags::ACK;
        h.mss = None;
        for n in 1..=4u32 {
            let mut o = Options::default();
            for i in 0..n {
                assert!(o.push_block(i * 100, i * 100 + 50));
            }
            h.opts = o;
            let len = options_len(&h);
            assert_eq!(len, 4 + 8 * n as usize);
            assert!(len <= OPTIONS_MAX);
            let mut out = Vec::new();
            encode(&mut out, &h, (&[], &[]));
            let blocks = parse(&out).expect("parse").header.opts.sack_len;
            assert_eq!(u32::from(blocks), n);
        }
        // More blocks than the option holds are cut when encoding.
        let mut o = Options {
            timestamp: Some((1, 1)),
            sack_len: 4,
            ..Options::default()
        };
        o.sack = [(1, 2), (3, 4), (5, 6), (7, 8)];
        h.opts = o;
        assert_eq!(options_len(&h), 40);
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        assert_eq!(parse(&out).expect("parse").header.opts.blocks().len(), 3);
    }

    #[test]
    fn options_that_belong_to_a_syn_are_not_written_on_other_segments() {
        let mut h = header();
        h.flags = flags::ACK;
        h.mss = None;
        h.opts = all_options();
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        let parsed = parse(&out).expect("parse").header;
        assert_eq!(parsed.opts.wscale, None);
        assert!(!parsed.opts.sack_permitted);
        assert_eq!(parsed.opts.timestamp, h.opts.timestamp);
    }

    #[test]
    fn the_window_scale_shift_is_capped_at_14_in_both_directions() {
        let mut h = header();
        h.flags = flags::SYN;
        h.mss = None;
        h.opts.wscale = Some(30);
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        // The option is 1, 3, 3, shift after the first byte of padding.
        let shift_at = IP_LEN + TCP_LEN + 3;
        assert_eq!(out[shift_at], 14);
        // A peer that sends 20 is read as 14.
        out[shift_at] = 20;
        fix_checksums(&mut out);
        assert_eq!(parse(&out).expect("parse").header.opts.wscale, Some(14));
    }

    #[test]
    fn unknown_and_malformed_options_are_skipped_without_harm() {
        let mut h = header();
        h.flags = flags::ACK;
        h.mss = None;
        h.opts.timestamp = Some((9, 8));
        let mut out = Vec::new();
        encode(&mut out, &h, (&[], &[]));
        // Turn the two NOPs before the timestamp into an unknown option of
        // length 2.
        out[IP_LEN + TCP_LEN] = 77;
        out[IP_LEN + TCP_LEN + 1] = 2;
        fix_checksums(&mut out);
        assert_eq!(
            parse(&out).expect("parse").header.opts.timestamp,
            Some((9, 8))
        );
        // A SACK option whose length is not 2 + 8n takes no block: the
        // length 11 would run to the end of the options, so the frame is
        // rejected as malformed.
        let mut h2 = header();
        h2.flags = flags::ACK;
        h2.mss = None;
        h2.opts.push_block(1, 2);
        let mut bad = Vec::new();
        encode(&mut bad, &h2, (&[], &[]));
        bad[IP_LEN + TCP_LEN + 3] = 11;
        fix_checksums(&mut bad);
        assert_eq!(parse(&bad).unwrap_err(), WireError::BadOffset);
        // A length of 6 is well formed but not a SACK length: no block.
        bad[IP_LEN + TCP_LEN + 3] = 6;
        fix_checksums(&mut bad);
        if let Ok(parsed) = parse(&bad) {
            assert!(parsed.header.opts.blocks().is_empty());
        }
    }
}
