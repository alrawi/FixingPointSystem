// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `ntp` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements `ntp.query`, `ntp.reply`, and `ntp.field`
//! behind the [`TemplateFn`] trait of `fps-flow`, so the interpreter renders
//! `{{ ntp.query(now_ms=now('epoch_ms')) }}` without a WebAssembly runtime.
//! Messages follow RFC 5905, NTP version 4, with the version of the request
//! echoed by the server, so version 3 clients work too.
//!
//! The functions take the instant from their `now_ms` argument and never read
//! a clock themselves. A scenario passes `now('epoch_ms')`, so the engine
//! clock, which tests and replays pin, decides every timestamp, and the
//! catalog stays free of hidden state. The engine clock ticks in milliseconds,
//! and the functions draw the sub millisecond part of each timestamp and a
//! small jitter of the root error from the session generator. The number of
//! words that a call draws depends only on its arguments, so one seed gives
//! one byte sequence under a pinned clock.
//!
//! A server keeps its character from reply to reply. Its precision, its root
//! error, and the phase of its clock updates follow from its reference
//! identifier, so the replies of one server agree, and the reference
//! timestamp changes only when the server updates its clock, as a real one
//! does, and never goes back.
//!
//! [`FUNCTIONS`] is the catalog that `modules/ntp/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
pub mod wire;

use std::net::Ipv4Addr;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS};
pub use wire::{Packet, WireError};

use wire::{
    micros_to_fraction, short_from_micros, timestamp, unix_ms, LEAP_UNSYNCHRONIZED, MODE_CLIENT,
    MODE_SERVER, MODE_SYMMETRIC_ACTIVE, MODE_SYMMETRIC_PASSIVE,
};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "ntp";

/// The precision exponent of every request, 2^-20 seconds or about one
/// microsecond, which a Linux host reports.
pub const CLIENT_PRECISION: i8 = -20;

/// The smallest poll exponent that a reply carries. RFC 5905 fixes the
/// minimum poll interval at 2^4 seconds, so a bare SNTP request with the
/// exponent 0 gets 4 back.
pub const MIN_POLL: i64 = 4;

/// The largest poll exponent of RFC 5905.
pub const MAX_POLL: i64 = 17;

/// The reference identifier of a secondary server that has none given. The
/// address lies in a documentation range of RFC 5737.
const DEFAULT_SYNC_SOURCE: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 9);

/// The reference identifier of a primary server that has none given.
const DEFAULT_PRIMARY: [u8; 4] = *b"GPS\0";

/// The template functions of the `ntp` module. The functions hold no state, so
/// the interpreter shares one value between all sessions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ntp;

fn invalid(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

/// Maps a conversion failure of an instant to an argument error.
fn era(e: WireError) -> FnError {
    invalid(e.to_string())
}

/// Draws a value from `lo` to `hi` inclusive. One call uses one word.
fn draw(rng: &mut dyn RngCore, lo: u32, hi: u32) -> u32 {
    lo + rng.next_u32() % (hi - lo + 1)
}

/// Draws the position of an instant inside its millisecond.
fn sub_ms(rng: &mut dyn RngCore) -> u32 {
    draw(rng, 0, 999_999)
}

/// Parses `reference` into a reference identifier for `stratum`.
///
/// An empty text gives the default of the stratum. A text that parses as a
/// dotted IPv4 address gives its four bytes, and any other text gives its
/// ASCII bytes padded with zeros, at most four printable characters, as the
/// codes of RFC 5905 table 11 (`GPS`, `PPS`, `NIST`) and the Kiss o' Death
/// codes (`RATE`, `DENY`, `RSTR`) are.
fn reference_id(stratum: u8, reference: &str) -> Result<[u8; 4], FnError> {
    if reference.is_empty() {
        return Ok(match stratum {
            0 => [0; 4],
            1 => DEFAULT_PRIMARY,
            _ => DEFAULT_SYNC_SOURCE.octets(),
        });
    }
    if let Ok(addr) = reference.parse::<Ipv4Addr>() {
        return Ok(addr.octets());
    }
    let b = reference.as_bytes();
    if b.len() > 4 || !b.iter().all(|c| c.is_ascii_graphic()) {
        return Err(invalid(format!(
            "reference '{reference}' is neither an IPv4 address nor 1 to 4 printable ASCII characters"
        )));
    }
    let mut id = [0u8; 4];
    id[..b.len()].copy_from_slice(b);
    Ok(id)
}

/// Checks a version argument.
fn check_version(version: i64) -> Result<u8, FnError> {
    u8::try_from(version)
        .ok()
        .filter(|v| (1..=4).contains(v))
        .ok_or_else(|| invalid(format!("version must lie in 1 to 4, got {version}")))
}

/// Checks a stratum argument.
fn check_stratum(stratum: i64) -> Result<u8, FnError> {
    u8::try_from(stratum)
        .ok()
        .filter(|s| *s <= 16)
        .ok_or_else(|| invalid(format!("stratum must lie in 0 to 16, got {stratum}")))
}

/// A number that depends only on the reference identifier, so that one server
/// keeps its precision, its root error, and the phase of its clock updates
/// from reply to reply. The high bits are the well mixed ones.
fn server_hash(id: [u8; 4]) -> u64 {
    u64::from(u32::from_be_bytes(id)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Returns the root delay and the root dispersion of a server with `stratum`
/// and the hash `h`, which grow with the distance from the primary source.
/// Each server has its own base error, and every packet adds up to 300
/// microseconds of jitter.
fn root_error(stratum: u8, h: u64, rng: &mut dyn RngCore) -> (u32, u32) {
    let jitter = u64::from(draw(rng, 0, 300));
    let delay = 2_000 + (h >> 24) % 18_000 + jitter;
    let dispersion = 1_000 + (h >> 48) % 9_000 + jitter;
    match stratum {
        0 => (0, 0),
        1 => (0, short_from_micros(dispersion / 8)),
        s => {
            let hops = u64::from(s - 1);
            (
                short_from_micros(delay * hops),
                short_from_micros(dispersion * hops),
            )
        }
    }
}

/// Returns the reference timestamp of a server: the time of its last clock
/// update. A primary server updates every 64 seconds and a secondary server
/// every 1024 seconds, each on a phase of its own, and the update lies one
/// second before the boundary. Replies between two updates therefore carry
/// one reference timestamp.
fn reference_time(now_ms: i64, stratum: u8, h: u64) -> Result<u64, FnError> {
    let period: i64 = if stratum == 1 { 64_000 } else { 1_024_000 };
    let phase = ((h >> 32) % period as u64) as i64;
    let age = (now_ms - phase).rem_euclid(period) + 1_000;
    timestamp(now_ms - age, ((h >> 8) % 1_000_000) as u32).map_err(era)
}

/// The arguments of `ntp.query`.
#[derive(Clone, Copy, Debug)]
pub struct QueryArgs<'a> {
    /// The instant of the request in milliseconds since the Unix epoch.
    pub now_ms: i64,
    /// The version, 1 to 4.
    pub version: i64,
    /// The poll exponent, 0 to 17.
    pub poll: i64,
    /// The mode, 3 for a client and 1 for a symmetric active peer.
    pub mode: i64,
    /// The stratum of the sender, 0 to 16.
    pub stratum: i64,
    /// The reference identifier for a stratum above 0, or the empty string
    /// for the default of the stratum.
    pub reference: &'a str,
}

/// Renders a request, which is a client packet of mode 3 or the packet of a
/// symmetric active peer of mode 1.
///
/// A client that has no clock to offer (stratum 0) leaves everything but the
/// header fields and the transmit timestamp zero, as an SNTP client does. A
/// peer with a stratum above 0 also states its root delay, root dispersion,
/// reference identifier, and the time of its last clock update, as `ntpd`
/// does. Mode 1 requires a stratum of 1 to 16.
pub fn query(a: &QueryArgs<'_>, rng: &mut dyn RngCore, out: &mut Vec<u8>) -> Result<(), FnError> {
    let QueryArgs {
        now_ms,
        version,
        poll,
        mode,
        stratum,
        reference,
    } = *a;
    let version = check_version(version)?;
    let stratum = check_stratum(stratum)?;
    let poll = i8::try_from(poll)
        .ok()
        .filter(|p| (0..=17).contains(p))
        .ok_or_else(|| invalid(format!("poll must lie in 0 to 17, got {poll}")))?;
    let mode = match u8::try_from(mode) {
        Ok(m @ (MODE_CLIENT | MODE_SYMMETRIC_ACTIVE)) => m,
        _ => return Err(invalid(format!("mode must be 1 or 3, got {mode}"))),
    };
    if mode == MODE_SYMMETRIC_ACTIVE && stratum == 0 {
        return Err(invalid("mode 1 needs a stratum from 1 to 16"));
    }
    let transmit = timestamp(now_ms, sub_ms(rng)).map_err(era)?;
    let mut p = Packet {
        leap: if stratum == 16 {
            LEAP_UNSYNCHRONIZED
        } else {
            0
        },
        version,
        mode,
        stratum,
        poll,
        precision: CLIENT_PRECISION,
        root_delay: 0,
        root_dispersion: 0,
        reference_id: [0; 4],
        reference: 0,
        origin: 0,
        receive: 0,
        transmit,
    };
    if stratum > 0 {
        p.reference_id = reference_id(stratum, reference)?;
        let h = server_hash(p.reference_id);
        (p.root_delay, p.root_dispersion) = root_error(stratum, h, rng);
        if stratum < 16 {
            p.reference = reference_time(now_ms, stratum, h)?;
        }
    } else if !reference.is_empty() {
        return Err(invalid("a request with stratum 0 has no reference"));
    }
    out.extend_from_slice(&p.encode());
    Ok(())
}

/// The arguments of `ntp.reply`.
#[derive(Clone, Copy, Debug)]
pub struct ReplyArgs<'a> {
    /// The request, a packet of mode 1 or 3.
    pub request: &'a [u8],
    /// The instant at which the server handles the request, in milliseconds
    /// since the Unix epoch.
    pub now_ms: i64,
    /// The stratum of the server, 1 to 15.
    pub stratum: i64,
    /// The reference identifier, or the empty string for the default of the
    /// stratum.
    pub reference: &'a str,
    /// A Kiss o' Death code. A non empty code makes the reply a KoD packet.
    pub kiss: &'a str,
}

/// Renders the reply to `request`.
///
/// A mode 3 request gets a mode 4 reply and a mode 1 request a mode 2 reply.
/// The reply copies the version of the request, copies the transmit
/// timestamp of the request into the origin timestamp, and clamps the poll
/// exponent to [`MIN_POLL`] and [`MAX_POLL`]. The receive timestamp is the
/// instant of the call, and the transmit timestamp follows it by 20 to 300
/// microseconds. A server of stratum 1 reports no root delay and a few
/// hundred microseconds of dispersion, and a secondary server reports both in
/// proportion to its distance from the primary source. The precision, the
/// root error, and the reference timestamp follow from the reference
/// identifier and the instant, so the replies of one server agree, and the
/// reference timestamp, the last clock update, moves forward every 64 seconds
/// for a primary server and every 1024 seconds for a secondary one.
///
/// A KoD reply (RFC 5905 section 7.4) has leap indicator 3, stratum 0, the
/// code as reference identifier, and no root error or reference timestamp.
pub fn reply(a: &ReplyArgs<'_>, rng: &mut dyn RngCore, out: &mut Vec<u8>) -> Result<(), FnError> {
    let q = Packet::parse(a.request).map_err(|e| protocol(format!("request: {e}")))?;
    let mode = match q.mode {
        MODE_CLIENT => MODE_SERVER,
        MODE_SYMMETRIC_ACTIVE => MODE_SYMMETRIC_PASSIVE,
        other => {
            return Err(protocol(format!(
                "a request has mode 1 or 3, the message has mode {other}"
            )))
        }
    };
    let stratum = check_stratum(a.stratum)?;
    if !(1..=15).contains(&stratum) && a.kiss.is_empty() {
        return Err(invalid(format!(
            "a server has a stratum from 1 to 15, got {stratum}"
        )));
    }
    let poll = i64::from(q.poll).clamp(MIN_POLL, MAX_POLL) as i8;
    let receive = timestamp(a.now_ms, sub_ms(rng)).map_err(era)?;
    let transmit = receive + micros_to_fraction(u64::from(draw(rng, 20, 300)));
    let id = reference_id(stratum, a.reference)?;
    let h = server_hash(id);
    let reference = reference_time(a.now_ms, stratum, h)?;
    let (root_delay, root_dispersion) = root_error(stratum, h, rng);
    let precision = -(18 + ((h >> 40) % 8) as i8);
    let mut p = Packet {
        leap: 0,
        version: q.version,
        mode,
        stratum,
        poll,
        precision,
        root_delay,
        root_dispersion,
        reference_id: id,
        reference,
        origin: q.transmit,
        receive,
        transmit,
    };
    if !a.kiss.is_empty() {
        if !a.reference.is_empty() {
            return Err(invalid("a Kiss o' Death reply takes its code from kiss"));
        }
        p.leap = LEAP_UNSYNCHRONIZED;
        p.stratum = 0;
        p.root_delay = 0;
        p.root_dispersion = 0;
        p.reference = 0;
        p.reference_id = reference_id(0, a.kiss)?;
    }
    out.extend_from_slice(&p.encode());
    Ok(())
}

/// Returns one field of the packet in `message` as text.
///
/// The names are `version`, `mode`, `leap`, `stratum`, `poll`, `precision`,
/// `reference_id` (the code of a stratum 0 or 1 packet, otherwise the dotted
/// address), `root_delay_ms`, `root_dispersion_ms`, and `transmit_ms`, the
/// transmit timestamp as milliseconds since the Unix epoch. A packet whose
/// transmit timestamp is zero has no `transmit_ms`, which is an error.
pub fn field(message: &[u8], name: &str) -> Result<String, FnError> {
    let p = Packet::parse(message).map_err(|e| protocol(e.to_string()))?;
    Ok(match name {
        "version" => p.version.to_string(),
        "mode" => p.mode.to_string(),
        "leap" => p.leap.to_string(),
        "stratum" => p.stratum.to_string(),
        "poll" => p.poll.to_string(),
        "precision" => p.precision.to_string(),
        "reference_id" => p.reference_text(),
        "root_delay_ms" => wire::short_to_ms(p.root_delay).to_string(),
        "root_dispersion_ms" => wire::short_to_ms(p.root_dispersion).to_string(),
        "transmit_ms" => unix_ms(p.transmit)
            .ok_or_else(|| protocol("the transmit timestamp is zero"))?
            .to_string(),
        other => {
            return Err(invalid(format!(
                "field '{other}' is not one of version, mode, leap, stratum, poll, precision, \
                 reference_id, root_delay_ms, root_dispersion_ms, and transmit_ms"
            )))
        }
    })
}

impl TemplateFn for Ntp {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not ntp", call.module),
            ));
        }
        match call.function {
            "query" => {
                let a = args::bind(&catalog::QUERY, &call.args)?;
                let q = QueryArgs {
                    now_ms: a.int("now_ms")?,
                    version: a.int("version")?,
                    poll: a.int("poll")?,
                    mode: a.int("mode")?,
                    stratum: a.int("stratum")?,
                    reference: a.text("reference")?,
                };
                query(&q, rng, out)
            }
            "reply" => {
                let a = args::bind(&catalog::REPLY, &call.args)?;
                let r = ReplyArgs {
                    request: a.bytes("request")?,
                    now_ms: a.int("now_ms")?,
                    stratum: a.int("stratum")?,
                    reference: a.text("reference")?,
                    kiss: a.text("kiss")?,
                };
                reply(&r, rng, out)
            }
            "field" => {
                let a = args::bind(&catalog::FIELD, &call.args)?;
                let text = field(a.bytes("message")?, a.text("name")?)?;
                out.extend_from_slice(text.as_bytes());
                Ok(())
            }
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("ntp has no function '{other}'"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fps_flow::function::NamedArg;
    use fps_flow::{ArgValue, CallArgs};
    use wire::HEADER_LEN;

    /// 2026-11-10T12:00:00Z in milliseconds.
    const T0: i64 = 1_794_312_000_000;

    fn render(
        function: &'static str,
        positional: &[ArgValue<'_>],
        named: &[NamedArg<'_>],
    ) -> Result<Vec<u8>, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::new(42);
        let mut out = Vec::new();
        let c = FnCall {
            module: "ntp",
            function,
            args: CallArgs { positional, named },
        };
        Ntp.render(&c, &mut rng, &mut out).map(|()| out)
    }

    fn kw<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
        NamedArg { name, value }
    }

    fn ask(named: &[NamedArg<'_>]) -> Vec<u8> {
        render("query", &[ArgValue::Int(T0)], named).unwrap()
    }

    fn answer(request: &[u8], now_ms: i64, named: &[NamedArg<'_>]) -> Packet {
        let out = render(
            "reply",
            &[ArgValue::Bytes(request), ArgValue::Int(now_ms)],
            named,
        )
        .unwrap();
        assert_eq!(out.len(), HEADER_LEN);
        Packet::parse(&out).unwrap()
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "dns",
            function: "query",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        let err = Ntp.render(&c, &mut rng, &mut Vec::new()).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
        let err = render("sync", &[], &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
    }

    #[test]
    fn a_default_query_is_an_sntp_style_client_packet() {
        let q = ask(&[]);
        assert_eq!(q.len(), 48);
        let p = Packet::parse(&q).unwrap();
        assert_eq!((p.leap, p.version, p.mode), (0, 4, 3));
        assert_eq!((p.stratum, p.poll, p.precision), (0, 6, CLIENT_PRECISION));
        assert_eq!(p.reference_id, [0; 4]);
        assert_eq!((p.root_delay, p.root_dispersion), (0, 0));
        assert_eq!((p.reference, p.origin, p.receive), (0, 0, 0));
        // The transmit timestamp lies in the millisecond that the caller gave.
        assert_eq!(unix_ms(p.transmit), Some(T0));
        // The same seed gives the same bytes, another seed another low bits.
        assert_eq!(q, ask(&[]));
        let mut rng = rand_pcg::Pcg64Mcg::new(7);
        let mut other = Vec::new();
        let q7 = QueryArgs {
            now_ms: T0,
            version: 4,
            poll: 6,
            mode: 3,
            stratum: 0,
            reference: "",
        };
        query(&q7, &mut rng, &mut other).unwrap();
        assert_ne!(q[40..], other[40..]);
        assert_eq!(q[..40], other[..40]);
    }

    #[test]
    fn query_takes_version_poll_and_mode() {
        for (version, poll) in [(1, 0), (3, 4), (4, 10), (4, 17)] {
            let q = ask(&[
                kw("version", ArgValue::Int(version)),
                kw("poll", ArgValue::Int(poll)),
            ]);
            let p = Packet::parse(&q).unwrap();
            assert_eq!((i64::from(p.version), i64::from(p.poll)), (version, poll));
        }
        let peer = ask(&[
            kw("mode", ArgValue::Int(1)),
            kw("stratum", ArgValue::Int(3)),
            kw("reference", ArgValue::Text("198.51.100.4")),
        ]);
        let p = Packet::parse(&peer).unwrap();
        assert_eq!((p.mode, p.stratum), (1, 3));
        assert_eq!(p.reference_id, [198, 51, 100, 4]);
        assert!(p.root_delay > 0 && p.root_dispersion > 0);
        // The clock was last set up to 1024 seconds before the request.
        let age = unix_ms(p.transmit).unwrap() - unix_ms(p.reference).unwrap();
        assert!((1_000..=1_025_000).contains(&age), "{age}");
    }

    #[test]
    fn query_checks_its_arguments() {
        let bad: &[&[NamedArg<'_>]] = &[
            &[kw("version", ArgValue::Int(0))],
            &[kw("version", ArgValue::Int(5))],
            &[kw("poll", ArgValue::Int(-1))],
            &[kw("poll", ArgValue::Int(18))],
            &[kw("mode", ArgValue::Int(4))],
            &[kw("mode", ArgValue::Int(1))],
            &[kw("stratum", ArgValue::Int(17))],
            &[kw("reference", ArgValue::Text("GPS"))],
            &[
                kw("stratum", ArgValue::Int(1)),
                kw("reference", ArgValue::Text("TOOLONG")),
            ],
            &[
                kw("stratum", ArgValue::Int(1)),
                kw("reference", ArgValue::Text("a b")),
            ],
        ];
        for named in bad {
            let err = render("query", &[ArgValue::Int(T0)], named).unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{named:?}");
        }
        // Outside era 0.
        let err = render("query", &[ArgValue::Int(2_200_000_000_000)], &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::InvalidArgument);
    }

    #[test]
    fn an_unsynchronized_peer_carries_leap_3_and_no_reference_time() {
        let q = ask(&[
            kw("mode", ArgValue::Int(1)),
            kw("stratum", ArgValue::Int(16)),
        ]);
        let p = Packet::parse(&q).unwrap();
        assert_eq!((p.leap, p.stratum, p.reference), (3, 16, 0));
    }

    #[test]
    fn the_reply_copies_the_request_and_stamps_the_server_times() {
        let q = ask(&[]);
        let req = Packet::parse(&q).unwrap();
        let r = answer(&q, T0 + 23, &[]);
        assert_eq!((r.leap, r.version, r.mode), (0, 4, 4));
        assert_eq!(r.stratum, 2);
        assert_eq!(r.poll, 6);
        assert_eq!(r.origin, req.transmit, "origin is the client transmit time");
        assert_eq!(unix_ms(r.receive), Some(T0 + 23));
        assert!(r.transmit > r.receive);
        // The server transmits within 300 microseconds.
        assert!(r.transmit - r.receive <= micros_to_fraction(300));
        assert_eq!(r.reference_id, [203, 0, 113, 9]);
        assert!(r.reference < r.receive);
        assert!(r.reference > 0);
        assert!((18..=25).contains(&-i16::from(r.precision)));
        assert!(r.root_delay > 0 && r.root_dispersion > 0);
    }

    #[test]
    fn the_reply_echoes_the_version_and_clamps_the_poll() {
        for (version, poll_in, poll_out) in [(3, 0, 4), (3, 6, 6), (4, 10, 10), (1, 3, 4)] {
            let q = ask(&[
                kw("version", ArgValue::Int(version)),
                kw("poll", ArgValue::Int(poll_in)),
            ]);
            let r = answer(&q, T0, &[]);
            assert_eq!(
                (i64::from(r.version), i64::from(r.poll)),
                (version, poll_out)
            );
        }
    }

    #[test]
    fn strata_set_the_reference_id_and_the_root_error() {
        let q = ask(&[]);
        let one = answer(&q, T0, &[kw("stratum", ArgValue::Int(1))]);
        assert_eq!(one.reference_id, *b"GPS\0");
        assert_eq!(one.root_delay, 0);
        assert!(one.root_dispersion < short_from_micros(10_000));
        let age = unix_ms(one.receive).unwrap() - unix_ms(one.reference).unwrap();
        assert!((1_000..=61_000).contains(&age), "{age}");
        let pps = answer(
            &q,
            T0,
            &[
                kw("stratum", ArgValue::Int(1)),
                kw("reference", ArgValue::Text("PPS")),
            ],
        );
        assert_eq!(pps.reference_id, *b"PPS\0");
        let addressed = answer(
            &q,
            T0,
            &[
                kw("stratum", ArgValue::Int(3)),
                kw("reference", ArgValue::Text("192.0.2.77")),
            ],
        );
        assert_eq!(addressed.reference_id, [192, 0, 2, 77]);
        let three = answer(&q, T0, &[kw("stratum", ArgValue::Int(3))]);
        let two = answer(&q, T0, &[]);
        // Each hop adds its own delay, so stratum 3 reports twice the unit.
        assert!(three.root_delay.abs_diff(two.root_delay * 2) <= 1);
        assert!(three.root_dispersion.abs_diff(two.root_dispersion * 2) <= 1);
    }

    #[test]
    fn a_peer_packet_gets_a_passive_reply() {
        let q = ask(&[
            kw("mode", ArgValue::Int(1)),
            kw("stratum", ArgValue::Int(2)),
        ]);
        let r = answer(&q, T0, &[]);
        assert_eq!(r.mode, 2);
        assert_eq!(r.origin, Packet::parse(&q).unwrap().transmit);
    }

    #[test]
    fn a_kiss_of_death_reply_follows_rfc_5905() {
        let q = ask(&[]);
        let r = answer(&q, T0, &[kw("kiss", ArgValue::Text("RATE"))]);
        assert_eq!((r.leap, r.stratum, r.mode), (3, 0, 4));
        assert_eq!(r.reference_id, *b"RATE");
        assert_eq!(r.reference_text(), "RATE");
        assert_eq!((r.root_delay, r.root_dispersion, r.reference), (0, 0, 0));
        assert_eq!(r.origin, Packet::parse(&q).unwrap().transmit);
        for (kiss, reference) in [("TOOLONG", ""), ("RATE", "192.0.2.1")] {
            let err = render(
                "reply",
                &[ArgValue::Bytes(&q), ArgValue::Int(T0)],
                &[
                    kw("kiss", ArgValue::Text(kiss)),
                    kw("reference", ArgValue::Text(reference)),
                ],
            )
            .unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{kiss} {reference}");
        }
    }

    #[test]
    fn reply_rejects_bad_requests_and_arguments() {
        let q = ask(&[]);
        let a = |request: &[u8], named: &[NamedArg<'_>]| {
            render(
                "reply",
                &[ArgValue::Bytes(request), ArgValue::Int(T0)],
                named,
            )
            .unwrap_err()
        };
        assert_eq!(a(b"short", &[]).kind, FnErrorKind::Protocol);
        // A server packet is not a request.
        let mut server = q.clone();
        server[0] = (server[0] & 0xf8) | 4;
        assert_eq!(a(&server, &[]).kind, FnErrorKind::Protocol);
        // Version 5 is no NTP version.
        let mut v5 = q.clone();
        v5[0] = (v5[0] & 0xc7) | (5 << 3);
        assert_eq!(a(&v5, &[]).kind, FnErrorKind::Protocol);
        for stratum in [0, 16, 17, -1] {
            let err = a(&q, &[kw("stratum", ArgValue::Int(stratum))]);
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{stratum}");
        }
    }

    #[test]
    fn field_reads_the_header() {
        let q = ask(&[kw("poll", ArgValue::Int(8))]);
        let f = |m: &[u8], name: &'static str| {
            render("field", &[ArgValue::Bytes(m), ArgValue::Text(name)], &[])
        };
        assert_eq!(f(&q, "version").unwrap(), b"4");
        assert_eq!(f(&q, "mode").unwrap(), b"3");
        assert_eq!(f(&q, "poll").unwrap(), b"8");
        assert_eq!(f(&q, "precision").unwrap(), b"-20");
        assert_eq!(f(&q, "transmit_ms").unwrap(), T0.to_string().as_bytes());
        let r = render(
            "reply",
            &[ArgValue::Bytes(&q), ArgValue::Int(T0)],
            &[kw("kiss", ArgValue::Text("DENY"))],
        )
        .unwrap();
        assert_eq!(f(&r, "reference_id").unwrap(), b"DENY");
        assert_eq!(f(&r, "stratum").unwrap(), b"0");
        assert_eq!(f(&r, "leap").unwrap(), b"3");
        let ok = render("reply", &[ArgValue::Bytes(&q), ArgValue::Int(T0)], &[]).unwrap();
        assert_eq!(f(&ok, "reference_id").unwrap(), b"203.0.113.9");
        assert!(f(&ok, "root_delay_ms").unwrap().len() <= 3);
        assert_eq!(f(b"x", "mode").unwrap_err().kind, FnErrorKind::Protocol);
        assert_eq!(
            f(&q, "jitter").unwrap_err().kind,
            FnErrorKind::InvalidArgument
        );
        // A packet without a transmit timestamp has no transmit_ms.
        let mut zero = q.clone();
        zero[40..48].fill(0);
        assert_eq!(
            f(&zero, "transmit_ms").unwrap_err().kind,
            FnErrorKind::Protocol
        );
    }

    #[test]
    fn one_server_keeps_its_character_between_replies() {
        let q = ask(&[]);
        // Replies from 10 seconds to 40 minutes apart.
        let mut last_reference = 0;
        let mut seen = std::collections::BTreeSet::new();
        for step in 0..100 {
            let now = T0 + step * 25_000;
            let r = answer(&q, now, &[]);
            assert_eq!(r.precision, answer(&q, T0, &[]).precision, "precision");
            assert_eq!(r.reference_id, [203, 0, 113, 9]);
            // The reference timestamp never goes back and trails the reply by
            // at most one update period and a second.
            assert!(r.reference >= last_reference, "step {step}");
            last_reference = r.reference;
            seen.insert(r.reference);
            let age = now - unix_ms(r.reference).unwrap();
            assert!((1_000..=1_025_000).contains(&age), "{age}");
            // The root error varies by the jitter alone.
            let first = answer(&q, T0, &[]);
            let jitter = micros_to_fraction(300) >> 16;
            assert!(r.root_delay.abs_diff(first.root_delay) as u64 <= jitter + 1);
        }
        // 2500 seconds hold two or three updates of a 1024 second period.
        assert!((2..=4).contains(&seen.len()), "{seen:?}");
        // Another server has another character.
        let other = answer(&q, T0, &[kw("reference", ArgValue::Text("198.51.100.4"))]);
        assert_ne!(other.reference, answer(&q, T0, &[]).reference);
    }
}
