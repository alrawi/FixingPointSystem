// SPDX-License-Identifier: GPL-3.0-only
//! The frame format of the UDP layer: an Ethernet header (built by the caller),
//! an IPv4 header without options or an IPv6 header without extension headers
//! (TRN-30), a UDP header, and the payload.
//!
//! Both checksums of IPv4 and the UDP checksum of either family are computed
//! on send and verified on receive. IPv6 has no header checksum, and its UDP
//! checksum is required: a datagram with a checksum of zero is dropped as a
//! wrong checksum (RFC 8200 section 8.1), where IPv4 accepts it as none
//! computed. A datagram that does not fit the MTU leaves as IPv4 fragments
//! (RFC 791) or as IPv6 fragments with a fragment header (RFC 8200 section
//! 4.5). The receive side does not reassemble: [`parse`] answers
//! [`RxError::Fragment`] for every fragment, so the layer drops it and counts
//! it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The length of the IPv4 header that the layer sends.
pub const IP_LEN: usize = 20;
/// The length of the IPv6 header that the layer sends and accepts.
pub const IP6_LEN: usize = 40;
/// The length of the IPv6 fragment header.
pub const FRAG6_LEN: usize = 8;
/// The length of the UDP header.
pub const UDP_LEN: usize = 8;
/// The IP protocol number of UDP.
pub const PROTO_UDP: u8 = 17;
/// The IPv6 next header value of the fragment header.
const NEXT_HEADER_FRAGMENT: u8 = 44;
/// The smallest MTU that carries an IPv4 packet with a fragment of data
/// (RFC 791: every host accepts 68 bytes). An IPv6 packet with a fragment
/// header and 16 bytes of data fits as well.
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

fn octets4(ip: IpAddr) -> [u8; 4] {
    match ip {
        IpAddr::V4(a) => a.octets(),
        // A pair of mixed families is a bug of the caller; it goes out as a
        // packet of the family of its source.
        IpAddr::V6(_) => [0; 4],
    }
}

fn octets6(ip: IpAddr) -> [u8; 16] {
    match ip {
        IpAddr::V6(a) => a.octets(),
        IpAddr::V4(_) => [0; 16],
    }
}

/// The sum of the UDP pseudo header (RFC 768 for IPv4, RFC 8200 section 8.1
/// for IPv6, where the length is 32 bits and the upper 16 are zero).
fn pseudo_sum(src: IpAddr, dst: IpAddr, udp_len: usize) -> u64 {
    let sum = match src {
        IpAddr::V4(_) => sum_words(sum_words(0, &octets4(src)), &octets4(dst)),
        IpAddr::V6(_) => sum_words(sum_words(0, &octets6(src)), &octets6(dst)),
    };
    sum + u64::from(PROTO_UDP) + udp_len as u64
}

/// Why a received packet is not a datagram for the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RxError {
    /// Shorter than its IP header or its UDP header, or a length field that
    /// contradicts the frame.
    Malformed,
    /// Neither IPv4 nor IPv6, or an IPv4 header that breaks its own rules.
    NotIp,
    /// The IPv4 header checksum is wrong.
    IpChecksum,
    /// The packet is a fragment. The layer does not reassemble.
    Fragment,
    /// The protocol is not UDP (or an IPv6 packet has another extension
    /// header than the fragment header, which the layer does not read).
    NotUdp,
    /// The UDP checksum is wrong. An IPv4 checksum of zero means none was
    /// computed and is accepted. An IPv6 checksum of zero is wrong.
    UdpChecksum,
}

/// A verified UDP datagram inside an IP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parsed<'a> {
    /// The source address.
    pub src_ip: IpAddr,
    /// The destination address.
    pub dst_ip: IpAddr,
    /// The UDP source port.
    pub src_port: u16,
    /// The UDP destination port.
    pub dst_port: u16,
    /// The UDP payload, without the padding that an Ethernet frame adds.
    pub payload: &'a [u8],
}

/// Reads one UDP datagram from an IPv4 or IPv6 packet (the slice starts at the
/// IP header and may be longer than the packet, as a padded Ethernet frame is).
///
/// # Errors
///
/// Returns the reason that the packet is not a datagram for the layer.
pub fn parse(ip: &[u8]) -> Result<Parsed<'_>, RxError> {
    match ip.first().map(|b| b >> 4) {
        Some(4) => parse4(ip),
        Some(6) => parse6(ip),
        Some(_) => Err(RxError::NotIp),
        None => Err(RxError::Malformed),
    }
}

fn parse4(ip: &[u8]) -> Result<Parsed<'_>, RxError> {
    let head = ip.get(..IP_LEN).ok_or(RxError::Malformed)?;
    let ihl = usize::from(head[0] & 0x0F) * 4;
    if ihl < IP_LEN {
        return Err(RxError::NotIp);
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
    let src = IpAddr::V4(Ipv4Addr::new(head[12], head[13], head[14], head[15]));
    let dst = IpAddr::V4(Ipv4Addr::new(head[16], head[17], head[18], head[19]));
    let datagram = ip.get(ihl..total).ok_or(RxError::Malformed)?;
    parse_udp(src, dst, datagram, true)
}

fn parse6(ip: &[u8]) -> Result<Parsed<'_>, RxError> {
    let head = ip.get(..IP6_LEN).ok_or(RxError::Malformed)?;
    let payload = usize::from(u16::from_be_bytes([head[4], head[5]]));
    let datagram = ip
        .get(IP6_LEN..IP6_LEN + payload)
        .ok_or(RxError::Malformed)?;
    match head[6] {
        PROTO_UDP => {}
        NEXT_HEADER_FRAGMENT => return Err(RxError::Fragment),
        _ => return Err(RxError::NotUdp),
    }
    let addr = |at: usize| {
        let mut octets = [0u8; 16];
        octets.copy_from_slice(&head[at..at + 16]);
        IpAddr::V6(Ipv6Addr::from(octets))
    };
    parse_udp(addr(8), addr(24), datagram, false)
}

/// Reads the UDP header of `datagram` and verifies the checksum. IPv4 accepts
/// a checksum of zero (`zero_ok`), IPv6 does not.
fn parse_udp(
    src: IpAddr,
    dst: IpAddr,
    datagram: &[u8],
    zero_ok: bool,
) -> Result<Parsed<'_>, RxError> {
    let udp = datagram.get(..UDP_LEN).ok_or(RxError::Malformed)?;
    let udp_len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if udp_len < UDP_LEN || udp_len > datagram.len() {
        return Err(RxError::Malformed);
    }
    let datagram = &datagram[..udp_len];
    let field = u16::from_be_bytes([udp[6], udp[7]]);
    if field == 0 && !zero_ok {
        return Err(RxError::UdpChecksum);
    }
    if field != 0 {
        let sum = sum_words(pseudo_sum(src, dst, udp_len), datagram);
        if fold(sum) != 0xFFFF {
            return Err(RxError::UdpChecksum);
        }
    }
    Ok(Parsed {
        src_ip: src,
        dst_ip: dst,
        src_port: u16::from_be_bytes([udp[0], udp[1]]),
        dst_port: u16::from_be_bytes([udp[2], udp[3]]),
        payload: &datagram[UDP_LEN..],
    })
}

/// The fields of a datagram to send.
#[derive(Clone, Copy, Debug)]
pub struct Outgoing<'a> {
    /// The source address. Both addresses are of one family, which is the
    /// family of the packets.
    pub src_ip: IpAddr,
    /// The destination address.
    pub dst_ip: IpAddr,
    /// The UDP source port.
    pub src_port: u16,
    /// The UDP destination port.
    pub dst_port: u16,
    /// The time to live, or the hop limit of IPv6.
    pub ttl: u8,
    /// The identification that the fragments of one datagram share: the low
    /// 16 bits for IPv4, all 32 for the IPv6 fragment header.
    pub id: u32,
    /// The payload.
    pub payload: &'a [u8],
}

/// The datagram does not fit in an IP packet (65,535 bytes with the IPv4
/// header, or 65,535 bytes after the IPv6 header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

/// The length of the IP header of the packets from `src`.
fn ip_len(src: IpAddr) -> usize {
    if src.is_ipv6() {
        IP6_LEN
    } else {
        IP_LEN
    }
}

/// The data bytes of a fragment: a multiple of 8 that fits the MTU.
fn fragment_chunk(mtu: usize, v6: bool) -> usize {
    let header = if v6 { IP6_LEN + FRAG6_LEN } else { IP_LEN };
    (mtu.max(MIN_MTU).saturating_sub(header) / 8) * 8
}

/// The number of frames that [`build`] makes for a payload of `len` bytes at
/// `mtu`, sent from `src`.
#[must_use]
pub fn frames_for(len: usize, mtu: usize, src: IpAddr) -> usize {
    let datagram = UDP_LEN + len;
    if ip_len(src) + datagram <= mtu {
        return 1;
    }
    datagram.div_ceil(fragment_chunk(mtu, src.is_ipv6()))
}

/// Writes an IPv4 header with its checksum to the end of `out`.
fn ip_header(out: &mut Vec<u8>, d: &Outgoing<'_>, total: u16, frag: u16) {
    let at = out.len();
    out.extend_from_slice(&[0x45, 0]);
    out.extend_from_slice(&total.to_be_bytes());
    // The identification of IPv4 has 16 bits.
    out.extend_from_slice(&(d.id as u16).to_be_bytes());
    out.extend_from_slice(&frag.to_be_bytes());
    out.extend_from_slice(&[d.ttl, PROTO_UDP, 0, 0]);
    out.extend_from_slice(&octets4(d.src_ip));
    out.extend_from_slice(&octets4(d.dst_ip));
    let sum = checksum(&out[at..at + IP_LEN]).to_be_bytes();
    out[at + 10] = sum[0];
    out[at + 11] = sum[1];
}

/// Writes an IPv6 header to the end of `out`.
fn ip6_header(out: &mut Vec<u8>, d: &Outgoing<'_>, payload_len: u16, next_header: u8) {
    out.extend_from_slice(&[0x60, 0, 0, 0]);
    out.extend_from_slice(&payload_len.to_be_bytes());
    out.extend_from_slice(&[next_header, d.ttl]);
    out.extend_from_slice(&octets6(d.src_ip));
    out.extend_from_slice(&octets6(d.dst_ip));
}

/// Builds the frames of one datagram and appends them to `frames`: each is
/// `head` (the Ethernet header) followed by an IP packet, padded with zeros
/// to `min_frame`. The packets are IPv6 when the source is an IPv6 address
/// and IPv4 otherwise. A datagram that exceeds `mtu` leaves as fragments that
/// carry no more than `mtu` bytes of IP each. The UDP checksum covers the
/// whole datagram and travels in the first fragment.
///
/// # Errors
///
/// Returns [`TooLong`] when the datagram does not fit an IP packet.
pub fn build(
    head: &[u8],
    d: &Outgoing<'_>,
    mtu: usize,
    min_frame: usize,
    frames: &mut Vec<Vec<u8>>,
) -> Result<(), TooLong> {
    let v6 = d.src_ip.is_ipv6();
    let ip_len = ip_len(d.src_ip);
    let datagram_len = UDP_LEN + d.payload.len();
    // IPv4 counts its header in the total length, IPv6 does not.
    let limit = if v6 {
        usize::from(u16::MAX)
    } else {
        usize::from(u16::MAX) - IP_LEN
    };
    if datagram_len > limit {
        return Err(TooLong);
    }
    let udp_len = u16::try_from(datagram_len).map_err(|_| TooLong)?;
    // The UDP header, with the checksum over pseudo header, header, payload.
    let mut udp = [0u8; UDP_LEN];
    udp[0..2].copy_from_slice(&d.src_port.to_be_bytes());
    udp[2..4].copy_from_slice(&d.dst_port.to_be_bytes());
    udp[4..6].copy_from_slice(&udp_len.to_be_bytes());
    let sum = sum_words(pseudo_sum(d.src_ip, d.dst_ip, datagram_len), &udp);
    let sum = !fold(sum_words(sum, d.payload));
    // A computed checksum of zero goes on the wire as all ones (RFC 768).
    let sum = if sum == 0 { 0xFFFF } else { sum };
    udp[6..8].copy_from_slice(&sum.to_be_bytes());

    let single = ip_len + datagram_len <= mtu;
    let chunk = if single {
        datagram_len
    } else {
        fragment_chunk(mtu, v6)
    };
    let mut offset = 0;
    while offset < datagram_len {
        let take = chunk.min(datagram_len - offset);
        let end = offset + take;
        let mut frame = Vec::with_capacity(head.len() + ip_len + FRAG6_LEN + take);
        frame.extend_from_slice(head);
        let more = end < datagram_len;
        let frag = if single {
            0
        } else {
            let more = if more { MORE_FRAGMENTS } else { 0 };
            more | u16::try_from(offset / 8).map_err(|_| TooLong)?
        };
        match (v6, single) {
            (false, _) => {
                let total = u16::try_from(IP_LEN + take).map_err(|_| TooLong)?;
                ip_header(&mut frame, d, total, frag);
            }
            (true, true) => ip6_header(&mut frame, d, udp_len, PROTO_UDP),
            (true, false) => {
                let payload = u16::try_from(FRAG6_LEN + take).map_err(|_| TooLong)?;
                ip6_header(&mut frame, d, payload, NEXT_HEADER_FRAGMENT);
                // The fragment header: the next header, a reserved byte, the
                // offset and the more flag (in the position of IPv4), and the
                // identification of 32 bits.
                frame.extend_from_slice(&[PROTO_UDP, 0]);
                let offset_field = ((frag & OFFSET_MASK) << 3) | u16::from(more);
                frame.extend_from_slice(&offset_field.to_be_bytes());
                frame.extend_from_slice(&d.id.to_be_bytes());
            }
        }
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
            src_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            dst_ip: IpAddr::V4(Ipv4Addr::new(11, 0, 0, 2)),
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
        assert_eq!(p.src_ip, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(p.dst_ip, IpAddr::V4(Ipv4Addr::new(11, 0, 0, 2)));
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
        assert_eq!(
            frames.len(),
            frames_for(payload.len(), 1500, outgoing(&[]).src_ip)
        );
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
        assert_eq!(frames.len(), frames_for(max, 1500, outgoing(&[]).src_ip));
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
        let mut v7 = ip.to_vec();
        v7[0] = 0x70;
        assert_eq!(parse(&v7), Err(RxError::NotIp));
        let mut short_ihl = ip.to_vec();
        short_ihl[0] = 0x44;
        assert_eq!(parse(&short_ihl), Err(RxError::NotIp));
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

    fn outgoing6(payload: &[u8]) -> Outgoing<'_> {
        Outgoing {
            src_ip: "fd00:1::1".parse::<Ipv6Addr>().unwrap().into(),
            dst_ip: "fd00:2::2".parse::<Ipv6Addr>().unwrap().into(),
            id: 0xA1B2_C3D4,
            ..outgoing(payload)
        }
    }

    fn one_frame6(payload: &[u8]) -> Vec<u8> {
        let mut frames = Vec::new();
        build(&[0; 14], &outgoing6(payload), 1500, 60, &mut frames).unwrap();
        assert_eq!(frames.len(), 1);
        frames.remove(0)
    }

    #[test]
    fn an_ipv6_frame_parses_back_and_pads_to_the_minimum() {
        let frame = one_frame6(b"hello");
        // 14 + 40 + 8 + 5 = 67 bytes, above the minimum, so no padding.
        assert_eq!(frame.len(), 67);
        assert_eq!(frame[14] >> 4, 6);
        assert_eq!(frame[14 + 6], PROTO_UDP);
        assert_eq!(frame[14 + 7], 64, "hop limit");
        let p = parse(&frame[14..]).unwrap();
        assert_eq!(p.payload, b"hello");
        assert_eq!((p.src_port, p.dst_port), (40000, 53));
        assert_eq!(p.src_ip, outgoing6(&[]).src_ip);
        assert_eq!(p.dst_ip, outgoing6(&[]).dst_ip);
        // Ethernet padding after the packet is not part of it.
        let mut padded = frame[14..].to_vec();
        padded.extend_from_slice(&[0; 30]);
        assert_eq!(parse(&padded).unwrap().payload, b"hello");
    }

    #[test]
    fn an_ipv6_checksum_matches_a_hand_computed_value() {
        let frame = one_frame6(&[0x12, 0x34]);
        let ip = &frame[14..];
        let mut sum: u32 = 0;
        for pair in ip[8..40].chunks(2) {
            sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
        }
        // The length of the datagram and the next header 17.
        sum += 10 + 17;
        // Ports, length, and the payload; the checksum field counts as zero.
        for word in [40000u32, 53, 10, 0x1234] {
            sum += word;
        }
        while sum > 0xFFFF {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        let expected = !(sum as u16);
        assert_eq!(
            u16::from_be_bytes([ip[IP6_LEN + 6], ip[IP6_LEN + 7]]),
            expected
        );
    }

    #[test]
    fn a_corrupted_byte_of_an_ipv6_frame_is_caught_except_in_unchecked_fields() {
        let frame = one_frame6(&[0x5a; 40]);
        let ip = &frame[14..];
        for i in 0..IP6_LEN + UDP_LEN + 40 {
            // The traffic class, the flow label, and the hop limit are in no
            // checksum.
            if i < 4 || i == 7 {
                continue;
            }
            let mut bad = ip.to_vec();
            bad[i] ^= 0x01;
            assert!(parse(&bad).is_err(), "byte {i} flipped went unnoticed");
        }
    }

    #[test]
    fn an_ipv6_udp_checksum_of_zero_is_wrong() {
        let mut frame = one_frame6(b"abc");
        frame[14 + IP6_LEN + 6] = 0;
        frame[14 + IP6_LEN + 7] = 0;
        assert_eq!(parse(&frame[14..]), Err(RxError::UdpChecksum));
    }

    #[test]
    fn ipv6_fragments_carry_a_fragment_header_and_are_refused_on_receive() {
        let payload: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
        let d = outgoing6(&payload);
        let mut frames = Vec::new();
        build(&[0; 14], &d, 1500, 60, &mut frames).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames.len(), frames_for(payload.len(), 1500, d.src_ip));
        let mut data = Vec::new();
        for (n, f) in frames.iter().enumerate() {
            let ip = &f[14..];
            assert_eq!(ip[6], 44, "next header is the fragment header");
            let len = IP6_LEN + usize::from(u16::from_be_bytes([ip[4], ip[5]]));
            assert!(len <= 1500);
            let fragment = &ip[IP6_LEN..IP6_LEN + FRAG6_LEN];
            assert_eq!(fragment[0], PROTO_UDP);
            let field = u16::from_be_bytes([fragment[2], fragment[3]]);
            assert_eq!(usize::from(field >> 3) * 8, data.len(), "offset");
            assert_eq!(field & 1 == 1, n + 1 < frames.len(), "more fragments");
            assert_eq!(
                u32::from_be_bytes([fragment[4], fragment[5], fragment[6], fragment[7]]),
                0xA1B2_C3D4,
                "same identification"
            );
            assert_eq!(parse(ip), Err(RxError::Fragment));
            data.extend_from_slice(&ip[IP6_LEN + FRAG6_LEN..len]);
        }
        assert_eq!(data.len(), UDP_LEN + payload.len());
        // The reassembled datagram verifies as one UDP datagram.
        let mut whole = frames[0][14..14 + IP6_LEN].to_vec();
        whole[4..6].copy_from_slice(&(data.len() as u16).to_be_bytes());
        whole[6] = PROTO_UDP;
        whole.extend_from_slice(&data);
        assert_eq!(parse(&whole).unwrap().payload, &payload[..]);
    }

    #[test]
    fn the_largest_ipv6_datagram_fits_and_one_more_byte_does_not() {
        let max = 65535 - UDP_LEN;
        let mut frames = Vec::new();
        build(&[0; 14], &outgoing6(&vec![1; max]), 1500, 60, &mut frames).unwrap();
        assert_eq!(frames.len(), frames_for(max, 1500, outgoing6(&[]).src_ip));
        assert_eq!(
            build(
                &[0; 14],
                &outgoing6(&vec![1; max + 1]),
                1500,
                60,
                &mut frames
            ),
            Err(TooLong)
        );
    }

    #[test]
    fn malformed_ipv6_packets_are_named() {
        let frame = one_frame6(b"x");
        let ip = &frame[14..];
        assert_eq!(parse(&ip[..30]), Err(RxError::Malformed));
        assert_eq!(parse(&[]), Err(RxError::Malformed));
        // A payload length beyond the frame.
        let mut long = ip.to_vec();
        long[4..6].copy_from_slice(&1000u16.to_be_bytes());
        assert_eq!(parse(&long), Err(RxError::Malformed));
        // TCP, or a hop by hop header, in front of the datagram.
        let mut tcp = ip.to_vec();
        tcp[6] = 6;
        assert_eq!(parse(&tcp), Err(RxError::NotUdp));
        tcp[6] = 0;
        assert_eq!(parse(&tcp), Err(RxError::NotUdp));
        // A UDP length that contradicts the payload length.
        let mut udp_len = ip.to_vec();
        udp_len[IP6_LEN + 4..IP6_LEN + 6].copy_from_slice(&200u16.to_be_bytes());
        assert_eq!(parse(&udp_len), Err(RxError::Malformed));
    }
}
