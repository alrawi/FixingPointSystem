// SPDX-License-Identifier: GPL-3.0-only
//! The NTP packet of RFC 5905 and its timestamps.
//!
//! A packet has a header of 48 bytes. A client request and a server reply use
//! only this header, so the codec reads and writes nothing else. It accepts
//! longer packets on input, because extension fields and a message
//! authentication code follow the header, and ignores what follows.
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +---+-----+-----+---------------+---------------+---------------+
//! | LI| VN  |Mode |    Stratum    |     Poll      |   Precision   |
//! +---+-----+-----+---------------+---------------+---------------+
//! |                          Root Delay                           |
//! |                       Root Dispersion                         |
//! |                     Reference ID (4 bytes)                    |
//! |         Reference, Origin, Receive, Transmit timestamps       |
//! |                     (4 times 8 bytes)                         |
//! ```

use std::fmt;
use std::net::Ipv4Addr;

/// The length of the header, which is the length of a packet without
/// extension fields.
pub const HEADER_LEN: usize = 48;

/// Seconds between the NTP epoch, 1900-01-01, and the Unix epoch, 1970-01-01.
pub const NTP_UNIX_OFFSET: i64 = 2_208_988_800;

/// The mode of a symmetric active peer.
pub const MODE_SYMMETRIC_ACTIVE: u8 = 1;
/// The mode of a symmetric passive peer.
pub const MODE_SYMMETRIC_PASSIVE: u8 = 2;
/// The mode of a client.
pub const MODE_CLIENT: u8 = 3;
/// The mode of a server.
pub const MODE_SERVER: u8 = 4;

/// The leap indicator of a server that has no synchronization, which a
/// Kiss o' Death packet also carries.
pub const LEAP_UNSYNCHRONIZED: u8 = 3;

/// The failures of [`Packet::parse`] and of the timestamp conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The message is shorter than the header.
    TooShort {
        /// The length of the message.
        len: usize,
    },
    /// The version lies outside 1 to 4.
    BadVersion {
        /// The version field.
        version: u8,
    },
    /// The instant does not fit the 32 bit seconds of NTP era 0, which ends
    /// in February 2036.
    OutOfEra {
        /// The instant in milliseconds since the Unix epoch.
        unix_ms: i64,
    },
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::TooShort { len } => {
                write!(f, "a packet has at least {HEADER_LEN} bytes, got {len}")
            }
            WireError::BadVersion { version } => {
                write!(f, "the version {version} lies outside 1 to 4")
            }
            WireError::OutOfEra { unix_ms } => write!(
                f,
                "the instant {unix_ms} ms does not lie in NTP era 0, 1900 to 2036"
            ),
        }
    }
}

impl std::error::Error for WireError {}

/// Converts an instant to a 64 bit NTP timestamp, whole seconds since 1900 in
/// the high half and a fraction of 2^-32 seconds in the low half.
///
/// `sub_ms` places the instant inside its millisecond, from 0 for the start
/// to 1 000 000 for the end in millionths of a millisecond. The clock of the
/// engine ticks in milliseconds, but a real clock does not, so the generator
/// supplies the low bits that the clock lacks.
pub fn timestamp(unix_ms: i64, sub_ms: u32) -> Result<u64, WireError> {
    let secs = unix_ms.div_euclid(1000) + NTP_UNIX_OFFSET;
    let secs = u32::try_from(secs).map_err(|_| WireError::OutOfEra { unix_ms })?;
    // Nanoseconds inside the second. Rounding the fraction up keeps
    // `unix_ms` from reading the start of a millisecond as the one before.
    let nanos = (unix_ms.rem_euclid(1000) as u64) * 1_000_000 + u64::from(sub_ms.min(999_999));
    let fraction = (nanos << 32).div_ceil(1_000_000_000);
    Ok((u64::from(secs) << 32) | fraction)
}

/// Returns the instant of a timestamp in milliseconds since the Unix epoch,
/// rounded down, or `None` for the zero timestamp that means unset.
pub fn unix_ms(timestamp: u64) -> Option<i64> {
    if timestamp == 0 {
        return None;
    }
    let secs = (timestamp >> 32) as i64 - NTP_UNIX_OFFSET;
    let ms = ((timestamp & 0xffff_ffff) * 1000) >> 32;
    Some(secs * 1000 + ms as i64)
}

/// Converts a duration in microseconds to 2^-32 second units.
pub fn micros_to_fraction(micros: u64) -> u64 {
    (micros << 32) / 1_000_000
}

/// Converts microseconds to the 16.16 fixed point seconds of the root delay
/// and the root dispersion, rounded down. The largest value is 65 535 seconds.
pub fn short_from_micros(micros: u64) -> u32 {
    u32::try_from((micros << 16) / 1_000_000).unwrap_or(u32::MAX)
}

/// Converts the 16.16 fixed point seconds of the root delay and the root
/// dispersion to milliseconds, rounded down.
pub fn short_to_ms(short: u32) -> u32 {
    ((u64::from(short) * 1000) >> 16) as u32
}

/// The 48 byte header of an NTP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    /// The leap indicator, 2 bits.
    pub leap: u8,
    /// The version, 3 bits.
    pub version: u8,
    /// The mode, 3 bits.
    pub mode: u8,
    /// The stratum. 0 is unspecified, which a Kiss o' Death packet uses, 1 is
    /// a primary server, and 2 to 15 are secondary servers.
    pub stratum: u8,
    /// The poll exponent, log2 of the interval in seconds.
    pub poll: i8,
    /// The precision exponent, log2 of the clock tick in seconds.
    pub precision: i8,
    /// The round trip delay to the primary source in 16.16 fixed point seconds.
    pub root_delay: u32,
    /// The error to the primary source in 16.16 fixed point seconds.
    pub root_dispersion: u32,
    /// An ASCII code for stratum 0 and 1, otherwise the IPv4 address of the
    /// synchronization source.
    pub reference_id: [u8; 4],
    /// When the clock was last set.
    pub reference: u64,
    /// The transmit timestamp of the request, which a server copies back.
    pub origin: u64,
    /// When the request arrived.
    pub receive: u64,
    /// When this packet left.
    pub transmit: u64,
}

impl Packet {
    /// Encodes the header.
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0] = (self.leap & 3) << 6 | (self.version & 7) << 3 | (self.mode & 7);
        b[1] = self.stratum;
        b[2] = self.poll as u8;
        b[3] = self.precision as u8;
        b[4..8].copy_from_slice(&self.root_delay.to_be_bytes());
        b[8..12].copy_from_slice(&self.root_dispersion.to_be_bytes());
        b[12..16].copy_from_slice(&self.reference_id);
        b[16..24].copy_from_slice(&self.reference.to_be_bytes());
        b[24..32].copy_from_slice(&self.origin.to_be_bytes());
        b[32..40].copy_from_slice(&self.receive.to_be_bytes());
        b[40..48].copy_from_slice(&self.transmit.to_be_bytes());
        b
    }

    /// Parses the header of a packet and ignores any extension fields and
    /// authentication code behind it.
    pub fn parse(message: &[u8]) -> Result<Packet, WireError> {
        let b = message
            .get(..HEADER_LEN)
            .ok_or(WireError::TooShort { len: message.len() })?;
        let version = (b[0] >> 3) & 7;
        if !(1..=4).contains(&version) {
            return Err(WireError::BadVersion { version });
        }
        let word = |at: usize| u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let stamp = |at: usize| (u64::from(word(at)) << 32) | u64::from(word(at + 4));
        Ok(Packet {
            leap: b[0] >> 6,
            version,
            mode: b[0] & 7,
            stratum: b[1],
            poll: b[2] as i8,
            precision: b[3] as i8,
            root_delay: word(4),
            root_dispersion: word(8),
            reference_id: [b[12], b[13], b[14], b[15]],
            reference: stamp(16),
            origin: stamp(24),
            receive: stamp(32),
            transmit: stamp(40),
        })
    }

    /// Returns the reference identifier as text, which is the ASCII code of a
    /// stratum 0 or 1 packet without its padding, or the dotted address of a
    /// secondary server.
    pub fn reference_text(&self) -> String {
        if self.stratum >= 2 {
            return Ipv4Addr::from(self.reference_id).to_string();
        }
        self.reference_id
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| char::from(*b))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Packet {
        Packet {
            leap: 0,
            version: 4,
            mode: MODE_SERVER,
            stratum: 2,
            poll: 6,
            precision: -23,
            root_delay: short_from_micros(12_000),
            root_dispersion: short_from_micros(7_000),
            reference_id: [192, 0, 2, 10],
            reference: 0xe9a3_4f10_8000_0000,
            origin: 0xe9a3_4f2a_1234_5678,
            receive: 0xe9a3_4f2a_2345_6789,
            transmit: 0xe9a3_4f2a_2346_0000,
        }
    }

    #[test]
    fn encodes_the_header_of_rfc_5905() {
        let b = sample().encode();
        assert_eq!(b.len(), 48);
        // LI 0, VN 4, mode 4.
        assert_eq!(b[0], 0x24);
        assert_eq!(b[1], 2);
        assert_eq!(b[2], 6);
        assert_eq!(b[3], 0xe9);
        assert_eq!(b[12..16], [192, 0, 2, 10]);
        assert_eq!(b[24..32], 0xe9a3_4f2a_1234_5678u64.to_be_bytes());
        assert_eq!(b[40..48], 0xe9a3_4f2a_2346_0000u64.to_be_bytes());
    }

    #[test]
    fn parse_inverts_encode_and_ignores_trailing_bytes() {
        let p = sample();
        assert_eq!(Packet::parse(&p.encode()), Ok(p));
        let mut long = p.encode().to_vec();
        long.extend_from_slice(&[0xaa; 20]);
        assert_eq!(Packet::parse(&long), Ok(p));
        for leap in 0..4 {
            for mode in 0..8 {
                let q = Packet { leap, mode, ..p };
                assert_eq!(Packet::parse(&q.encode()), Ok(q));
            }
        }
    }

    #[test]
    fn parse_rejects_short_packets_and_bad_versions() {
        assert_eq!(
            Packet::parse(&[0u8; 47]),
            Err(WireError::TooShort { len: 47 })
        );
        for version in [0u8, 5, 7] {
            let mut b = sample().encode();
            b[0] = (b[0] & 0xc7) | (version << 3);
            assert_eq!(Packet::parse(&b), Err(WireError::BadVersion { version }));
        }
    }

    #[test]
    fn timestamps_convert_between_the_epochs() {
        // 2026-11-10T12:00:00.250Z.
        let ms = 1_794_312_000_250;
        let t = timestamp(ms, 0).unwrap();
        assert_eq!(t >> 32, 1_794_312_000 + 2_208_988_800);
        // A quarter second is 2^30 units.
        assert_eq!(t & 0xffff_ffff, 1 << 30);
        assert_eq!(unix_ms(t), Some(ms));
        // The sub millisecond part stays inside its millisecond.
        let late = timestamp(ms, 999_999).unwrap();
        assert!(late > t);
        assert_eq!(unix_ms(late), Some(ms));
        assert_eq!(unix_ms(0), None);
    }

    #[test]
    fn timestamps_reject_instants_outside_era_zero() {
        // 1899 and 2037.
        for ms in [-2_209_000_000_000, 2_120_000_000_000] {
            assert_eq!(timestamp(ms, 0), Err(WireError::OutOfEra { unix_ms: ms }));
        }
        assert!(timestamp(-NTP_UNIX_OFFSET * 1000, 0).is_ok());
        assert!(timestamp(2_085_978_495_999, 0).is_ok());
        assert!(timestamp(2_085_978_496_000, 0).is_err());
    }

    #[test]
    fn short_format_and_microseconds_convert() {
        assert_eq!(short_from_micros(1_000_000), 0x0001_0000);
        assert_eq!(short_from_micros(500_000), 0x8000);
        assert_eq!(short_from_micros(u64::MAX >> 17), u32::MAX);
        assert_eq!(short_to_ms(short_from_micros(12_000)), 11, "rounds down");
        assert_eq!(micros_to_fraction(1_000_000), 1 << 32);
        assert_eq!(micros_to_fraction(500_000), 1 << 31);
    }

    #[test]
    fn reference_text_reads_codes_and_addresses() {
        let mut p = sample();
        assert_eq!(p.reference_text(), "192.0.2.10");
        p.stratum = 1;
        p.reference_id = *b"GPS\0";
        assert_eq!(p.reference_text(), "GPS");
        p.stratum = 0;
        p.reference_id = *b"RATE";
        assert_eq!(p.reference_text(), "RATE");
    }
}
