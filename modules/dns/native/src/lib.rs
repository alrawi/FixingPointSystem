// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `dns` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements `dns.query`, `dns.answer_ip`, `dns.response`,
//! and `dns.tcp_length` behind the [`TemplateFn`] trait of `fps-flow`, so the
//! interpreter renders `{{ dns.query(name=vars.host, qtype='AAAA') }}` without
//! a WebAssembly runtime. Messages follow RFC 1035 with the record types A,
//! AAAA, CNAME, MX, and TXT, and NS and SOA for the zone apex.
//!
//! The server side answers from a [`Zone`] in master file format. [`Dns`]
//! holds one parsed zone, by default the zone that the module ships in
//! `modules/dns/zones/example.test.zone`, and every session shares it.
//! A UDP response longer than the `udp_size` argument carries only the
//! question and the TC flag, so the client retries over TCP, where each
//! message carries the two byte length prefix of RFC 1035 section 4.2.2.
//!
//! [`FUNCTIONS`] is the catalog that `modules/dns/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
pub mod wire;
pub mod zone;

use std::net::IpAddr;
use std::sync::Arc;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS};
pub use wire::{Message, Name, RData, Rcode, Record, RecordType};
pub use zone::{Lookup, Zone, ZoneError};

use wire::{
    Question, CLASS_IN, FLAG_AA, FLAG_QR, FLAG_RD, FLAG_TC, HEADER_LEN, MAX_UDP, UDP_LIMIT,
};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "dns";

/// Bits of the opcode in the flag word.
const OPCODE_MASK: u16 = 0x7800;

/// The template functions of the `dns` module for one session.
///
/// The value shares its zone through an [`Arc`], so cloning it for each
/// session is cheap. Only `dns.query` draws from the session generator, one
/// 32 bit word per call for the transaction id.
#[derive(Clone, Debug)]
pub struct Dns {
    zone: Arc<Zone>,
}

impl Dns {
    /// Creates the functions with the zone that `dns.response` answers from.
    pub fn new(zone: Zone) -> Self {
        Dns {
            zone: Arc::new(zone),
        }
    }

    /// Creates the functions with the zone that the module ships.
    pub fn bundled() -> Result<Self, ZoneError> {
        Zone::bundled().map(Dns::new)
    }

    /// Returns the zone.
    pub fn zone(&self) -> &Zone {
        &self.zone
    }
}

fn invalid(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

fn internal(e: wire::WireError) -> FnError {
    FnError::new(FnErrorKind::Internal, e.to_string())
}

/// Renders a query for `name` and the type mnemonic `qtype`.
pub fn query(
    name: &str,
    qtype: &str,
    tcp: bool,
    rng: &mut dyn RngCore,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    let name = Name::parse(name).map_err(|e| invalid(e.to_string()))?;
    let qtype = RecordType::parse(qtype).ok_or_else(|| {
        invalid(format!(
            "qtype '{qtype}' is not one of A, AAAA, CNAME, MX, TXT, NS, and SOA"
        ))
    })?;
    // The low half of one draw, so each call advances the generator once.
    let id = (rng.next_u32() & 0xffff) as u16;
    let m = Message {
        id,
        flags: FLAG_RD,
        questions: vec![Question {
            name,
            qtype: qtype.code(),
            qclass: CLASS_IN,
        }],
        ..Message::default()
    };
    let bytes = if tcp { m.encode_tcp() } else { m.encode() };
    out.extend_from_slice(&bytes.map_err(internal)?);
    Ok(())
}

/// Returns the first A or AAAA address in the answer section of `message`.
pub fn answer_ip(message: &[u8]) -> Result<IpAddr, FnError> {
    let m = Message::parse(message).map_err(|e| protocol(e.to_string()))?;
    if !m.is_response() {
        return Err(protocol("the message is a query, not a response"));
    }
    m.answers
        .iter()
        .find_map(|r| match r.data {
            RData::A(a) => Some(IpAddr::V4(a)),
            RData::Aaaa(a) => Some(IpAddr::V6(a)),
            _ => None,
        })
        .ok_or_else(|| {
            let what = if m.is_truncated() {
                "the response is truncated and has no address"
            } else {
                "the answer section has no address"
            };
            protocol(what)
        })
}

/// The arguments of `dns.response`.
#[derive(Clone, Copy, Debug)]
pub struct ResponseArgs<'a> {
    /// The query message, without a TCP length prefix.
    pub query: &'a [u8],
    /// One address to answer with, or the empty string to answer from the
    /// zone.
    pub address: &'a str,
    /// The TTL of the `address` answer.
    pub ttl: i64,
    /// Prefix the message with its TCP length and never truncate it.
    pub tcp: bool,
    /// The largest UDP message before truncation.
    pub udp_size: i64,
}

/// Builds the response to a query from `zone`, or from one address.
pub fn response(zone: &Zone, a: &ResponseArgs<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
    let ttl = u32::try_from(a.ttl)
        .ok()
        .filter(|t| *t <= i32::MAX as u32)
        .ok_or_else(|| invalid(format!("ttl must lie in 0 to 2147483647, got {}", a.ttl)))?;
    let udp_size = usize::try_from(a.udp_size)
        .ok()
        .filter(|s| (UDP_LIMIT..=MAX_UDP).contains(s))
        .ok_or_else(|| {
            invalid(format!(
                "udp_size must lie in {UDP_LIMIT} to {}, got {}",
                MAX_UDP, a.udp_size
            ))
        })?;
    let address: Option<IpAddr> = if a.address.is_empty() {
        None
    } else {
        Some(
            a.address
                .parse()
                .map_err(|_| invalid(format!("address '{}' is not an IP address", a.address)))?,
        )
    };
    let q = Message::parse(a.query).map_err(|e| protocol(format!("query: {e}")))?;
    if q.is_response() {
        return Err(protocol("the message is a response, not a query"));
    }
    let mut r = Message {
        id: q.id,
        flags: FLAG_QR | (q.flags & (OPCODE_MASK | FLAG_RD)),
        questions: q.questions.iter().take(1).cloned().collect(),
        ..Message::default()
    };
    let rcode = match (q.opcode(), q.questions.as_slice()) {
        (0, [question]) if question.qclass != CLASS_IN => Rcode::Refused,
        (0, [question]) => match address {
            Some(ip) => {
                r.flags |= FLAG_AA;
                r.answers.push(Record {
                    name: question.name.clone(),
                    class: CLASS_IN,
                    ttl,
                    data: match ip {
                        IpAddr::V4(v4) => RData::A(v4),
                        IpAddr::V6(v6) => RData::Aaaa(v6),
                    },
                });
                Rcode::NoError
            }
            None => {
                let l = zone.lookup(&question.name, question.qtype);
                if l.authoritative {
                    r.flags |= FLAG_AA;
                }
                r.answers = l.answers;
                r.authority = l.authority;
                r.additional = l.additional;
                l.rcode.unwrap_or(Rcode::ServFail)
            }
        },
        (0, _) => Rcode::FormErr,
        _ => Rcode::NotImp,
    };
    r.flags |= rcode.code();
    let mut bytes = r.encode().map_err(internal)?;
    if !a.tcp && bytes.len() > udp_size {
        // RFC 2181 section 9: drop the records, keep the question, set TC.
        r.flags |= FLAG_TC;
        r.answers.clear();
        r.authority.clear();
        r.additional.clear();
        bytes = r.encode().map_err(internal)?;
    }
    if a.tcp {
        let len = u16::try_from(bytes.len())
            .map_err(|_| internal(wire::WireError::TooLong { len: bytes.len() }))?;
        out.extend_from_slice(&len.to_be_bytes());
    }
    out.extend_from_slice(&bytes);
    Ok(())
}

/// Returns the length that a two byte TCP length prefix announces.
pub fn tcp_length(prefix: &[u8]) -> Result<u16, FnError> {
    match prefix {
        [hi, lo] => {
            let n = u16::from_be_bytes([*hi, *lo]);
            if usize::from(n) < HEADER_LEN {
                return Err(protocol(format!(
                    "the length prefix announces {n} bytes, less than a header"
                )));
            }
            Ok(n)
        }
        _ => Err(protocol(format!(
            "a length prefix has 2 bytes, got {}",
            prefix.len()
        ))),
    }
}

impl TemplateFn for Dns {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not dns", call.module),
            ));
        }
        match call.function {
            "query" => {
                let a = args::bind(&catalog::QUERY, &call.args)?;
                query(
                    a.text("name")?,
                    a.text("qtype")?,
                    a.boolean("tcp")?,
                    rng,
                    out,
                )
            }
            "answer_ip" => {
                let a = args::bind(&catalog::ANSWER_IP, &call.args)?;
                let ip = answer_ip(a.bytes("message")?)?;
                out.extend_from_slice(ip.to_string().as_bytes());
                Ok(())
            }
            "response" => {
                let a = args::bind(&catalog::RESPONSE, &call.args)?;
                let r = ResponseArgs {
                    query: a.bytes("query")?,
                    address: a.text("address")?,
                    ttl: a.int("ttl")?,
                    tcp: a.boolean("tcp")?,
                    udp_size: a.int("udp_size")?,
                };
                response(&self.zone, &r, out)
            }
            "tcp_length" => {
                let a = args::bind(&catalog::TCP_LENGTH, &call.args)?;
                let n = tcp_length(a.bytes("prefix")?)?;
                out.extend_from_slice(n.to_string().as_bytes());
                Ok(())
            }
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("dns has no function '{other}'"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fps_flow::function::NamedArg;
    use fps_flow::{ArgValue, CallArgs};
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn dns() -> Dns {
        Dns::bundled().unwrap()
    }

    fn render(
        f: &mut Dns,
        function: &'static str,
        positional: &[ArgValue<'_>],
        named: &[NamedArg<'_>],
    ) -> Result<Vec<u8>, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::new(42);
        let mut out = Vec::new();
        let c = FnCall {
            module: "dns",
            function,
            args: CallArgs { positional, named },
        };
        f.render(&c, &mut rng, &mut out).map(|()| out)
    }

    fn kw<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
        NamedArg { name, value }
    }

    fn ask(name: &str, qtype: &str) -> Vec<u8> {
        render(
            &mut dns(),
            "query",
            &[ArgValue::Text(name), ArgValue::Text(qtype)],
            &[],
        )
        .unwrap()
    }

    fn answer(query: &[u8], named: &[NamedArg<'_>]) -> Message {
        let out = render(&mut dns(), "response", &[ArgValue::Bytes(query)], named).unwrap();
        Message::parse(&out).unwrap()
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "http1",
            function: "query",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        let err = dns().render(&c, &mut rng, &mut Vec::new()).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
        let err = render(&mut dns(), "resolve", &[], &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
    }

    #[test]
    fn query_renders_one_question_with_a_seeded_id() {
        let q = ask("www.example.test", "aaaa");
        let m = Message::parse(&q).unwrap();
        assert_eq!(m.flags, FLAG_RD);
        assert_eq!(m.questions.len(), 1);
        assert_eq!(m.questions[0].qtype, 28);
        assert_eq!(q.len(), HEADER_LEN + 18 + 4);
        // The same seed gives the same id, another seed another one.
        assert_eq!(q, ask("www.example.test", "AAAA"));
        let mut rng = rand_pcg::Pcg64Mcg::new(7);
        let mut other = Vec::new();
        query("www.example.test", "AAAA", false, &mut rng, &mut other).unwrap();
        assert_ne!(q[..2], other[..2]);
        assert_eq!(q[2..], other[2..]);
    }

    #[test]
    fn query_over_tcp_carries_the_length_prefix() {
        let q = render(
            &mut dns(),
            "query",
            &[ArgValue::Text("www.example.test")],
            &[kw("tcp", ArgValue::Boolean(true))],
        )
        .unwrap();
        assert_eq!(usize::from(u16::from_be_bytes([q[0], q[1]])), q.len() - 2);
        assert!(Message::parse(&q[2..]).is_ok());
    }

    #[test]
    fn query_rejects_bad_names_and_types() {
        for (name, qtype) in [("a..b", "A"), ("www.example.test", "SRV"), ("a b", "A")] {
            let err = render(
                &mut dns(),
                "query",
                &[ArgValue::Text(name), ArgValue::Text(qtype)],
                &[],
            )
            .unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{name} {qtype}");
        }
    }

    #[test]
    fn response_answers_each_record_type_from_the_zone() {
        for (name, qtype, want) in [
            (
                "www.example.test",
                "A",
                vec![RData::A(Ipv4Addr::new(192, 0, 2, 1))],
            ),
            (
                "www.example.test",
                "AAAA",
                vec![RData::Aaaa("2001:db8::1".parse::<Ipv6Addr>().unwrap())],
            ),
            (
                "alias.example.test",
                "CNAME",
                vec![RData::Cname(Name::parse("www.example.test").unwrap())],
            ),
            (
                "_dmarc.example.test",
                "TXT",
                vec![RData::Txt(vec![
                    b"v=DMARC1; p=reject; rua=mailto:dmarc@example.test".to_vec(),
                ])],
            ),
        ] {
            let m = answer(&ask(name, qtype), &[]);
            assert_eq!(m.rcode(), Some(Rcode::NoError), "{name} {qtype}");
            assert_eq!(m.flags & FLAG_AA, FLAG_AA);
            let got: Vec<RData> = m.answers.into_iter().map(|r| r.data).collect();
            assert_eq!(got, want, "{name} {qtype}");
        }
        let mx = answer(&ask("example.test", "MX"), &[]);
        assert_eq!(mx.answers.len(), 2);
        assert_eq!(mx.additional.len(), 3, "mail A, mail AAAA, and mail2 A");
    }

    #[test]
    fn response_echoes_id_question_and_rd() {
        let q = ask("WWW.Example.Test", "A");
        let r = render(&mut dns(), "response", &[ArgValue::Bytes(&q)], &[]).unwrap();
        assert_eq!(r[..2], q[..2]);
        assert_eq!(u16::from_be_bytes([r[2], r[3]]), 0x8500);
        assert_eq!(r[HEADER_LEN..q.len()], q[HEADER_LEN..]);
    }

    #[test]
    fn response_with_an_address_ignores_the_zone() {
        let q = ask("missing.example.test", "A");
        let m = answer(
            &q,
            &[
                kw("address", ArgValue::Text("2001:db8::7")),
                kw("ttl", ArgValue::Int(60)),
            ],
        );
        assert_eq!(m.rcode(), Some(Rcode::NoError));
        assert_eq!(m.answers[0].ttl, 60);
        assert!(matches!(m.answers[0].data, RData::Aaaa(_)));
    }

    #[test]
    fn negative_and_refused_answers() {
        let nx = answer(&ask("missing.example.test", "A"), &[]);
        assert_eq!(nx.rcode(), Some(Rcode::NxDomain));
        assert_eq!(nx.authority.len(), 1);
        let nodata = answer(&ask("mail2.example.test", "AAAA"), &[]);
        assert_eq!(nodata.rcode(), Some(Rcode::NoError));
        assert!(nodata.answers.is_empty());
        let refused = answer(&ask("www.other.test", "A"), &[]);
        assert_eq!(refused.rcode(), Some(Rcode::Refused));
        assert_eq!(refused.flags & FLAG_AA, 0);
    }

    #[test]
    fn a_large_udp_answer_truncates_and_tcp_carries_it_whole() {
        let q = ask("big.example.test", "TXT");
        let out = render(&mut dns(), "response", &[ArgValue::Bytes(&q)], &[]).unwrap();
        assert!(out.len() <= UDP_LIMIT);
        let udp = Message::parse(&out).unwrap();
        assert!(udp.is_truncated());
        assert!(udp.answers.is_empty());
        assert_eq!(udp.questions.len(), 1);
        let big = render(
            &mut dns(),
            "response",
            &[ArgValue::Bytes(&q)],
            &[kw("udp_size", ArgValue::Int(4096))],
        )
        .unwrap();
        assert!(!Message::parse(&big).unwrap().is_truncated());
        let tcp = render(
            &mut dns(),
            "response",
            &[ArgValue::Bytes(&q)],
            &[kw("tcp", ArgValue::Boolean(true))],
        )
        .unwrap();
        assert_eq!(tcp_length(&tcp[..2]).unwrap() as usize, tcp.len() - 2);
        let m = Message::parse(&tcp[2..]).unwrap();
        assert!(!m.is_truncated());
        assert!(tcp.len() > UDP_LIMIT);
        assert!(matches!(&m.answers[0].data, RData::Txt(s) if s.len() == 4));
    }

    #[test]
    fn malformed_queries_fail_or_get_error_codes() {
        let err = render(&mut dns(), "response", &[ArgValue::Bytes(b"short")], &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Protocol);
        let mut q = ask("www.example.test", "A");
        // A response is not a query.
        q[2] |= 0x80;
        let err = render(&mut dns(), "response", &[ArgValue::Bytes(&q)], &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Protocol);
        // Opcode 2 (STATUS) is not implemented.
        q[2] = 0x10 | 0x01;
        assert_eq!(answer(&q, &[]).rcode(), Some(Rcode::NotImp));
        // No question is a format error.
        let empty = [0x12, 0x34, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(answer(&empty, &[]).rcode(), Some(Rcode::FormErr));
        // Class CH is refused.
        let mut ch = ask("www.example.test", "A");
        let n = ch.len();
        ch[n - 1] = 3;
        assert_eq!(answer(&ch, &[]).rcode(), Some(Rcode::Refused));
    }

    #[test]
    fn response_checks_its_arguments() {
        let q = ask("www.example.test", "A");
        for named in [
            [kw("address", ArgValue::Text("not-an-ip"))],
            [kw("ttl", ArgValue::Int(-1))],
            [kw("udp_size", ArgValue::Int(100))],
            [kw("udp_size", ArgValue::Int(70_000))],
        ] {
            let err = render(&mut dns(), "response", &[ArgValue::Bytes(&q)], &named).unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{named:?}");
        }
    }

    #[test]
    fn answer_ip_reads_the_first_address_after_cnames() {
        let r = render(
            &mut dns(),
            "response",
            &[ArgValue::Bytes(&ask("static.example.test", "AAAA"))],
            &[],
        )
        .unwrap();
        let ip = render(&mut dns(), "answer_ip", &[ArgValue::Bytes(&r)], &[]).unwrap();
        assert_eq!(ip, b"2001:db8::80");
        let q = ask("www.example.test", "A");
        assert_eq!(
            answer_ip(&q).unwrap_err().message,
            "the message is a query, not a response"
        );
        let nx = render(
            &mut dns(),
            "response",
            &[ArgValue::Bytes(&ask("nope.example.test", "A"))],
            &[],
        )
        .unwrap();
        assert_eq!(answer_ip(&nx).unwrap_err().kind, FnErrorKind::Protocol);
        let big = render(
            &mut dns(),
            "response",
            &[ArgValue::Bytes(&ask("big.example.test", "TXT"))],
            &[],
        )
        .unwrap();
        assert!(answer_ip(&big).unwrap_err().message.contains("truncated"));
    }

    #[test]
    fn tcp_length_reads_two_bytes() {
        let n = render(
            &mut dns(),
            "tcp_length",
            &[ArgValue::Bytes(&[0x01, 0x02])],
            &[],
        )
        .unwrap();
        assert_eq!(n, b"258");
        for bad in [&[0u8][..], &[0, 1, 2], &[0, 11]] {
            assert_eq!(tcp_length(bad).unwrap_err().kind, FnErrorKind::Protocol);
        }
    }
}
