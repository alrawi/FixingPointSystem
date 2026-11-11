// SPDX-License-Identifier: GPL-3.0-only
//! Synthetic MPEG-TS segments (ISO/IEC 13818-1).
//!
//! A segment is a whole number of 188 octet packets. It opens with a PAT and a
//! PMT that name one H.264 video stream and one AAC audio stream, and repeats
//! both every 100 packets. The rest is a repeating cycle of one video access
//! unit of ten packets, with a PCR in the adaptation field of its first
//! packet, and one audio frame. Every elementary stream payload is random
//! bytes behind a valid PES header, so the segment parses as transport stream
//! and carries no third party media.

use rand::RngCore;

/// The size of one transport stream packet.
pub const PACKET: usize = 188;

/// The largest segment that the module writes, in octets.
pub const MAX_SEGMENT: u64 = 16 << 20;

const PMT_PID: u16 = 0x1000;
const VIDEO_PID: u16 = 0x0100;
const AUDIO_PID: u16 = 0x0101;
/// Video packets for each audio packet in the cycle.
const VIDEO_RUN: usize = 10;
/// A PAT and a PMT follow every this many packets.
const REPEAT: usize = 100;

/// The number of packets of a segment of `segment_ms` milliseconds at `bits_per_second`.
pub fn packets(bits_per_second: u64, segment_ms: u64) -> u64 {
    let bytes = bits_per_second * segment_ms / 8000;
    ((bytes + PACKET as u64 / 2) / PACKET as u64).max(3)
}

/// The size of a segment in octets.
pub fn size(bits_per_second: u64, segment_ms: u64) -> Result<u64, String> {
    let n = packets(bits_per_second, segment_ms) * PACKET as u64;
    if n > MAX_SEGMENT {
        return Err(format!(
            "a segment of {n} octets is larger than the limit of {MAX_SEGMENT}"
        ));
    }
    Ok(n)
}

/// CRC-32/MPEG-2, which closes every PSI section.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn section(table_id: u8, body: &[u8]) -> Vec<u8> {
    // section_length counts the bytes after itself, which are the body and the CRC.
    let len = body.len() + 4;
    let mut s = vec![table_id, 0xB0 | ((len >> 8) as u8 & 0x0F), len as u8];
    s.extend_from_slice(body);
    let crc = crc32(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

/// The PAT section, which names program 1 at the PMT PID.
pub fn pat() -> Vec<u8> {
    let mut b = vec![0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01];
    b.extend_from_slice(&(0xE000 | PMT_PID).to_be_bytes());
    section(0x00, &b)
}

/// The PMT section, which names the video and the audio stream.
pub fn pmt() -> Vec<u8> {
    let mut b = vec![0x00, 0x01, 0xC1, 0x00, 0x00];
    b.extend_from_slice(&(0xE000 | VIDEO_PID).to_be_bytes());
    b.extend_from_slice(&[0xF0, 0x00]);
    for (stream_type, pid) in [(0x1B, VIDEO_PID), (0x0F, AUDIO_PID)] {
        b.push(stream_type);
        b.extend_from_slice(&(0xE000 | pid).to_be_bytes());
        b.extend_from_slice(&[0xF0, 0x00]);
    }
    section(0x02, &b)
}

/// The five octets of a PTS with the marker bits of a PES header.
fn pts_field(pts: u64) -> [u8; 5] {
    let pts = pts & 0x1_FFFF_FFFF;
    [
        0x20 | (((pts >> 30) as u8 & 0x07) << 1) | 1,
        (pts >> 22) as u8,
        (((pts >> 15) as u8 & 0x7F) << 1) | 1,
        (pts >> 7) as u8,
        (((pts as u8) & 0x7F) << 1) | 1,
    ]
}

struct Writer<'a> {
    out: &'a mut Vec<u8>,
    counters: [u8; 3],
}

impl Writer<'_> {
    fn counter(&mut self, pid: u16) -> u8 {
        let slot = match pid {
            VIDEO_PID => 1,
            AUDIO_PID => 2,
            _ => 0,
        };
        let c = self.counters[slot];
        self.counters[slot] = (c + 1) & 0x0F;
        c
    }

    fn header(&mut self, pid: u16, start: bool, adaptation: u8) {
        let cc = self.counter(pid);
        self.out.push(0x47);
        self.out
            .push((u8::from(start) << 6) | ((pid >> 8) as u8 & 0x1F));
        self.out.push(pid as u8);
        self.out.push((adaptation << 4) | cc);
    }

    /// A packet that carries a PSI section behind a zero pointer field.
    fn psi(&mut self, pid: u16, section: &[u8]) {
        self.header(pid, true, 0b01);
        self.out.push(0);
        self.out.extend_from_slice(section);
        let used = 1 + section.len();
        self.out.resize(self.out.len() + (PACKET - 4 - used), 0xFF);
    }

    /// The first packet of a video access unit, with a PCR and a PES header.
    fn video_start(&mut self, pts: u64, rng: &mut dyn RngCore) {
        self.header(VIDEO_PID, true, 0b11);
        // Adaptation field of 7 octets after its length: flags and a 6 octet PCR.
        let pcr = pts.saturating_sub(9000) & 0x1_FFFF_FFFF;
        self.out.push(7);
        self.out.push(0x10);
        self.out.push((pcr >> 25) as u8);
        self.out.push((pcr >> 17) as u8);
        self.out.push((pcr >> 9) as u8);
        self.out.push((pcr >> 1) as u8);
        self.out.push((((pcr & 1) as u8) << 7) | 0x7E);
        self.out.push(0);
        // PES header: start code, stream id, unbounded length, flags, header length, PTS.
        self.out
            .extend_from_slice(&[0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x80, 0x05]);
        self.out.extend_from_slice(&pts_field(pts));
        // An access unit delimiter opens the NAL units.
        self.out
            .extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x09, 0xF0]);
        let used = 8 + 14 + 6;
        self.fill(PACKET - 4 - used, rng);
    }

    fn payload_packet(&mut self, pid: u16, rng: &mut dyn RngCore) {
        self.header(pid, false, 0b01);
        self.fill(PACKET - 4, rng);
    }

    /// One audio frame in one packet, with an ADTS header.
    fn audio(&mut self, pts: u64, rng: &mut dyn RngCore) {
        self.header(AUDIO_PID, true, 0b01);
        let room = PACKET - 4;
        // PES_packet_length counts the octets after the length field.
        let len = (room - 6) as u16;
        self.out.extend_from_slice(&[0x00, 0x00, 0x01, 0xC0]);
        self.out.extend_from_slice(&len.to_be_bytes());
        self.out.extend_from_slice(&[0x80, 0x80, 0x05]);
        self.out.extend_from_slice(&pts_field(pts));
        self.out.extend_from_slice(&[0xFF, 0xF1, 0x50, 0x80]);
        self.fill(room - 14 - 4, rng);
    }

    fn fill(&mut self, n: usize, rng: &mut dyn RngCore) {
        let at = self.out.len();
        self.out.resize(at + n, 0);
        rng.fill_bytes(&mut self.out[at..]);
    }
}

/// Appends the segment `index` of a stream with `packets` packets to `out`. The presentation
/// times continue from segment to segment at `segment_ms` per segment.
pub fn segment(
    index: u64,
    segment_ms: u64,
    packets: u64,
    rng: &mut dyn RngCore,
    out: &mut Vec<u8>,
) {
    let (pat, pmt) = (pat(), pmt());
    let first_pts = 90_000 + index * segment_ms * 90;
    let mut w = Writer {
        out,
        counters: [0; 3],
    };
    // Spread the access units of the segment over its length at 30 frames per second.
    let mut frame = 0u64;
    let mut run = 0usize;
    for p in 0..packets as usize {
        match p % REPEAT {
            0 => w.psi(0, &pat),
            1 => w.psi(PMT_PID, &pmt),
            _ => {
                let pts = first_pts + frame * 3000;
                if run == VIDEO_RUN {
                    w.audio(pts, rng);
                    run = 0;
                } else if run == 0 {
                    w.video_start(pts, rng);
                    frame += 1;
                    run = 1;
                } else {
                    w.payload_packet(VIDEO_PID, rng);
                    run += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> rand_pcg::Pcg64Mcg {
        rand_pcg::Pcg64Mcg::seed_from_u64(7)
    }

    #[test]
    fn crc_matches_the_known_pat() {
        // The PAT of one program with its PMT at PID 0x1000, which appears in every
        // reference stream: the CRC is 2A B1 04 B2.
        let pat = [
            0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00,
        ];
        assert_eq!(crc32(&pat), 0x2AB1_04B2);
        assert_eq!(super::pat()[12..], [0x2A, 0xB1, 0x04, 0xB2]);
    }

    #[test]
    fn every_section_checks_out() {
        for s in [pat(), pmt()] {
            assert_eq!(crc32(&s), 0, "a section with its CRC divides to zero");
            let len = (usize::from(s[1] & 0x0F) << 8) | usize::from(s[2]);
            assert_eq!(s.len(), 3 + len);
        }
    }

    #[test]
    fn size_follows_the_bit_rate() {
        assert_eq!(packets(800_000, 6000), 3191);
        assert_eq!(size(800_000, 6000).unwrap(), 3191 * 188);
        assert_eq!(packets(8_000, 100), 3);
        assert!(size(200_000_000, 60_000).is_err());
    }

    #[test]
    fn a_segment_is_whole_packets_with_valid_framing() {
        let mut out = Vec::new();
        segment(3, 2000, 250, &mut rng(), &mut out);
        assert_eq!(out.len(), 250 * PACKET);
        let mut next = [0u8; 3];
        let mut pids = std::collections::BTreeMap::new();
        for (i, p) in out.chunks(PACKET).enumerate() {
            assert_eq!(p[0], 0x47, "sync byte of packet {i}");
            let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
            *pids.entry(pid).or_insert(0) += 1;
            let slot = match pid {
                VIDEO_PID => 1,
                AUDIO_PID => 2,
                _ => 0,
            };
            if slot != 0 {
                assert_eq!(p[3] & 0x0F, next[slot], "continuity of pid {pid} at {i}");
                next[slot] = (next[slot] + 1) & 0x0F;
            }
        }
        assert_eq!(pids[&0], 3, "PAT at 0, 100, and 200");
        assert_eq!(pids[&PMT_PID], 3);
        assert!(pids[&VIDEO_PID] > pids[&AUDIO_PID] * 8);
        assert_eq!(&out[4..5], &[0], "pointer field of the PAT");
    }

    #[test]
    fn same_seed_same_bytes_and_other_index_other_times() {
        let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
        segment(1, 2000, 50, &mut rng(), &mut a);
        segment(1, 2000, 50, &mut rng(), &mut b);
        segment(2, 2000, 50, &mut rng(), &mut c);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn pts_field_round_trips() {
        let f = pts_field(0x1_2345_6789);
        let back = (u64::from(f[0] >> 1 & 7) << 30)
            | (u64::from(f[1]) << 22)
            | (u64::from(f[2] >> 1) << 15)
            | (u64::from(f[3]) << 7)
            | u64::from(f[4] >> 1);
        assert_eq!(back, 0x1_2345_6789);
        assert_eq!(f[0] & 0xF1, 0x21);
    }
}
