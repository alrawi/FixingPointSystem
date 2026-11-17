// SPDX-License-Identifier: GPL-3.0-only
//! The DHCPv4 message of RFC 2131 with the options of RFC 2132 that a client
//! needs.
//!
//! [`Message::parse`] reads the UDP payload of a DHCP packet and never panics
//! on short or odd input. It keeps the fields that a client acts on, skips
//! every option it does not know, and follows the option overload of option
//! 52, which moves options into the `file` and `sname` fields. [`Message::encode`]
//! writes the payload with the message type first, the end option, and zero
//! padding up to the 300 bytes that old BOOTP relays expect.

use std::net::Ipv4Addr;

use crate::pool::MacAddr;

/// The UDP port of a DHCP server.
pub const SERVER_PORT: u16 = 67;
/// The UDP port of a DHCP client.
pub const CLIENT_PORT: u16 = 68;
/// The shortest payload that [`Message::encode`] writes, in bytes.
pub const MIN_PAYLOAD: usize = 300;

/// The BOOTP operation of a message from a client.
pub const BOOTREQUEST: u8 = 1;
/// The BOOTP operation of a message from a server.
pub const BOOTREPLY: u8 = 2;
/// The broadcast flag of the `flags` field.
pub const FLAG_BROADCAST: u16 = 0x8000;

const MAGIC: [u8; 4] = [99, 130, 83, 99];
const FIXED_LEN: usize = 236;
const HTYPE_ETHERNET: u8 = 1;

/// The option codes that this module reads or writes.
pub mod code {
    /// Padding, one byte.
    pub const PAD: u8 = 0;
    /// The subnet mask.
    pub const SUBNET_MASK: u8 = 1;
    /// The routers of the subnet.
    pub const ROUTER: u8 = 3;
    /// The DNS servers.
    pub const DNS: u8 = 6;
    /// The address that the client asks for.
    pub const REQUESTED_IP: u8 = 50;
    /// The lease time in seconds.
    pub const LEASE_TIME: u8 = 51;
    /// Which fields of the message carry options too.
    pub const OVERLOAD: u8 = 52;
    /// The message type.
    pub const MESSAGE_TYPE: u8 = 53;
    /// The identifier of the server.
    pub const SERVER_ID: u8 = 54;
    /// The options that the client wants.
    pub const PARAM_REQUEST: u8 = 55;
    /// A text message of the server, for example the reason of a NAK.
    pub const MESSAGE: u8 = 56;
    /// The renewal time T1 in seconds.
    pub const RENEWAL_TIME: u8 = 58;
    /// The rebinding time T2 in seconds.
    pub const REBINDING_TIME: u8 = 59;
    /// The identifier of the client.
    pub const CLIENT_ID: u8 = 61;
    /// The end of the options.
    pub const END: u8 = 255;
}

/// The type of a DHCP message, option 53.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageType {
    /// The client looks for servers.
    Discover,
    /// A server offers an address.
    Offer,
    /// The client asks for an address, or renews one.
    Request,
    /// The client refuses an address.
    Decline,
    /// The server confirms a lease.
    Ack,
    /// The server refuses a request.
    Nak,
    /// The client gives its lease back.
    Release,
    /// The client asks for configuration only.
    Inform,
}

impl MessageType {
    /// The value of option 53.
    pub fn value(self) -> u8 {
        match self {
            Self::Discover => 1,
            Self::Offer => 2,
            Self::Request => 3,
            Self::Decline => 4,
            Self::Ack => 5,
            Self::Nak => 6,
            Self::Release => 7,
            Self::Inform => 8,
        }
    }

    /// The type of a value of option 53, or `None` for an unknown value.
    pub fn from_value(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Discover,
            2 => Self::Offer,
            3 => Self::Request,
            4 => Self::Decline,
            5 => Self::Ack,
            6 => Self::Nak,
            7 => Self::Release,
            8 => Self::Inform,
            _ => return None,
        })
    }
}

/// Errors that [`Message::parse`] reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The payload is shorter than the fixed fields and the magic cookie.
    #[error("dhcp message of {len} bytes is shorter than the {FIXED_LEN} byte header and cookie")]
    TooShort {
        /// The length of the payload.
        len: usize,
    },
    /// The four bytes after the fixed fields are not the DHCP magic cookie, so
    /// the message is plain BOOTP.
    #[error("dhcp message has no magic cookie")]
    BadMagic,
    /// The hardware type is not Ethernet with six address bytes.
    #[error("dhcp message has hardware type {htype} with {hlen} address bytes, not ethernet")]
    UnsupportedHardware {
        /// The hardware type.
        htype: u8,
        /// The hardware address length.
        hlen: u8,
    },
    /// An option runs past the end of its area.
    #[error("dhcp option {code} is truncated")]
    TruncatedOption {
        /// The option code.
        code: u8,
    },
    /// An option has a length that its type does not allow.
    #[error("dhcp option {code} has length {len}")]
    BadOptionLength {
        /// The option code.
        code: u8,
        /// The length byte of the option.
        len: usize,
    },
    /// The message has no option 53.
    #[error("dhcp message has no message type")]
    MissingMessageType,
    /// Option 53 holds a value that no message type has.
    #[error("dhcp message type {0} is unknown")]
    UnknownMessageType(u8),
}

/// The options of a message that a client reads or writes. Options that are
/// not here are skipped on parse and never written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Option 54, the identifier of the server.
    pub server_id: Option<Ipv4Addr>,
    /// Option 50, the address that the client asks for.
    pub requested_ip: Option<Ipv4Addr>,
    /// Option 51, the lease time in seconds. `u32::MAX` means forever.
    pub lease_secs: Option<u32>,
    /// Option 58, the renewal time T1 in seconds.
    pub renewal_secs: Option<u32>,
    /// Option 59, the rebinding time T2 in seconds.
    pub rebinding_secs: Option<u32>,
    /// Option 1, the subnet mask.
    pub subnet_mask: Option<Ipv4Addr>,
    /// Option 3, the routers in order of preference.
    pub routers: Vec<Ipv4Addr>,
    /// Option 6, the DNS servers.
    pub dns: Vec<Ipv4Addr>,
    /// Option 61, the client identifier.
    pub client_id: Option<Vec<u8>>,
    /// Option 55, the option codes that the client wants.
    pub param_request: Vec<u8>,
    /// Option 56, the text of a server message.
    pub message: Option<String>,
}

/// One DHCP message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// [`BOOTREQUEST`] or [`BOOTREPLY`].
    pub op: u8,
    /// The number of relays that the message passed.
    pub hops: u8,
    /// The transaction identifier.
    pub xid: u32,
    /// Seconds since the client began to acquire or renew.
    pub secs: u16,
    /// The flags, of which only [`FLAG_BROADCAST`] is defined.
    pub flags: u16,
    /// The client address, set when the client can answer ARP for it.
    pub ciaddr: Ipv4Addr,
    /// The address that the server gives the client.
    pub yiaddr: Ipv4Addr,
    /// The address of the next server of the boot process.
    pub siaddr: Ipv4Addr,
    /// The address of the relay.
    pub giaddr: Ipv4Addr,
    /// The hardware address of the client.
    pub chaddr: MacAddr,
    /// The message type.
    pub kind: MessageType,
    /// The options.
    pub options: Options,
}

impl Message {
    /// Returns a message of `kind` with every address zero, the operation that
    /// fits the kind, and no options.
    pub fn new(kind: MessageType, xid: u32, chaddr: MacAddr) -> Self {
        let op = match kind {
            MessageType::Offer | MessageType::Ack | MessageType::Nak => BOOTREPLY,
            _ => BOOTREQUEST,
        };
        Self {
            op,
            hops: 0,
            xid,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr,
            kind,
            options: Options::default(),
        }
    }

    /// Reports whether the message asks for a broadcast reply.
    pub fn wants_broadcast(&self) -> bool {
        self.flags & FLAG_BROADCAST != 0
    }

    /// Parses the UDP payload of a DHCP packet.
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let short = || ParseError::TooShort { len: b.len() };
        if b.len() < FIXED_LEN + MAGIC.len() {
            return Err(short());
        }
        let u8_at = |at: usize| b.get(at).copied().ok_or_else(short);
        let ip_at = |at: usize| arr::<4>(b, at).map(Ipv4Addr::from).ok_or_else(short);
        let (htype, hlen) = (u8_at(1)?, u8_at(2)?);
        if htype != HTYPE_ETHERNET || hlen != 6 {
            return Err(ParseError::UnsupportedHardware { htype, hlen });
        }
        if arr::<4>(b, FIXED_LEN) != Some(MAGIC) {
            return Err(ParseError::BadMagic);
        }
        let mut options = Options::default();
        let mut kind = None;
        let mut overload = 0;
        let opts = b.get(FIXED_LEN + MAGIC.len()..).ok_or_else(short)?;
        parse_options(opts, &mut options, &mut kind, &mut overload)?;
        // Option 52 says that the `file` (1) and `sname` (2) fields hold
        // options, and the `file` field comes first.
        if overload & 1 != 0 {
            parse_options(
                b.get(108..FIXED_LEN).ok_or_else(short)?,
                &mut options,
                &mut kind,
                &mut 0,
            )?;
        }
        if overload & 2 != 0 {
            parse_options(
                b.get(44..108).ok_or_else(short)?,
                &mut options,
                &mut kind,
                &mut 0,
            )?;
        }
        let kind = kind.ok_or(ParseError::MissingMessageType)?;
        let kind = MessageType::from_value(kind).ok_or(ParseError::UnknownMessageType(kind))?;
        Ok(Self {
            op: u8_at(0)?,
            hops: u8_at(3)?,
            xid: u32::from_be_bytes(arr::<4>(b, 4).ok_or_else(short)?),
            secs: u16::from_be_bytes(arr::<2>(b, 8).ok_or_else(short)?),
            flags: u16::from_be_bytes(arr::<2>(b, 10).ok_or_else(short)?),
            ciaddr: ip_at(12)?,
            yiaddr: ip_at(16)?,
            siaddr: ip_at(20)?,
            giaddr: ip_at(24)?,
            chaddr: MacAddr(arr::<6>(b, 28).ok_or_else(short)?),
            kind,
            options,
        })
    }

    /// Writes the UDP payload of the message.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0u8; FIXED_LEN];
        out[0] = self.op;
        out[1] = HTYPE_ETHERNET;
        out[2] = 6;
        out[3] = self.hops;
        out[4..8].copy_from_slice(&self.xid.to_be_bytes());
        out[8..10].copy_from_slice(&self.secs.to_be_bytes());
        out[10..12].copy_from_slice(&self.flags.to_be_bytes());
        out[12..16].copy_from_slice(&self.ciaddr.octets());
        out[16..20].copy_from_slice(&self.yiaddr.octets());
        out[20..24].copy_from_slice(&self.siaddr.octets());
        out[24..28].copy_from_slice(&self.giaddr.octets());
        out[28..34].copy_from_slice(&self.chaddr.0);
        out.extend_from_slice(&MAGIC);
        put(&mut out, code::MESSAGE_TYPE, &[self.kind.value()]);
        let o = &self.options;
        let ip = |out: &mut Vec<u8>, c: u8, v: Option<Ipv4Addr>| {
            if let Some(v) = v {
                put(out, c, &v.octets());
            }
        };
        let secs = |out: &mut Vec<u8>, c: u8, v: Option<u32>| {
            if let Some(v) = v {
                put(out, c, &v.to_be_bytes());
            }
        };
        if let Some(id) = &o.client_id {
            put(&mut out, code::CLIENT_ID, id);
        }
        ip(&mut out, code::REQUESTED_IP, o.requested_ip);
        ip(&mut out, code::SERVER_ID, o.server_id);
        secs(&mut out, code::LEASE_TIME, o.lease_secs);
        secs(&mut out, code::RENEWAL_TIME, o.renewal_secs);
        secs(&mut out, code::REBINDING_TIME, o.rebinding_secs);
        ip(&mut out, code::SUBNET_MASK, o.subnet_mask);
        put_ips(&mut out, code::ROUTER, &o.routers);
        put_ips(&mut out, code::DNS, &o.dns);
        if !o.param_request.is_empty() {
            put(&mut out, code::PARAM_REQUEST, &o.param_request);
        }
        if let Some(m) = &o.message {
            put(&mut out, code::MESSAGE, m.as_bytes());
        }
        out.push(code::END);
        if out.len() < MIN_PAYLOAD {
            out.resize(MIN_PAYLOAD, 0);
        }
        out
    }
}

fn arr<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at.checked_add(N)?)?.try_into().ok()
}

/// Writes one option. A value longer than 255 bytes is cut, which no option
/// of this module reaches.
fn put(out: &mut Vec<u8>, c: u8, data: &[u8]) {
    let data = data.get(..255).unwrap_or(data);
    out.push(c);
    // The slice above holds at most 255 bytes.
    out.push(data.len() as u8);
    out.extend_from_slice(data);
}

fn put_ips(out: &mut Vec<u8>, c: u8, ips: &[Ipv4Addr]) {
    if ips.is_empty() {
        return;
    }
    let bytes: Vec<u8> = ips.iter().take(63).flat_map(|a| a.octets()).collect();
    put(out, c, &bytes);
}

fn ips(code: u8, data: &[u8]) -> Result<Vec<Ipv4Addr>, ParseError> {
    let (chunks, rest) = data.as_chunks::<4>();
    if chunks.is_empty() || !rest.is_empty() {
        return Err(ParseError::BadOptionLength {
            code,
            len: data.len(),
        });
    }
    Ok(chunks.iter().copied().map(Ipv4Addr::from).collect())
}

fn one_ip(code: u8, data: &[u8]) -> Result<Ipv4Addr, ParseError> {
    match <[u8; 4]>::try_from(data) {
        Ok(a) => Ok(Ipv4Addr::from(a)),
        Err(_) => Err(ParseError::BadOptionLength {
            code,
            len: data.len(),
        }),
    }
}

fn one_u32(code: u8, data: &[u8]) -> Result<u32, ParseError> {
    match <[u8; 4]>::try_from(data) {
        Ok(a) => Ok(u32::from_be_bytes(a)),
        Err(_) => Err(ParseError::BadOptionLength {
            code,
            len: data.len(),
        }),
    }
}

fn parse_options(
    area: &[u8],
    o: &mut Options,
    kind: &mut Option<u8>,
    overload: &mut u8,
) -> Result<(), ParseError> {
    let mut i = 0;
    while let Some(&c) = area.get(i) {
        i += 1;
        match c {
            code::PAD => continue,
            code::END => return Ok(()),
            _ => {}
        }
        let len = usize::from(
            area.get(i)
                .copied()
                .ok_or(ParseError::TruncatedOption { code: c })?,
        );
        i += 1;
        let data = area
            .get(i..i + len)
            .ok_or(ParseError::TruncatedOption { code: c })?;
        i += len;
        match c {
            code::SUBNET_MASK => o.subnet_mask = Some(one_ip(c, data)?),
            code::ROUTER => o.routers = ips(c, data)?,
            code::DNS => o.dns = ips(c, data)?,
            code::REQUESTED_IP => o.requested_ip = Some(one_ip(c, data)?),
            code::LEASE_TIME => o.lease_secs = Some(one_u32(c, data)?),
            code::RENEWAL_TIME => o.renewal_secs = Some(one_u32(c, data)?),
            code::REBINDING_TIME => o.rebinding_secs = Some(one_u32(c, data)?),
            code::SERVER_ID => o.server_id = Some(one_ip(c, data)?),
            code::OVERLOAD | code::MESSAGE_TYPE => {
                let [v] = data else {
                    return Err(ParseError::BadOptionLength { code: c, len });
                };
                if c == code::OVERLOAD {
                    *overload = *v;
                } else {
                    *kind = Some(*v);
                }
            }
            code::CLIENT_ID => o.client_id = Some(data.to_vec()),
            code::PARAM_REQUEST => o.param_request = data.to_vec(),
            code::MESSAGE => o.message = Some(String::from_utf8_lossy(data).into_owned()),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: MacAddr = MacAddr([2, 0, 0, 0, 0, 7]);

    /// A DISCOVER written out byte by byte from RFC 2131 and RFC 2132, so the
    /// encoder is checked against something other than the parser.
    fn discover_bytes() -> Vec<u8> {
        let mut b = vec![0u8; 236];
        b[0] = 1; // BOOTREQUEST
        b[1] = 1; // ethernet
        b[2] = 6;
        b[4..8].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
        b[10] = 0x80; // broadcast flag
        b[28..34].copy_from_slice(&MAC.0);
        b.extend_from_slice(&[99, 130, 83, 99]);
        b.extend_from_slice(&[53, 1, 1]);
        b.extend_from_slice(&[61, 7, 1, 2, 0, 0, 0, 0, 7]);
        b.extend_from_slice(&[55, 3, 1, 3, 51]);
        b.push(255);
        b.resize(300, 0);
        b
    }

    fn discover() -> Message {
        let mut m = Message::new(MessageType::Discover, 0x1234_5678, MAC);
        m.flags = FLAG_BROADCAST;
        m.options.client_id = Some(vec![1, 2, 0, 0, 0, 0, 7]);
        m.options.param_request = vec![1, 3, 51];
        m
    }

    #[test]
    fn a_discover_encodes_to_the_bytes_of_the_rfc() {
        assert_eq!(discover().encode(), discover_bytes());
    }

    #[test]
    fn the_bytes_of_the_rfc_parse_to_the_discover() {
        assert_eq!(Message::parse(&discover_bytes()).unwrap(), discover());
    }

    #[test]
    fn an_offer_with_every_option_round_trips() {
        let mut m = Message::new(MessageType::Offer, 9, MAC);
        m.secs = 3;
        m.hops = 1;
        m.yiaddr = Ipv4Addr::new(10, 0, 0, 55);
        m.siaddr = Ipv4Addr::new(10, 0, 0, 1);
        m.giaddr = Ipv4Addr::new(10, 0, 9, 1);
        m.options = Options {
            server_id: Some(Ipv4Addr::new(10, 0, 0, 1)),
            requested_ip: Some(Ipv4Addr::new(10, 0, 0, 55)),
            lease_secs: Some(3600),
            renewal_secs: Some(1800),
            rebinding_secs: Some(3150),
            subnet_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            routers: vec![Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2)],
            dns: vec![Ipv4Addr::new(10, 0, 0, 53)],
            client_id: Some(vec![1, 2, 0, 0, 0, 0, 7]),
            param_request: vec![1, 3, 6],
            message: Some("hello".into()),
        };
        let bytes = m.encode();
        assert!(bytes.len() >= MIN_PAYLOAD);
        assert_eq!(Message::parse(&bytes).unwrap(), m);
        assert_eq!(m.op, BOOTREPLY);
    }

    #[test]
    fn padding_and_unknown_options_are_skipped() {
        let mut b = discover_bytes();
        b.truncate(240);
        b.extend_from_slice(&[0, 0, 53, 1, 1, 0, 252, 3, 9, 9, 9, 12, 2, b'h', b'i', 255]);
        let m = Message::parse(&b).unwrap();
        assert_eq!(m.kind, MessageType::Discover);
        assert_eq!(m.options, Options::default());
    }

    #[test]
    fn a_message_without_an_end_option_ends_with_its_area() {
        let mut b = discover_bytes();
        b.truncate(240);
        b.extend_from_slice(&[53, 1, 3]);
        assert_eq!(Message::parse(&b).unwrap().kind, MessageType::Request);
    }

    #[test]
    fn option_overload_reads_the_file_and_sname_fields() {
        let mut b = discover_bytes();
        b.truncate(240);
        b.extend_from_slice(&[53, 1, 5, 52, 1, 3, 255]);
        // `sname` starts at 44 and `file` at 108.
        b[108..108 + 6].copy_from_slice(&[51, 4, 0, 0, 0x0e, 0x10]);
        b[108 + 6] = 255;
        b[44..44 + 7].copy_from_slice(&[3, 4, 10, 0, 0, 1, 255]);
        let m = Message::parse(&b).unwrap();
        assert_eq!(m.kind, MessageType::Ack);
        assert_eq!(m.options.lease_secs, Some(3600));
        assert_eq!(m.options.routers, vec![Ipv4Addr::new(10, 0, 0, 1)]);
    }

    #[test]
    fn short_and_odd_input_is_an_error() {
        let ok = discover_bytes();
        assert!(matches!(
            Message::parse(&ok[..239]),
            Err(ParseError::TooShort { len: 239 })
        ));
        let mut b = ok.clone();
        b[236] = 0;
        assert_eq!(Message::parse(&b), Err(ParseError::BadMagic));
        let mut b = ok.clone();
        b[1] = 6;
        assert_eq!(
            Message::parse(&b),
            Err(ParseError::UnsupportedHardware { htype: 6, hlen: 6 })
        );
        let mut b = ok.clone();
        b[2] = 8;
        assert!(matches!(
            Message::parse(&b),
            Err(ParseError::UnsupportedHardware { .. })
        ));
        // An option whose length runs past the end of the payload.
        let mut b = ok.clone();
        b.truncate(240);
        b.extend_from_slice(&[53, 1, 1, 61, 9, 1, 2]);
        assert_eq!(
            Message::parse(&b),
            Err(ParseError::TruncatedOption { code: 61 })
        );
        let mut b = ok.clone();
        b.truncate(240);
        b.extend_from_slice(&[53, 1, 1, 51]);
        assert_eq!(
            Message::parse(&b),
            Err(ParseError::TruncatedOption { code: 51 })
        );
        // A lease time of three bytes, a router list of five, a type of two.
        for (opt, code) in [
            (vec![51u8, 3, 0, 0, 1], 51u8),
            (vec![3, 5, 1, 2, 3, 4, 5], 3),
            (vec![50, 2, 1, 2], 50),
            (vec![53, 2, 1, 1], 53),
            (vec![3, 0], 3),
        ] {
            let mut b = ok.clone();
            b.truncate(240);
            b.extend_from_slice(&[53, 1, 1]);
            b.extend_from_slice(&opt);
            assert!(
                matches!(
                    Message::parse(&b),
                    Err(ParseError::BadOptionLength { code: c, .. }) if c == code
                ),
                "option {code}"
            );
        }
        let mut b = ok.clone();
        b.truncate(240);
        b.extend_from_slice(&[255]);
        assert_eq!(Message::parse(&b), Err(ParseError::MissingMessageType));
        let mut b = ok;
        b.truncate(240);
        b.extend_from_slice(&[53, 1, 99, 255]);
        assert_eq!(Message::parse(&b), Err(ParseError::UnknownMessageType(99)));
    }

    #[test]
    fn every_message_type_round_trips_through_its_value() {
        for v in 1..=8 {
            assert_eq!(MessageType::from_value(v).unwrap().value(), v);
        }
        assert_eq!(MessageType::from_value(0), None);
        assert_eq!(MessageType::from_value(9), None);
    }

    #[test]
    fn a_non_utf8_server_message_does_not_fail_the_parse() {
        let mut b = discover_bytes();
        b.truncate(240);
        b.extend_from_slice(&[53, 1, 6, 56, 2, 0xff, 0xfe, 255]);
        let m = Message::parse(&b).unwrap();
        assert_eq!(m.kind, MessageType::Nak);
        assert_eq!(m.options.message.as_deref(), Some("\u{fffd}\u{fffd}"));
    }

    proptest::proptest! {
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..400)) {
            let _ = Message::parse(&bytes);
        }

        #[test]
        fn random_options_after_a_valid_header_never_panic(
            opts in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..120)
        ) {
            let mut b = discover_bytes();
            b.truncate(240);
            b.extend_from_slice(&opts);
            let _ = Message::parse(&b);
        }

        #[test]
        fn a_parsed_message_encodes_and_parses_again_to_itself(
            xid in proptest::prelude::any::<u32>(),
            lease in proptest::option::of(proptest::prelude::any::<u32>()),
            router in proptest::option::of(proptest::prelude::any::<[u8; 4]>()),
        ) {
            let mut m = Message::new(MessageType::Ack, xid, MAC);
            m.options.lease_secs = lease;
            m.options.routers = router.map(|r| vec![Ipv4Addr::from(r)]).unwrap_or_default();
            proptest::prop_assert_eq!(Message::parse(&m.encode()).unwrap(), m);
        }
    }
}
