// SPDX-License-Identifier: GPL-3.0-only
//! The frame format of the UDP layer: an Ethernet header (built by the caller),
//! an IPv4 header without options, a UDP header, and the payload.
//!
//! Both checksums are computed on send and verified on receive. A datagram
//! that does not fit the MTU leaves as IPv4 fragments (RFC 791). The receive
//! side does not reassemble: [`parse`] answers [`RxError::Fragment`] for
//! every fragment, so the layer drops it and counts it.

use std::net::Ipv4Addr;

/// The length of the IPv4 header that the layer sends.
pub const IP_LEN: usize = 20;
/// The length of the UDP header.
pub const UDP_LEN: usize = 8;
/// The IP protocol number of UDP.
pub const PROTO_UDP: u8 = 17;
/// The smallest MTU that carries an IPv4 packet with a fragment of data
/// (RFC 791: every host accepts 68 bytes).
pub const MIN_MTU: usize = 68;

/// The more fragments flag in the flags and offset field.
const MORE_FRAGMENTS: u16 = 0x2000;
/// The mask of the fragment offset in the flags and offset field.
const OFFSET_MASK: u16 = 0x1FFF;

/// Adds the 16 bit words of `data` (big endian, an odd last byte padded with
/// zero) to `sum`. The sum stays below 2^63 for any frame.
fn sum_words(mut sum: u64, data: &[u8]) -> u64 {
    let (quads, rest) = data.as_chunks::<8>();
    for quad in quads {
        let v = u64::from_be_bytes(*quad);
        sum += (v >> 48) + ((v >> 32) & 0xFFFF) + ((v >> 16) & 0xFFFF) + (v & 0xFFFF);
    }
    let (pairs, tail) = rest.as_chunks::<2>();
    for pair in pairs {
        sum += u64::from(u16::from_be_bytes(*pair));
    }
    if let [last] = tail {
        sum += u64::from(*last) << 8;
    }
    sum
}

/// Folds a sum of words into the 16 bit ones' complement sum.
fn fold(mut sum: u64) -> u16 {
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    // The loop leaves at most 16 bits.
    u16::try_from(sum).unwrap_or(u16::MAX)
}

/// The Internet checksum of `data`: the complement of the ones' complement sum.
#[must_use]
pub fn checksum(data: &[u8]) -> u16 {
    !fold(sum_words(0, data))
}

/// The sum of the UDP pseudo header (RFC 768).
fn pseudo_sum(src: [u8; 4], dst: [u8; 4], udp_len: usize) -> u64 {
    let mut sum = sum_words(0, &src);
    sum = sum_words(sum, &dst);
    sum + u64::from(PROTO_UDP) + udp_len as u64
}

/// Why a received packet is not a datagram for the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RxError {
    /// Shorter than its IPv4 header or its UDP header, or a length field that
    /// contradicts the frame.
    Malformed,
    /// Not IPv4, or an IPv4 header that breaks its own rules.
    NotIpv4,
    /// The IPv4 header checksum is wrong.
    IpChecksum,
    /// The packet is a fragment. The layer does not reassemble.
    Fragment,
    /// The protocol is not UDP.
    NotUdp,
    /// The UDP checksum is wrong. A checksum of zero means none was computed
    /// and is accepted.
    UdpChecksum,
}

/// A verified UDP datagram inside an IPv4 packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parsed<'a> {
    /// The IPv4 source address.
    pub src_ip: Ipv4Addr,
    /// The IPv4 destination address.
    pub dst_ip: Ipv4Addr,
    /// The UDP source port.
    pub src_port: u16,
    /// The UDP destination port.
    pub dst_port: u16,
    /// The UDP payload, without the padding that an Ethernet frame adds.
    pub payload: &'a [u8],
}

/// Reads one UDP datagram from an IPv4 packet (the slice starts at the IPv4
/// header and may be longer than the packet, as a padded Ethernet frame is).
///
/// # Errors
///
/// Returns the reason that the packet is not a datagram for the layer.
pub fn parse(ip: &[u8]) -> Result<Parsed<'_>, RxError> {
    let head = ip.get(..IP_LEN).ok_or(RxError::Malformed)?;
    if head[0] >> 4 != 4 {
        return Err(RxError::NotIpv4);
    }
    let ihl = usize::from(head[0] & 0x0F) * 4;
    if ihl < IP_LEN {
        return Err(RxError::NotIpv4);
    }
    let header = ip.get(..ihl).ok_or(RxError::Malformed)?;
    let total = usize::from(u16::from_be_bytes([head[2], head[3]]));
    if total < ihl || total > ip.len() {
        return Err(RxError::Malformed);
    }
    if fold(sum_words(0, header)) != 0xFFFF {
        return Err(RxError::IpChecksum);
    }
    let frag = u16::from_be_bytes([head[6], head[7]]);
    if frag & (MORE_FRAGMENTS | OFFSET_MASK) != 0 {
        return Err(RxError::Fragment);
    }
    if head[9] != PROTO_UDP {
        return Err(RxError::NotUdp);
    }
    let datagram = ip.get(ihl..total).ok_or(RxError::Malformed)?;
    let udp = datagram.get(..UDP_LEN).ok_or(RxError::Malformed)?;
    let udp_len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if udp_len < UDP_LEN || udp_len > datagram.len() {
        return Err(RxError::Malformed);
    }
    let datagram = &datagram[..udp_len];
    let src = [head[12], head[13], head[14], head[15]];
    let dst = [head[16], head[17], head[18], head[19]];
    if u16::from_be_bytes([udp[6], udp[7]]) != 0 {
        let sum = sum_words(pseudo_sum(src, dst, udp_len), datagram);
        if fold(sum) != 0xFFFF {
            return Err(RxError::UdpChecksum);
        }
    }
    Ok(Parsed {
        src_ip: Ipv4Addr::from(src),
        dst_ip: Ipv4Addr::from(dst),
        src_port: u16::from_be_bytes([udp[0], udp[1]]),
        dst_port: u16::from_be_bytes([udp[2], udp[3]]),
        payload: &datagram[UDP_LEN..],
    })
}

/// The fields of a datagram to send.
#[derive(Clone, Copy, Debug)]
pub struct Outgoing<'a> {
    /// The IPv4 source address.
    pub src_ip: Ipv4Addr,
    /// The IPv4 destination address.
    pub dst_ip: Ipv4Addr,
    /// The UDP source port.
    pub src_port: u16,
    /// The UDP destination port.
    pub dst_port: u16,
    /// The time to live.
    pub ttl: u8,
    /// The IPv4 identification, which the fragments of one datagram share.
    pub id: u16,
    /// The payload.
    pub payload: &'a [u8],
}

/// The datagram does not fit in an IPv4 packet (65,535 bytes with the IPv4
/// header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

/// The data bytes of a fragment: a multiple of 8 that fits the MTU.
fn fragment_chunk(mtu: usize) -> usize {
    ((mtu.max(MIN_MTU) - IP_LEN) / 8) * 8
}

/// The number of frames that [`build`] makes for a payload of `len` bytes at
/// `mtu`.
#[must_use]
pub fn frames_for(len: usize, mtu: usize) -> usize {
    let datagram = UDP_LEN + len;
    if IP_LEN + datagram <= mtu {
        return 1;
    }
    datagram.div_ceil(fragment_chunk(mtu))
}

/// Writes an IPv4 header with its checksum to the end of `out`.
fn ip_header(out: &mut Vec<u8>, d: &Outgoing<'_>, total: u16, frag: u16) {
    let at = out.len();
    out.extend_from_slice(&[0x45, 0]);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(&d.id.to_be_bytes());
    out.extend_from_slice(&frag.to_be_bytes());
    out.extend_from_slice(&[d.ttl, PROTO_UDP, 0, 0]);
    out.extend_from_slice(&d.src_ip.octets());
    out.extend_from_slice(&d.dst_ip.octets());
    let sum = checksum(&out[at..at + IP_LEN]).to_be_bytes();
    out[at + 10] = sum[0];
    out[at + 11] = sum[1];
}

/// Builds the frames of one datagram and appends them to `frames`: each is
/// `head` (the Ethernet header) followed by an IPv4 packet, padded with zeros
/// to `min_frame`. A datagram that exceeds `mtu` leaves as fragments that
/// carry no more than `mtu` bytes of IP each. The UDP checksum covers the
/// whole datagram and travels in the first fragment.
///
/// # Errors
///
/// Returns [`TooLong`] when the datagram does not fit an IPv4 packet.
pub fn build(
    head: &[u8],
    d: &Outgoing<'_>,
    mtu: usize,
    min_frame: usize,
    frames: &mut Vec<Vec<u8>>,
) -> Result<(), TooLong> {
    let datagram_len = UDP_LEN + d.payload.len();
    if IP_LEN + datagram_len > usize::from(u16::MAX) {
        return Err(TooLong);
    }
    let udp_len = u16::try_from(datagram_len).map_err(|_| TooLong)?;
    // The UDP header, with the checksum over pseudo header, header, payload.
    let mut udp = [0u8; UDP_LEN];
    udp[0..2].copy_from_slice(&d.src_port.to_be_bytes());
    udp[2..4].copy_from_slice(&d.dst_port.to_be_bytes());
    udp[4..6].copy_from_slice(&udp_len.to_be_bytes());
    let sum = sum_words(
        pseudo_sum(d.src_ip.octets(), d.dst_ip.octets(), datagram_len),
        &udp,
    );
    let sum = !fold(sum_words(sum, d.payload));
    // A computed checksum of zero goes on the wire as all ones (RFC 768).
    let sum = if sum == 0 { 0xFFFF } else { sum };
    udp[6..8].copy_from_slice(&sum.to_be_bytes());

    let single = IP_LEN + datagram_len <= mtu;
    let chunk = if single {
        datagram_len
    } else {
        fragment_chunk(mtu)
    };
    let mut offset = 0;
    while offset < datagram_len {
        let take = chunk.min(datagram_len - offset);
        let end = offset + take;
        let mut frame = Vec::with_capacity(head.len() + IP_LEN + take);
        frame.extend_from_slice(head);
        let frag = if single {
            0
        } else {
            let more = if end >= datagram_len {
                0
            } else {
                MORE_FRAGMENTS
            };
            more | u16::try_from(offset / 8).map_err(|_| TooLong)?
        };
        let total = u16::try_from(IP_LEN + take).map_err(|_| TooLong)?;
        ip_header(&mut frame, d, total, frag);
        // The bytes `offset..end` of the datagram: the UDP header, then the
        // payload.
        let mut at = offset;
        if at < UDP_LEN {
            let upto = end.min(UDP_LEN);
            frame.extend_from_slice(&udp[at..upto]);
            at = upto;
        }
        if at < end {
            frame.extend_from_slice(&d.payload[at - UDP_LEN..end - UDP_LEN]);
        }
        if frame.len() < min_frame {
            frame.resize(min_frame, 0);
        }
        frames.push(frame);
        offset = end;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn outgoing(payload: &[u8]) -> Outgoing<'_> {
        Outgoing {
            src_ip: Ipv4Addr::new(10, 0, 0, 1),
            dst_ip: Ipv4Addr::new(11, 0, 0, 2),
            src_port: 40000,
            dst_port: 53,
            ttl: 64,
            id: 7,
            payload,
        }
    }

    fn one_frame(payload: &[u8]) -> Vec<u8> {
        let mut frames = Vec::new();
        build(&[0; 14], &outgoing(payload), 1500, 60, &mut frames).unwrap();
        assert_eq!(frames.len(), 1);
        frames.remove(0)
    }

    #[test]
    fn checksum_matches_the_rfc_1071_example() {
        // RFC 1071 section 3: the sum of these words is 0xDDF2, so the
        // checksum is 0x220D.
        let data = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(checksum(&data), 0x220d);
        // An odd length pads with a zero byte.
        assert_eq!(checksum(&[0x12]), !0x1200);
        assert_eq!(checksum(&[]), 0xFFFF);
    }

    #[test]
    fn a_built_frame_parses_back_and_pads_to_the_minimum() {
        let frame = one_frame(b"hello");
        assert_eq!(frame.len(), 60, "14 + 20 + 8 + 5 padded to 60");
        let p = parse(&frame[14..]).unwrap();
        assert_eq!(p.payload, b"hello");
        assert_eq!((p.src_port, p.dst_port), (40000, 53));
        assert_eq!(p.src_ip, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(p.dst_ip, Ipv4Addr::new(11, 0, 0, 2));
    }

    #[test]
    fn every_corrupted_byte_is_caught_by_a_checksum() {
        let frame = one_frame(&[0x5a; 40]);
        let ip = &frame[14..];
        // The checksums cover bytes 0..20 of the IP header and the whole UDP
        // datagram. Flip each byte in turn: parse must refuse every one,
        // except where the flip only changes a field that no rule checks
        // (the TOS byte, which the header checksum still covers).
        for i in 0..IP_LEN + UDP_LEN + 40 {
            let mut bad = ip.to_vec();
            bad[i] ^= 0x01;
            assert!(parse(&bad).is_err(), "byte {i} flipped went unnoticed");
        }
    }

    #[test]
    fn a_zero_udp_checksum_means_none_and_is_accepted() {
        let mut frame = one_frame(b"abc");
        frame[14 + IP_LEN + 6] = 0;
        frame[14 + IP_LEN + 7] = 0;
        assert_eq!(parse(&frame[14..]).unwrap().payload, b"abc");
    }

    #[test]
    fn a_computed_checksum_of_zero_is_sent_as_all_ones() {
        // With a payload of two zero bytes the checksum field is `f`. A
        // payload of `f` adds `f` to the sum and makes it 0xFFFF, so the
        // checksum computes to zero.
        let first = one_frame(&[0, 0]);
        let f = [first[14 + IP_LEN + 6], first[14 + IP_LEN + 7]];
        let frame = one_frame(&f);
        let field = u16::from_be_bytes([frame[14 + IP_LEN + 6], frame[14 + IP_LEN + 7]]);
        assert_eq!(field, 0xFFFF, "a computed zero goes out as all ones");
        assert_eq!(parse(&frame[14..]).unwrap().payload, &f);
    }

    #[test]
    fn fragments_split_on_eight_byte_boundaries_and_carry_the_header_once() {
        let payload: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
        let mut frames = Vec::new();
        build(&[0; 14], &outgoing(&payload), 1500, 60, &mut frames).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames.len(), frames_for(payload.len(), 1500));
        let mut data = Vec::new();
        for (n, f) in frames.iter().enumerate() {
            let ip = &f[14..];
            let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
            assert!(total <= 1500);
            assert_eq!(fold(sum_words(0, &ip[..IP_LEN])), 0xFFFF, "header checksum");
            let frag = u16::from_be_bytes([ip[6], ip[7]]);
            assert_eq!(usize::from(frag & OFFSET_MASK) * 8, data.len(), "offset");
            assert_eq!(frag & MORE_FRAGMENTS != 0, n + 1 < frames.len(), "MF");
            assert_eq!(u16::from_be_bytes([ip[4], ip[5]]), 7, "same id");
            // Every fragment is refused by the receive side.
            assert_eq!(parse(ip), Err(RxError::Fragment));
            data.extend_from_slice(&ip[IP_LEN..total]);
        }
        // The reassembled datagram verifies as one UDP datagram.
        assert_eq!(data.len(), UDP_LEN + payload.len());
        let mut whole = frames[0][14..14 + IP_LEN].to_vec();
        whole[2..4].copy_from_slice(&((IP_LEN + data.len()) as u16).to_be_bytes());
        whole[6..8].copy_from_slice(&[0, 0]);
        whole[10..12].copy_from_slice(&[0, 0]);
        let sum = checksum(&whole).to_be_bytes();
        whole[10..12].copy_from_slice(&sum);
        whole.extend_from_slice(&data);
        let p = parse(&whole).unwrap();
        assert_eq!(p.payload, &payload[..]);
    }

    #[test]
    fn the_largest_datagram_fits_and_one_more_byte_does_not() {
        let max = fps_io::MAX_DATAGRAM;
        let mut frames = Vec::new();
        build(&[0; 14], &outgoing(&vec![1; max]), 1500, 60, &mut frames).unwrap();
        assert_eq!(frames.len(), frames_for(max, 1500));
        assert_eq!(
            build(
                &[0; 14],
                &outgoing(&vec![1; max + 1]),
                1500,
                60,
                &mut frames
            ),
            Err(TooLong)
        );
    }

    #[test]
    fn malformed_packets_are_named() {
        let frame = one_frame(b"x");
        let ip = &frame[14..];
        assert_eq!(parse(&ip[..10]), Err(RxError::Malformed));
        let mut v6 = ip.to_vec();
        v6[0] = 0x60;
        assert_eq!(parse(&v6), Err(RxError::NotIpv4));
        let mut short_ihl = ip.to_vec();
        short_ihl[0] = 0x44;
        assert_eq!(parse(&short_ihl), Err(RxError::NotIpv4));
        // A total length beyond the frame.
        let mut long = ip.to_vec();
        long[2..4].copy_from_slice(&1000u16.to_be_bytes());
        assert_eq!(parse(&long), Err(RxError::Malformed));
        // TCP instead of UDP, with a valid header checksum.
        let mut tcp = ip.to_vec();
        tcp[9] = 6;
        tcp[10..12].copy_from_slice(&[0, 0]);
        let sum = checksum(&tcp[..IP_LEN]).to_be_bytes();
        tcp[10..12].copy_from_slice(&sum);
        assert_eq!(parse(&tcp), Err(RxError::NotUdp));
        // A UDP length that contradicts the IP length, with valid checksums
        // would still be caught by the length check first.
        let mut udp_len = ip.to_vec();
        udp_len[IP_LEN + 4..IP_LEN + 6].copy_from_slice(&200u16.to_be_bytes());
        assert_eq!(parse(&udp_len), Err(RxError::Malformed));
    }
}
