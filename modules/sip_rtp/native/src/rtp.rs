// SPDX-License-Identifier: GPL-3.0-only
//! RTP packets of RFC 3550 for one audio stream.

use rand::RngCore;

use crate::codec::Codec;

/// The fixed header of an RTP packet in bytes, with no CSRC list and no extension.
#[cfg(test)]
pub(crate) const HEADER_LEN: usize = 12;

/// The state of the stream that one flow side sends.
///
/// A real sender picks the sequence number, the timestamp, and the
/// synchronization source at random when the stream starts, and then counts
/// up by one packet and one packet time per packet (RFC 3550 section 5.1).
#[derive(Clone, Debug)]
pub(crate) struct Stream {
    ssrc: u32,
    seq: u16,
    timestamp: u32,
    first: bool,
}

impl Stream {
    /// Starts a stream with values from the session generator.
    pub(crate) fn start(rng: &mut dyn RngCore) -> Stream {
        Stream {
            ssrc: rng.next_u32(),
            seq: (rng.next_u32() & 0xFFFF) as u16,
            timestamp: rng.next_u32(),
            first: true,
        }
    }

    /// Appends the next packet to `out` and advances the stream.
    ///
    /// The first packet carries the marker bit, which starts a talk spurt.
    /// The payload is random, as the output of a speech codec looks.
    pub(crate) fn next(
        &mut self,
        codec: &Codec,
        pt: u8,
        ptime: u32,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) {
        out.push(0x80);
        out.push(pt | if self.first { 0x80 } else { 0 });
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.ssrc.to_be_bytes());
        let n = codec.payload_len(ptime, rng);
        let at = out.len();
        out.resize(at + n, 0);
        if let Some(payload) = out.get_mut(at..) {
            rng.fill_bytes(payload);
            // An Opus packet starts with a table of contents byte, and dissectors reject a
            // random one (RFC 6716, section 3.1). Config 8 to 11 is SILK wideband with
            // 10, 20, 40, and 60 ms, and code 0 is one frame that fills the rest.
            if codec.name == "opus" {
                if let Some(toc) = payload.first_mut() {
                    let config: u8 = match ptime {
                        10 => 8,
                        40 => 10,
                        60 => 11,
                        _ => 9,
                    };
                    *toc = config << 3;
                }
            }
        }
        self.first = false;
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(codec.ts_step(ptime));
    }
}

/// The fields of an RTP header that the tests and checks read.
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

/// Parses the fixed header of a packet.
#[cfg(test)]
pub(crate) fn parse(packet: &[u8]) -> Option<Header> {
    if packet.len() < HEADER_LEN || packet[0] >> 6 != 2 {
        return None;
    }
    Some(Header {
        marker: packet[1] & 0x80 != 0,
        pt: packet[1] & 0x7F,
        seq: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::CODECS;

    #[test]
    fn a_stream_counts_up_and_marks_only_its_first_packet() {
        let mut rng = rand_pcg::Pcg64Mcg::new(7);
        let mut s = Stream::start(&mut rng);
        let mut seen = Vec::new();
        for _ in 0..5 {
            let mut p = Vec::new();
            s.next(&CODECS[0], 0, 20, &mut rng, &mut p);
            assert_eq!(p.len(), HEADER_LEN + 160);
            seen.push(parse(&p).unwrap());
        }
        assert!(seen[0].marker);
        assert!(seen[1..].iter().all(|h| !h.marker));
        for w in seen.windows(2) {
            assert_eq!(w[1].seq, w[0].seq.wrapping_add(1));
            assert_eq!(w[1].timestamp, w[0].timestamp.wrapping_add(160));
            assert_eq!(w[1].ssrc, w[0].ssrc);
            assert_eq!(w[1].pt, 0);
        }
    }

    #[test]
    fn counters_wrap_around() {
        let mut rng = rand_pcg::Pcg64Mcg::new(7);
        let mut s = Stream {
            ssrc: 1,
            seq: u16::MAX,
            timestamp: u32::MAX - 100,
            first: false,
        };
        let mut a = Vec::new();
        s.next(&CODECS[4], 111, 20, &mut rng, &mut a);
        let mut b = Vec::new();
        s.next(&CODECS[4], 111, 20, &mut rng, &mut b);
        let (a, b) = (parse(&a).unwrap(), parse(&b).unwrap());
        assert_eq!((a.seq, b.seq), (u16::MAX, 0));
        assert_eq!(b.timestamp, a.timestamp.wrapping_add(960));
    }

    #[test]
    fn a_short_packet_or_a_wrong_version_does_not_parse() {
        assert!(parse(&[0x80; 11]).is_none());
        assert!(parse(&[0x40; 12]).is_none());
    }
}
