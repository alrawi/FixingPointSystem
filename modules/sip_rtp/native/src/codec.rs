// SPDX-License-Identifier: GPL-3.0-only
//! The audio codecs that the module offers, answers, and streams.

use rand::RngCore;

/// The payload type that the module gives to `telephone-event`, which every
/// offer lists so that the SDP looks like the one of a desk phone.
pub(crate) const TELEPHONE_EVENT_PT: u8 = 101;

/// The `rtpmap` of `telephone-event`.
pub(crate) const TELEPHONE_EVENT: &str = "telephone-event/8000";

/// How many payload bytes one packet carries.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Payload {
    /// A constant bit rate: this many bytes for every millisecond of audio.
    PerMs(usize),
    /// A variable bit rate: between `lo` and `hi` bytes for 20 ms of audio, scaled with the
    /// packet time.
    Vbr { lo: usize, hi: usize },
}

/// One audio codec.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Codec {
    /// The name that scenarios use.
    pub name: &'static str,
    /// The payload type of an offer that the module writes.
    pub pt: u8,
    /// The `rtpmap` value, `name/clock` and the channel count when it is not one.
    pub rtpmap: &'static str,
    /// The RTP timestamp clock in hertz.
    pub clock: u32,
    /// The `fmtp` parameters, when the codec has any.
    pub fmtp: Option<&'static str>,
    /// The payload size rule.
    pub payload: Payload,
}

/// The codecs in the order that the module lists them in an offer.
pub(crate) const CODECS: [Codec; 5] = [
    Codec {
        name: "PCMU",
        pt: 0,
        rtpmap: "PCMU/8000",
        clock: 8000,
        fmtp: None,
        payload: Payload::PerMs(8),
    },
    Codec {
        name: "PCMA",
        pt: 8,
        rtpmap: "PCMA/8000",
        clock: 8000,
        fmtp: None,
        payload: Payload::PerMs(8),
    },
    // RFC 3551 section 4.5.2 gives G.722 the RTP clock 8000 although it samples at 16 kHz.
    Codec {
        name: "G722",
        pt: 9,
        rtpmap: "G722/8000",
        clock: 8000,
        fmtp: None,
        payload: Payload::PerMs(8),
    },
    // Ten bytes for every ten milliseconds.
    Codec {
        name: "G729",
        pt: 18,
        rtpmap: "G729/8000",
        clock: 8000,
        fmtp: Some("annexb=no"),
        payload: Payload::PerMs(1),
    },
    Codec {
        name: "opus",
        pt: 111,
        rtpmap: "opus/48000/2",
        clock: 48000,
        fmtp: Some("minptime=10;useinbandfec=1"),
        payload: Payload::Vbr { lo: 40, hi: 110 },
    },
];

impl Codec {
    /// Looks a codec up by name, ignoring case as SDP does.
    pub(crate) fn by_name(name: &str) -> Option<&'static Codec> {
        CODECS.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Looks a codec up by its static payload type of RFC 3551.
    pub(crate) fn by_static_pt(pt: u8) -> Option<&'static Codec> {
        CODECS.iter().find(|c| c.pt == pt && c.pt < 96)
    }

    /// Returns the RTP timestamp step of one packet of `ptime` milliseconds.
    pub(crate) fn ts_step(&self, ptime: u32) -> u32 {
        // The clock is at most 48000, so the product stays far below 2^32.
        self.clock / 1000 * ptime
    }

    /// Returns the payload size of one packet of `ptime` milliseconds.
    pub(crate) fn payload_len(&self, ptime: u32, rng: &mut dyn RngCore) -> usize {
        match self.payload {
            Payload::PerMs(n) => n * ptime as usize,
            Payload::Vbr { lo, hi } => {
                let span = (hi - lo + 1) as u64;
                let n = lo + (rng.next_u64() % span) as usize;
                n * ptime as usize / 20
            }
        }
    }
}

/// Splits a comma separated codec list into known codecs, in order.
pub(crate) fn parse_list(function: &str, list: &str) -> Result<Vec<&'static Codec>, String> {
    let mut out: Vec<&'static Codec> = Vec::new();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let codec = Codec::by_name(name).ok_or_else(|| {
            format!("'{name}' in '{function}' is not a codec of this module (PCMU, PCMA, G722, G729, opus)")
        })?;
        if !out.iter().any(|c| c.name == codec.name) {
            out.push(codec);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_sizes_follow_the_bit_rate() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        assert_eq!(CODECS[0].payload_len(20, &mut rng), 160);
        assert_eq!(CODECS[1].payload_len(30, &mut rng), 240);
        assert_eq!(CODECS[2].payload_len(20, &mut rng), 160);
        assert_eq!(CODECS[3].payload_len(20, &mut rng), 20);
        for _ in 0..200 {
            let n = CODECS[4].payload_len(20, &mut rng);
            assert!((40..=110).contains(&n), "{n}");
        }
    }

    #[test]
    fn timestamps_advance_by_the_clock_per_packet() {
        assert_eq!(CODECS[0].ts_step(20), 160);
        assert_eq!(CODECS[4].ts_step(20), 960);
        assert_eq!(CODECS[3].ts_step(10), 80);
    }

    #[test]
    fn names_resolve_without_regard_to_case() {
        assert_eq!(Codec::by_name("pcmu").map(|c| c.pt), Some(0));
        assert_eq!(Codec::by_name("OPUS").map(|c| c.pt), Some(111));
        assert!(Codec::by_name("speex").is_none());
        assert_eq!(Codec::by_static_pt(8).map(|c| c.name), Some("PCMA"));
        assert!(Codec::by_static_pt(111).is_none());
    }

    #[test]
    fn a_list_keeps_order_and_drops_repeats() {
        let l = parse_list("invite", "opus, PCMU,opus").unwrap();
        let names: Vec<_> = l.iter().map(|c| c.name).collect();
        assert_eq!(names, ["opus", "PCMU"]);
        assert!(parse_list("invite", "PCMU,speex").is_err());
    }
}
