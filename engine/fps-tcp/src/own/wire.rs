// SPDX-License-Identifier: GPL-3.0-only
//! The frame format of the own stack: an IPv4 header without options, a TCP
//! header with at most the MSS option, and the payload. There is no Ethernet
//! header, because the stack runs over a point to point link of two hosts.
//!
//! Both checksums are computed on send and verified on receive, so the
//! measurements include their cost.

/// The length of the IPv4 header that the stack sends and accepts.
pub const IP_LEN: usize = 20;
/// The length of the TCP header without options.
pub const TCP_LEN: usize = 20;
/// The length of the MSS option.
pub const MSS_OPTION_LEN: usize = 4;

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
    /// The IPv4 source address.
    pub src_ip: u32,
    /// The IPv4 destination address.
    pub dst_ip: u32,
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
    /// The IP version or header length is not the one the stack uses.
    #[error("not an IPv4 header without options")]
    NotIpv4,
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
    let mut chunks = data.chunks_exact(4);
    for chunk in &mut chunks {
        sum += u64::from(u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    let tail = chunks.remainder();
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

fn pseudo_sum(src_ip: u32, dst_ip: u32, tcp_len: usize) -> u64 {
    u64::from(src_ip >> 16)
        + u64::from(src_ip & 0xFFFF)
        + u64::from(dst_ip >> 16)
        + u64::from(dst_ip & 0xFFFF)
        + 6
        + tcp_len as u64
}

/// Appends the frame for `header` and the payload `(first, second)` to
/// `out`. The payload comes in two slices, because a ring buffer hands out
/// two.
pub fn encode(out: &mut Vec<u8>, header: &Header, payload: (&[u8], &[u8])) {
    let options = if header.mss.is_some() {
        MSS_OPTION_LEN
    } else {
        0
    };
    let tcp_len = TCP_LEN + options + payload.0.len() + payload.1.len();
    let total = IP_LEN + tcp_len;
    let base = out.len();
    out.reserve(total);
    out.extend_from_slice(&[0x45, 0]);
    out.extend_from_slice(&(total as u16).to_be_bytes());
    out.extend_from_slice(&[0, 0, 0x40, 0, 64, 6, 0, 0]);
    out.extend_from_slice(&header.src_ip.to_be_bytes());
    out.extend_from_slice(&header.dst_ip.to_be_bytes());
    let ip_sum = !fold(sum_words(0, &out[base..base + IP_LEN]));
    out[base + 10..base + 12].copy_from_slice(&ip_sum.to_be_bytes());
    out.extend_from_slice(&header.src_port.to_be_bytes());
    out.extend_from_slice(&header.dst_port.to_be_bytes());
    out.extend_from_slice(&header.seq.to_be_bytes());
    out.extend_from_slice(&header.ack.to_be_bytes());
    out.push((((TCP_LEN + options) / 4) as u8) << 4);
    out.push(header.flags);
    out.extend_from_slice(&header.window.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    if let Some(mss) = header.mss {
        out.extend_from_slice(&[2, 4]);
        out.extend_from_slice(&mss.to_be_bytes());
    }
    out.extend_from_slice(payload.0);
    out.extend_from_slice(payload.1);
    let tcp_start = base + IP_LEN;
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

/// Parses and verifies one frame.
pub fn parse(frame: &[u8]) -> Result<Parsed<'_>, WireError> {
    if frame.len() < IP_LEN + TCP_LEN {
        return Err(WireError::Short);
    }
    if frame[0] != 0x45 {
        return Err(WireError::NotIpv4);
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
    let src_ip = be32(frame, 12);
    let dst_ip = be32(frame, 16);
    let tcp = &frame[IP_LEN..total];
    let sum = pseudo_sum(src_ip, dst_ip, tcp.len()) + sum_words(0, tcp);
    if fold(sum) != 0xFFFF {
        return Err(WireError::BadTcpChecksum);
    }
    let offset = usize::from(tcp[12] >> 4) * 4;
    if offset < TCP_LEN || offset > tcp.len() {
        return Err(WireError::BadOffset);
    }
    let mut mss = None;
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
                if kind == 2 && len == 4 {
                    mss = Some(be16(options, 2));
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
        },
        payload: &tcp[offset..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            src_ip: 0x0A00_0001,
            dst_ip: 0x0A00_0002,
            src_port: 40000,
            dst_port: 80,
            seq: 0xFFFF_FFF0,
            ack: 77,
            flags: flags::SYN | flags::ACK,
            window: 65535,
            mss: Some(1460),
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
        let mut v6 = out.clone();
        v6[0] = 0x60;
        assert_eq!(parse(&v6).unwrap_err(), WireError::NotIpv4);
        assert_eq!(
            parse(&out[..out.len() - 1]).unwrap_err(),
            WireError::BadLength
        );
    }
}
