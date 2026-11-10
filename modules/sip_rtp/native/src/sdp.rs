// SPDX-License-Identifier: GPL-3.0-only
//! Session descriptions of RFC 8866 with the offer and answer rules of RFC 3264.
//!
//! The module writes one audio stream per description, and it reads the
//! description of the other side to find the audio port and to pick a codec.

use crate::codec::{Codec, TELEPHONE_EVENT, TELEPHONE_EVENT_PT};

/// What the module reads out of a description.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Sdp {
    /// The port of the audio media line.
    pub port: u16,
    /// The payload types of the media line, in order.
    pub payloads: Vec<u8>,
    /// The `rtpmap` of each payload type that has one.
    pub rtpmaps: Vec<(u8, String)>,
    /// The `a=ptime` value, when present.
    pub ptime: Option<u32>,
}

/// A payload type of an offer that names a codec of this module.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Offered {
    /// The codec.
    pub codec: &'static Codec,
    /// The payload type that the offer uses for it.
    pub pt: u8,
}

impl Sdp {
    /// Reads the audio media line, the `rtpmap` lines, and `a=ptime`.
    pub(crate) fn parse(body: &str) -> Result<Sdp, String> {
        let mut sdp = Sdp::default();
        let mut have_media = false;
        for line in body.split("\r\n").filter(|l| !l.is_empty()) {
            if let Some(rest) = line.strip_prefix("m=") {
                if have_media {
                    // Only the first media description matters to this module.
                    break;
                }
                let mut f = rest.split(' ');
                if f.next() != Some("audio") {
                    return Err(format!("the first media line is not audio: 'm={rest}'"));
                }
                sdp.port = f
                    .next()
                    .and_then(|p| p.split('/').next())
                    .and_then(|p| p.parse().ok())
                    .ok_or_else(|| format!("the media line has no valid port: 'm={rest}'"))?;
                if f.next() != Some("RTP/AVP") {
                    return Err(format!("the media line is not RTP/AVP: 'm={rest}'"));
                }
                for pt in f {
                    sdp.payloads.push(
                        pt.parse()
                            .ok()
                            .filter(|p| *p < 128)
                            .ok_or_else(|| format!("bad payload type '{pt}' in 'm={rest}'"))?,
                    );
                }
                have_media = true;
            } else if let Some(rest) = line.strip_prefix("a=rtpmap:") {
                let (pt, map) = rest
                    .split_once(' ')
                    .ok_or_else(|| format!("bad rtpmap '{line}'"))?;
                let pt: u8 = pt.parse().map_err(|_| format!("bad rtpmap '{line}'"))?;
                sdp.rtpmaps.push((pt, map.to_string()));
            } else if let Some(rest) = line.strip_prefix("a=ptime:") {
                sdp.ptime = Some(rest.parse().map_err(|_| format!("bad ptime '{line}'"))?);
            }
        }
        if !have_media {
            return Err("the session description has no audio media line".into());
        }
        if sdp.payloads.is_empty() {
            return Err("the audio media line lists no payload type".into());
        }
        Ok(sdp)
    }

    /// Names the codec that a payload type of this description stands for.
    fn codec_of(&self, pt: u8) -> Option<&'static Codec> {
        if let Some((_, map)) = self.rtpmaps.iter().find(|(p, _)| *p == pt) {
            let name = map.split('/').next().unwrap_or("");
            return Codec::by_name(name);
        }
        Codec::by_static_pt(pt)
    }

    /// Returns true when the payload type is `telephone-event`.
    pub(crate) fn is_telephone_event(&self, pt: u8) -> bool {
        self.rtpmaps
            .iter()
            .any(|(p, m)| *p == pt && m.to_ascii_lowercase().starts_with("telephone-event/"))
    }

    /// Returns the codecs of this description that the module knows, in the
    /// order of the media line. `telephone-event` and unknown codecs stay out.
    pub(crate) fn offered(&self) -> Vec<Offered> {
        self.payloads
            .iter()
            .filter_map(|pt| self.codec_of(*pt).map(|codec| Offered { codec, pt: *pt }))
            .collect()
    }

    /// Returns the `telephone-event` payload type, when the description has one.
    pub(crate) fn telephone_event(&self) -> Option<u8> {
        self.payloads
            .iter()
            .copied()
            .find(|pt| self.is_telephone_event(*pt))
    }
}

/// Writes a description with one audio stream.
///
/// `lines` lists the codecs in order of preference, `event` the payload type
/// of `telephone-event` when it is offered, and `session` the numeric
/// session id of the origin line.
pub(crate) fn build(
    user: &str,
    ip: &str,
    port: u16,
    lines: &[Offered],
    event: Option<u8>,
    ptime: u32,
    session: u32,
) -> String {
    let mut pts: Vec<String> = lines.iter().map(|l| l.pt.to_string()).collect();
    if let Some(pt) = event {
        pts.push(pt.to_string());
    }
    let mut s = String::new();
    s.push_str("v=0\r\n");
    s.push_str(&format!("o={user} {session} {session} IN IP4 {ip}\r\n"));
    s.push_str("s=Talk\r\n");
    s.push_str(&format!("c=IN IP4 {ip}\r\n"));
    s.push_str("t=0 0\r\n");
    s.push_str(&format!("m=audio {port} RTP/AVP {}\r\n", pts.join(" ")));
    for l in lines {
        s.push_str(&format!("a=rtpmap:{} {}\r\n", l.pt, l.codec.rtpmap));
        if let Some(f) = l.codec.fmtp {
            s.push_str(&format!("a=fmtp:{} {f}\r\n", l.pt));
        }
    }
    if let Some(pt) = event {
        s.push_str(&format!("a=rtpmap:{pt} {TELEPHONE_EVENT}\r\n"));
        s.push_str(&format!("a=fmtp:{pt} 0-16\r\n"));
    }
    s.push_str(&format!("a=ptime:{ptime}\r\n"));
    s.push_str("a=sendrecv\r\n");
    s
}

/// The payload type that the module gives `telephone-event` in an offer.
pub(crate) const fn event_pt() -> u8 {
    TELEPHONE_EVENT_PT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::CODECS;

    fn lines(names: &[&str]) -> Vec<Offered> {
        names
            .iter()
            .map(|n| {
                let codec = Codec::by_name(n).unwrap();
                Offered {
                    codec,
                    pt: codec.pt,
                }
            })
            .collect()
    }

    #[test]
    fn an_offer_round_trips_through_the_parser() {
        let text = build(
            "1002",
            "192.0.2.10",
            16384,
            &lines(&["PCMU", "PCMA"]),
            Some(event_pt()),
            20,
            4242,
        );
        assert!(text.starts_with("v=0\r\no=1002 4242 4242 IN IP4 192.0.2.10\r\n"));
        assert!(text.ends_with("a=ptime:20\r\na=sendrecv\r\n"));
        let sdp = Sdp::parse(&text).unwrap();
        assert_eq!(sdp.port, 16384);
        assert_eq!(sdp.payloads, [0, 8, 101]);
        assert_eq!(sdp.ptime, Some(20));
        let names: Vec<_> = sdp.offered().iter().map(|o| o.codec.name).collect();
        assert_eq!(names, ["PCMU", "PCMA"]);
        assert_eq!(sdp.telephone_event(), Some(101));
    }

    #[test]
    fn dynamic_payload_types_resolve_through_the_rtpmap() {
        let text = build(
            "u",
            "192.0.2.1",
            4000,
            &[Offered {
                codec: &CODECS[4],
                pt: 109,
            }],
            None,
            20,
            1,
        );
        let sdp = Sdp::parse(&text).unwrap();
        let o = sdp.offered();
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].codec.name, o[0].pt), ("opus", 109));
        assert_eq!(sdp.telephone_event(), None);
    }

    #[test]
    fn a_description_without_audio_is_refused() {
        assert!(Sdp::parse("v=0\r\ns=x\r\n").is_err());
        assert!(Sdp::parse("v=0\r\nm=video 4000 RTP/AVP 96\r\n").is_err());
        assert!(Sdp::parse("v=0\r\nm=audio x RTP/AVP 0\r\n").is_err());
        assert!(Sdp::parse("v=0\r\nm=audio 4000 RTP/SAVP 0\r\n").is_err());
        assert!(Sdp::parse("v=0\r\nm=audio 4000 RTP/AVP\r\n").is_err());
        assert!(Sdp::parse("v=0\r\nm=audio 4000 RTP/AVP 300\r\n").is_err());
    }
}
