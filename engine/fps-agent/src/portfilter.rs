// SPDX-License-Identifier: GPL-3.0-only
//! The port filter of a capture (AGT-12).
//!
//! `StartCaptureCommand.port` is empty to capture every session of the plan.
//! Otherwise it names what the capture keeps, in one of two forms.
//!
//! 1. The name of a port of the plan (`network.ports[].name` of the test
//!    configuration), which is what the REST contract of `startCapture`
//!    requires. The capture keeps the sessions that run between hosts of the
//!    pools that sit on that port: a session matches when its client or its
//!    server address lies in the prefix of such a pool.
//! 2. A server port of TCP and UDP: one port (`53`), a comma separated list
//!    (`80,443`), a range (`8000-8999`), or a list of both (`53,8000-8999`).
//!    The capture keeps the sessions whose server port is in the set, for TCP
//!    and for UDP alike.
//!
//! A name wins over a number, so a plan port that is called `53` is a name.
//! A text that is neither is [`InvalidPort`], which the session reports as
//! `ERROR_CODE_INVALID_ARGUMENT`.
//!
//! # Where the filter acts
//!
//! The taps of the TCP sessions live in `fps-runtime`, which hands every
//! session of a worker to the capture of the shared cell, and the capture
//! records every sampled session into its ring. The filter therefore acts when
//! the agent joins the ring into the file ([`FrameFilter`]): it reads the
//! Ethernet frames that the capture synthesized, finds where a session starts,
//! takes its server end from the first frame (the SYN of a TCP session, the
//! first datagram of a UDP session, both sent by the client), and drops the
//! frames of the sessions that do not match. A filtered file holds the matching
//! sessions only, and its packet count, size, and digest describe that file.
//! The ring itself still takes the other sessions, so its byte limit counts
//! them, see `docs/wiki/Agent.md`.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ops::RangeInclusive;

use fps_hosts::{Ipv4Prefix, Ipv6Prefix};

use crate::plan::Plan;

/// The most items that a list of server ports may have.
const MAX_ITEMS: usize = 64;

/// A text that names neither a port of the plan nor a set of server ports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidPort(String);

/// An address prefix of a host pool.
#[derive(Debug, Clone, Copy)]
enum Net {
    V4(Ipv4Prefix),
    V6(Ipv6Prefix),
}

impl Net {
    fn contains(&self, ip: IpAddr) -> bool {
        match (self, ip) {
            (Net::V4(n), IpAddr::V4(a)) => n.contains(a),
            (Net::V6(n), IpAddr::V6(a)) => n.contains(a),
            _ => false,
        }
    }
}

/// The address prefixes of the host pools that sit on one port of the plan.
#[derive(Debug, Clone)]
pub struct PortNets {
    port: String,
    nets: Vec<Net>,
}

/// Returns the prefixes of the host pools of `plan`, by the port that they sit
/// on. A prefix that does not parse is left out, because the run path refused
/// such a plan before.
pub fn port_nets(plan: &Plan) -> Vec<PortNets> {
    let mut out: Vec<PortNets> = Vec::new();
    for pool in &plan.host_pools {
        let mut nets = Vec::new();
        if let Ok(p) = pool.ipv4_prefix.parse::<Ipv4Prefix>() {
            nets.push(Net::V4(p));
        }
        if let Ok(p) = pool.ipv6_prefix.parse::<Ipv6Prefix>() {
            nets.push(Net::V6(p));
        }
        match out.iter_mut().find(|n| n.port == pool.port) {
            Some(n) => n.nets.extend(nets),
            None => out.push(PortNets {
                port: pool.port.clone(),
                nets,
            }),
        }
    }
    out
}

/// What a capture keeps.
#[derive(Debug, Clone, Default)]
pub struct PortFilter {
    kind: Kind,
}

#[derive(Debug, Clone, Default)]
enum Kind {
    /// Every session.
    #[default]
    All,
    /// The sessions whose server port lies in one of the ranges.
    ServerPorts(Vec<RangeInclusive<u16>>),
    /// The sessions with a client or server host in the prefixes.
    Hosts(Vec<Net>),
}

impl PortFilter {
    /// Returns the filter that keeps every session.
    pub fn all() -> Self {
        Self::default()
    }

    /// Parses the `port` of a `StartCaptureCommand`. `plan_ports` are the port
    /// names of the plan and `pools` the pools on them.
    ///
    /// # Errors
    ///
    /// [`InvalidPort`] when `spec` is neither empty, nor the name of a port of
    /// the plan, nor a valid set of server ports.
    pub fn parse(
        spec: &str,
        plan_ports: &[String],
        pools: &[PortNets],
    ) -> Result<Self, InvalidPort> {
        if spec.is_empty() {
            return Ok(Self::all());
        }
        if plan_ports.iter().any(|p| p == spec) {
            let nets = pools
                .iter()
                .filter(|p| p.port == spec)
                .flat_map(|p| p.nets.iter().copied())
                .collect();
            return Ok(Self {
                kind: Kind::Hosts(nets),
            });
        }
        match parse_ports(spec) {
            Ok(ranges) => Ok(Self {
                kind: Kind::ServerPorts(ranges),
            }),
            Err(why) => Err(InvalidPort(format!(
                "port {spec:?} is not a port of the plan ({}) and not a server port, a list such as 80,443, or a range such as 8000-8999: {why}",
                if plan_ports.is_empty() {
                    "the plan has none".to_owned()
                } else {
                    plan_ports.join(", ")
                }
            ))),
        }
    }

    /// Returns true when the filter keeps every session.
    pub fn is_all(&self) -> bool {
        matches!(self.kind, Kind::All)
    }

    fn matches(&self, client: IpAddr, server: IpAddr, server_port: u16) -> bool {
        match &self.kind {
            Kind::All => true,
            Kind::ServerPorts(ranges) => ranges.iter().any(|r| r.contains(&server_port)),
            Kind::Hosts(nets) => nets
                .iter()
                .any(|n| n.contains(client) || n.contains(server)),
        }
    }
}

fn parse_ports(spec: &str) -> Result<Vec<RangeInclusive<u16>>, String> {
    let port = |s: &str| -> Result<u16, String> {
        let s = s.trim();
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("{s:?} is not a port number"));
        }
        match s.parse::<u16>() {
            Ok(0) | Err(_) => Err(format!("{s:?} is not a port from 1 to 65535")),
            Ok(p) => Ok(p),
        }
    };
    let items: Vec<&str> = spec.split(',').collect();
    if items.len() > MAX_ITEMS {
        return Err(format!("more than {MAX_ITEMS} items"));
    }
    items
        .into_iter()
        .map(|item| match item.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (port(a)?, port(b)?);
                if a > b {
                    return Err(format!("the range {item:?} ends before it starts"));
                }
                Ok(a..=b)
            }
            None => port(item).map(|p| p..=p),
        })
        .collect()
}

/// The part of an Ethernet frame that the filter reads.
struct Ends {
    tcp: bool,
    src: (IpAddr, u16),
    dst: (IpAddr, u16),
    /// A TCP segment with SYN and without ACK, which opens a session.
    opens: bool,
}

/// Reads the Ethernet, IP, and TCP or UDP headers of a frame that the capture
/// synthesized, which has no VLAN tag and no IP extension header.
fn ends_of(frame: &[u8]) -> Option<Ends> {
    let ethertype = u16::from_be_bytes([*frame.get(12)?, *frame.get(13)?]);
    let ip = frame.get(14..)?;
    let (proto, src, dst, l4) = match ethertype {
        0x0800 => {
            let ihl = usize::from(ip.first()? & 0x0f) * 4;
            let src = Ipv4Addr::from(<[u8; 4]>::try_from(ip.get(12..16)?).ok()?);
            let dst = Ipv4Addr::from(<[u8; 4]>::try_from(ip.get(16..20)?).ok()?);
            (*ip.get(9)?, src.into(), dst.into(), ip.get(ihl..)?)
        }
        0x86dd => {
            let src = Ipv6Addr::from(<[u8; 16]>::try_from(ip.get(8..24)?).ok()?);
            let dst = Ipv6Addr::from(<[u8; 16]>::try_from(ip.get(24..40)?).ok()?);
            (*ip.get(6)?, src.into(), dst.into(), ip.get(40..)?)
        }
        _ => return None,
    };
    let sport = u16::from_be_bytes([*l4.first()?, *l4.get(1)?]);
    let dport = u16::from_be_bytes([*l4.get(2)?, *l4.get(3)?]);
    let tcp = match proto {
        6 => true,
        17 => false,
        _ => return None,
    };
    let opens = tcp && l4.get(13).is_some_and(|f| f & 0x12 == 0x02);
    Some(Ends {
        tcp,
        src: (src, sport),
        dst: (dst, dport),
        opens,
    })
}

/// Decides frame by frame whether a [`PortFilter`] keeps the session that the
/// frame belongs to.
///
/// The capture writes the frames of one session one after another, and the
/// first frame of a session goes from the client to the server. A frame starts
/// a new session when its two endpoints differ from those of the frame before,
/// or when it is a TCP SYN. The decision of that first frame holds for the rest
/// of the session.
#[derive(Debug)]
pub struct FrameFilter<'a> {
    filter: &'a PortFilter,
    current: Option<(bool, (IpAddr, u16), (IpAddr, u16), bool)>,
}

impl<'a> FrameFilter<'a> {
    /// Returns a filter that starts at the first frame of a file.
    pub fn new(filter: &'a PortFilter) -> Self {
        Self {
            filter,
            current: None,
        }
    }

    /// Returns true when the capture keeps `frame`. A frame that is no TCP or
    /// UDP segment is kept by the filter that keeps everything, and dropped by
    /// every other.
    pub fn keep(&mut self, frame: &[u8]) -> bool {
        if self.filter.is_all() {
            return true;
        }
        let Some(e) = ends_of(frame) else {
            return false;
        };
        let same = |a: (IpAddr, u16), b: (IpAddr, u16)| a == b;
        if let Some((tcp, a, b, keep)) = self.current {
            let continues = tcp == e.tcp
                && ((same(a, e.src) && same(b, e.dst)) || (same(a, e.dst) && same(b, e.src)));
            if continues && !e.opens {
                return keep;
            }
        }
        let keep = self.filter.matches(e.src.0, e.dst.0, e.dst.1);
        self.current = Some((e.tcp, e.src, e.dst, keep));
        keep
    }
}

impl fmt::Display for PortFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::All => f.write_str("every session"),
            Kind::ServerPorts(r) => {
                let items: Vec<String> = r
                    .iter()
                    .map(|r| {
                        if r.start() == r.end() {
                            r.start().to_string()
                        } else {
                            format!("{}-{}", r.start(), r.end())
                        }
                    })
                    .collect();
                write!(f, "server ports {}", items.join(","))
            }
            Kind::Hosts(n) => write!(f, "the hosts of {} pool prefixes", n.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets(port: &str, prefixes: &[&str]) -> PortNets {
        PortNets {
            port: port.into(),
            nets: prefixes
                .iter()
                .map(|p| Net::V4(p.parse().expect("prefix")))
                .collect(),
        }
    }

    fn plan_ports() -> Vec<String> {
        vec!["client0".into(), "server0".into(), "53".into()]
    }

    fn pools() -> Vec<PortNets> {
        vec![
            nets("client0", &["10.1.0.0/24"]),
            nets("server0", &["10.2.0.0/24"]),
        ]
    }

    fn parse(spec: &str) -> Result<PortFilter, InvalidPort> {
        PortFilter::parse(spec, &plan_ports(), &pools())
    }

    /// An Ethernet, IPv4, and TCP or UDP frame with no payload.
    fn frame(tcp: bool, src: ([u8; 4], u16), dst: ([u8; 4], u16), flags: u8) -> Vec<u8> {
        let mut f = vec![0u8; 12];
        f.extend_from_slice(&[0x08, 0x00]);
        let mut ip = vec![0u8; 20];
        ip[0] = 0x45;
        ip[9] = if tcp { 6 } else { 17 };
        ip[12..16].copy_from_slice(&src.0);
        ip[16..20].copy_from_slice(&dst.0);
        f.extend_from_slice(&ip);
        let mut l4 = vec![0u8; if tcp { 20 } else { 8 }];
        l4[0..2].copy_from_slice(&src.1.to_be_bytes());
        l4[2..4].copy_from_slice(&dst.1.to_be_bytes());
        if tcp {
            l4[13] = flags;
        }
        f.extend_from_slice(&l4);
        f
    }

    const SYN: u8 = 0x02;
    const ACK: u8 = 0x10;

    #[test]
    fn an_empty_port_keeps_everything() {
        let f = parse("").unwrap();
        assert!(f.is_all());
        let mut ff = FrameFilter::new(&f);
        assert!(ff.keep(&frame(true, ([1, 1, 1, 1], 1), ([2, 2, 2, 2], 2), SYN)));
        assert!(ff.keep(&[0u8; 3]), "a filter for everything reads nothing");
    }

    #[test]
    fn a_name_beats_a_number_and_a_number_is_a_server_port() {
        assert!(matches!(parse("server0").unwrap().kind, Kind::Hosts(n) if n.len() == 1));
        assert!(matches!(parse("53").unwrap().kind, Kind::Hosts(_)));
        let Kind::ServerPorts(r) = parse("80").unwrap().kind else {
            panic!("80 is a server port");
        };
        assert_eq!(r, [80..=80]);
        let Kind::ServerPorts(r) = parse("53, 8000-8999,443").unwrap().kind else {
            panic!("a list");
        };
        assert_eq!(r, [53..=53, 8000..=8999, 443..=443]);
        assert!(parse("65535").is_ok());
    }

    #[test]
    fn what_is_no_port_is_refused() {
        for bad in [
            "nope",
            "0",
            "65536",
            "-1",
            "80-",
            "-80",
            "90-80",
            "80,,81",
            "80,",
            ",80",
            "8 0",
            "0x50",
            "+80",
            "tcp/80",
            "80-90-100",
            "１２",
        ] {
            let e = parse(bad).expect_err(bad);
            assert!(e.to_string().contains("client0, server0"), "{bad}: {e}");
        }
        let many = vec!["1"; MAX_ITEMS + 1].join(",");
        assert!(parse(&many).is_err());
        assert!(PortFilter::parse("nope", &[], &[])
            .unwrap_err()
            .to_string()
            .contains("the plan has none"));
    }

    #[test]
    fn a_server_port_filter_judges_a_session_by_its_first_frame() {
        let f = parse("80,9000-9100").unwrap();
        let mut ff = FrameFilter::new(&f);
        let (c, s) = ([10, 1, 0, 1], [10, 2, 0, 1]);
        // A TCP session to port 80: the SYN opens it and the reverse frames follow.
        assert!(ff.keep(&frame(true, (c, 40000), (s, 80), SYN)));
        assert!(ff.keep(&frame(true, (s, 80), (c, 40000), SYN | ACK)));
        assert!(ff.keep(&frame(true, (c, 40000), (s, 80), ACK)));
        // A session to port 81 is skipped, whichever way its frames go.
        assert!(!ff.keep(&frame(true, (c, 40001), (s, 81), SYN)));
        assert!(!ff.keep(&frame(true, (s, 81), (c, 40001), SYN | ACK)));
        // A client port of the filter does not count: only the server end does.
        assert!(!ff.keep(&frame(true, (c, 80), (s, 8080), SYN)));
        assert!(!ff.keep(&frame(true, (s, 8080), (c, 80), ACK)));
        // The same four tuple again is a new session, because it starts with a SYN.
        assert!(ff.keep(&frame(true, (c, 80), (s, 9050), SYN)));
        // UDP uses the first datagram, which the client sent.
        assert!(!ff.keep(&frame(false, (c, 5000), (s, 53), 0)));
        assert!(!ff.keep(&frame(false, (s, 53), (c, 5000), 0)));
        assert!(ff.keep(&frame(false, (c, 5001), (s, 9001), 0)));
        assert!(ff.keep(&frame(false, (s, 9001), (c, 5001), 0)));
        // A frame that is not IP is dropped.
        assert!(!ff.keep(&[0u8; 60]));
    }

    #[test]
    fn a_port_name_keeps_the_sessions_of_its_pools() {
        let f = parse("server0").unwrap();
        let mut ff = FrameFilter::new(&f);
        // The server host sits in 10.2.0.0/24.
        assert!(ff.keep(&frame(
            true,
            ([10, 9, 0, 1], 4000),
            ([10, 2, 0, 7], 80),
            SYN
        )));
        assert!(!ff.keep(&frame(
            true,
            ([10, 9, 0, 1], 4001),
            ([10, 8, 0, 7], 80),
            SYN
        )));
        // So does the client host of the other port.
        let f = parse("client0").unwrap();
        let mut ff = FrameFilter::new(&f);
        assert!(ff.keep(&frame(false, ([10, 1, 0, 5], 4000), ([10, 8, 0, 7], 53), 0)));
        assert!(!ff.keep(&frame(false, ([10, 7, 0, 5], 4000), ([10, 8, 0, 7], 53), 0)));
    }

    #[test]
    fn an_ipv6_frame_is_read_too() {
        let mut f = vec![0u8; 12];
        f.extend_from_slice(&[0x86, 0xdd]);
        let mut ip = vec![0u8; 40];
        ip[0] = 0x60;
        ip[6] = 17;
        ip[8..24].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
        ip[24..40].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
        f.extend_from_slice(&ip);
        let mut l4 = vec![0u8; 8];
        l4[0..2].copy_from_slice(&5000u16.to_be_bytes());
        l4[2..4].copy_from_slice(&5300u16.to_be_bytes());
        f.extend_from_slice(&l4);
        let yes = parse("5300").unwrap();
        let no = parse("5301").unwrap();
        assert!(FrameFilter::new(&yes).keep(&f));
        assert!(!FrameFilter::new(&no).keep(&f));
    }
}
