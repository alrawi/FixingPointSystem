// SPDX-License-Identifier: GPL-3.0-only
//! The DNS message format of RFC 1035 section 4.
//!
//! [`Message::parse`] reads a message from untrusted bytes. It checks every
//! offset, accepts only compression pointers that point backwards, and never
//! panics. [`Message::encode`] writes a message with name compression, as
//! RFC 1035 section 4.1.4 describes, and [`Message::encode_tcp`] prefixes
//! the two byte length that RFC 1035 section 4.2.2 requires on TCP.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Length of the fixed message header.
pub const HEADER_LEN: usize = 12;

/// The UDP payload limit of a message without EDNS, RFC 1035 section 2.3.4.
pub const UDP_LIMIT: usize = 512;

/// Largest UDP payload over IPv4, the limit of the datagram rules.
pub const MAX_UDP: usize = 65_507;

/// Largest message that fits the two byte TCP length prefix.
pub const TCP_LIMIT: usize = 65_535;

/// Longest label, RFC 1035 section 2.3.4.
pub const MAX_LABEL: usize = 63;

/// Longest name in wire format, including the root label.
pub const MAX_NAME: usize = 255;

/// The class IN.
pub const CLASS_IN: u16 = 1;

/// Header flag QR, set in responses.
pub const FLAG_QR: u16 = 0x8000;
/// Header flag AA, an authoritative answer.
pub const FLAG_AA: u16 = 0x0400;
/// Header flag TC, a truncated message.
pub const FLAG_TC: u16 = 0x0200;
/// Header flag RD, recursion desired.
pub const FLAG_RD: u16 = 0x0100;
/// Header flag RA, recursion available.
pub const FLAG_RA: u16 = 0x0080;

/// Errors of parsing or encoding a message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    /// The message ends inside a field.
    #[error("message ends at offset {offset} inside the {field}")]
    Truncated {
        /// Offset at which the field starts.
        offset: usize,
        /// The field that the message cuts off.
        field: &'static str,
    },
    /// A compression pointer does not point to an earlier name.
    #[error("compression pointer at offset {offset} points forward to {target}")]
    BadPointer {
        /// Offset of the pointer.
        offset: usize,
        /// Offset that it names.
        target: usize,
    },
    /// A label uses the reserved length bits 01 or 10.
    #[error("label at offset {offset} has the reserved type bits {bits:#04x}")]
    BadLabel {
        /// Offset of the label length byte.
        offset: usize,
        /// The length byte.
        bits: u8,
    },
    /// A name is longer than 255 bytes in wire format.
    #[error("name at offset {offset} is longer than {MAX_NAME} bytes")]
    NameTooLong {
        /// Offset at which the name starts.
        offset: usize,
    },
    /// The data of a record does not match its type.
    #[error("record data of type {rtype} at offset {offset} is malformed")]
    BadRecordData {
        /// The record type.
        rtype: u16,
        /// Offset of the record data.
        offset: usize,
    },
    /// The encoded message exceeds the TCP length prefix.
    #[error("message of {len} bytes exceeds {TCP_LIMIT} bytes")]
    TooLong {
        /// Encoded length.
        len: usize,
    },
}

/// Errors of a name written as text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    /// The name has an empty label, as in `a..b`.
    #[error("name '{0}' has an empty label")]
    EmptyLabel(String),
    /// A label is longer than 63 bytes.
    #[error("name '{0}' has a label longer than {MAX_LABEL} bytes")]
    LabelTooLong(String),
    /// The name is longer than 255 bytes in wire format.
    #[error("name '{0}' is longer than {MAX_NAME} bytes in wire format")]
    TooLong(String),
    /// The name holds a byte that is not printable ASCII, or white space.
    #[error("name '{0}' holds a byte that is not printable ASCII")]
    BadByte(String),
}

/// A domain name as a sequence of labels, without the root label.
///
/// Labels keep their case, so a response repeats the question as the query
/// wrote it, and [`Name::eq_ignore_case`] compares names as RFC 4343 asks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Name {
    labels: Vec<Vec<u8>>,
}

impl Name {
    /// The root name.
    pub fn root() -> Self {
        Name::default()
    }

    /// Parses a name in dotted text form. A trailing dot is optional, and
    /// `.` or the empty string names the root.
    pub fn parse(text: &str) -> Result<Self, NameError> {
        let err = |f: fn(String) -> NameError| f(text.to_string());
        if text.bytes().any(|b| !b.is_ascii_graphic()) {
            return Err(err(NameError::BadByte));
        }
        let trimmed = text.strip_suffix('.').unwrap_or(text);
        if trimmed.is_empty() {
            return Ok(Name::root());
        }
        let mut labels = Vec::new();
        for l in trimmed.split('.') {
            if l.is_empty() {
                return Err(err(NameError::EmptyLabel));
            }
            if l.len() > MAX_LABEL {
                return Err(err(NameError::LabelTooLong));
            }
            labels.push(l.as_bytes().to_vec());
        }
        let name = Name { labels };
        if name.wire_len() > MAX_NAME {
            return Err(err(NameError::TooLong));
        }
        Ok(name)
    }

    /// Returns the labels.
    pub fn labels(&self) -> &[Vec<u8>] {
        &self.labels
    }

    /// Returns the length in uncompressed wire format.
    pub fn wire_len(&self) -> usize {
        self.labels.iter().map(|l| l.len() + 1).sum::<usize>() + 1
    }

    /// Returns true when both names are equal under ASCII case folding.
    pub fn eq_ignore_case(&self, other: &Name) -> bool {
        self.labels.len() == other.labels.len()
            && self
                .labels
                .iter()
                .zip(&other.labels)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    /// Returns true when `self` equals `zone` or lies below it.
    pub fn is_within(&self, zone: &Name) -> bool {
        let n = zone.labels.len();
        self.labels.len() >= n
            && self.labels[self.labels.len() - n..]
                .iter()
                .zip(&zone.labels)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    /// Returns the name with `suffix` appended, or `None` when the result
    /// is longer than 255 bytes.
    pub fn join(&self, suffix: &Name) -> Option<Name> {
        let mut labels = self.labels.clone();
        labels.extend(suffix.labels.iter().cloned());
        let name = Name { labels };
        (name.wire_len() <= MAX_NAME).then_some(name)
    }

    /// Returns the lowercase dotted key of the name, without a trailing dot.
    pub fn key(&self) -> String {
        self.to_string().to_ascii_lowercase()
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.labels.is_empty() {
            return f.write_str(".");
        }
        for (i, l) in self.labels.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            for &b in l {
                if b == b'.' || b == b'\\' {
                    write!(f, "\\{}", b as char)?;
                } else if b.is_ascii_graphic() {
                    write!(f, "{}", b as char)?;
                } else {
                    write!(f, "\\{b:03}")?;
                }
            }
        }
        Ok(())
    }
}

/// A record type that the module knows by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordType {
    /// An IPv4 address, RFC 1035.
    A,
    /// An authoritative name server, RFC 1035.
    Ns,
    /// A canonical name, RFC 1035.
    Cname,
    /// The start of a zone of authority, RFC 1035.
    Soa,
    /// A mail exchange, RFC 1035.
    Mx,
    /// Text strings, RFC 1035.
    Txt,
    /// An IPv6 address, RFC 3596.
    Aaaa,
}

impl RecordType {
    /// Every type, in code order.
    pub const ALL: [RecordType; 7] = [
        RecordType::A,
        RecordType::Ns,
        RecordType::Cname,
        RecordType::Soa,
        RecordType::Mx,
        RecordType::Txt,
        RecordType::Aaaa,
    ];

    /// Returns the type code.
    pub fn code(self) -> u16 {
        match self {
            RecordType::A => 1,
            RecordType::Ns => 2,
            RecordType::Cname => 5,
            RecordType::Soa => 6,
            RecordType::Mx => 15,
            RecordType::Txt => 16,
            RecordType::Aaaa => 28,
        }
    }

    /// Returns the type of a code.
    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.code() == code)
    }

    /// Returns the mnemonic.
    pub fn as_str(self) -> &'static str {
        match self {
            RecordType::A => "A",
            RecordType::Ns => "NS",
            RecordType::Cname => "CNAME",
            RecordType::Soa => "SOA",
            RecordType::Mx => "MX",
            RecordType::Txt => "TXT",
            RecordType::Aaaa => "AAAA",
        }
    }

    /// Parses a mnemonic, ignoring case.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str().eq_ignore_ascii_case(text))
    }
}

impl fmt::Display for RecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Response codes, RFC 1035 section 4.1.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rcode {
    /// No error.
    NoError,
    /// The server could not interpret the query.
    FormErr,
    /// The server failed.
    ServFail,
    /// The name does not exist.
    NxDomain,
    /// The server does not implement the query kind.
    NotImp,
    /// The server refuses the query.
    Refused,
}

impl Rcode {
    /// Returns the four bit code.
    pub fn code(self) -> u16 {
        match self {
            Rcode::NoError => 0,
            Rcode::FormErr => 1,
            Rcode::ServFail => 2,
            Rcode::NxDomain => 3,
            Rcode::NotImp => 4,
            Rcode::Refused => 5,
        }
    }

    /// Returns the code of the low four bits of `flags`, when the module
    /// knows it.
    pub fn from_flags(flags: u16) -> Option<Self> {
        [
            Rcode::NoError,
            Rcode::FormErr,
            Rcode::ServFail,
            Rcode::NxDomain,
            Rcode::NotImp,
            Rcode::Refused,
        ]
        .into_iter()
        .find(|r| r.code() == flags & 0x000f)
    }
}

/// The data of one record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RData {
    /// An IPv4 address.
    A(Ipv4Addr),
    /// An IPv6 address.
    Aaaa(Ipv6Addr),
    /// A name server.
    Ns(Name),
    /// A canonical name.
    Cname(Name),
    /// A mail exchange with its preference.
    Mx {
        /// Lower values are preferred.
        preference: u16,
        /// The exchange host.
        exchange: Name,
    },
    /// One or more character strings of at most 255 bytes each.
    Txt(Vec<Vec<u8>>),
    /// The start of a zone of authority.
    Soa {
        /// The primary name server.
        mname: Name,
        /// The mailbox of the responsible person.
        rname: Name,
        /// Zone serial number.
        serial: u32,
        /// Refresh interval in seconds.
        refresh: u32,
        /// Retry interval in seconds.
        retry: u32,
        /// Expire limit in seconds.
        expire: u32,
        /// Negative caching TTL in seconds, RFC 2308.
        minimum: u32,
    },
    /// The raw data of a type that the module does not know.
    Other(u16, Vec<u8>),
}

impl RData {
    /// Returns the type code.
    pub fn type_code(&self) -> u16 {
        match self {
            RData::A(_) => RecordType::A.code(),
            RData::Aaaa(_) => RecordType::Aaaa.code(),
            RData::Ns(_) => RecordType::Ns.code(),
            RData::Cname(_) => RecordType::Cname.code(),
            RData::Mx { .. } => RecordType::Mx.code(),
            RData::Txt(_) => RecordType::Txt.code(),
            RData::Soa { .. } => RecordType::Soa.code(),
            RData::Other(t, _) => *t,
        }
    }
}

/// One resource record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Owner name.
    pub name: Name,
    /// Class, normally [`CLASS_IN`].
    pub class: u16,
    /// Time to live in seconds.
    pub ttl: u32,
    /// The typed data.
    pub data: RData,
}

/// One question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// The name asked for.
    pub name: Name,
    /// The type code asked for.
    pub qtype: u16,
    /// The class asked for.
    pub qclass: u16,
}

/// A DNS message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    /// Transaction id.
    pub id: u16,
    /// The flag word, including opcode and response code.
    pub flags: u16,
    /// Question section.
    pub questions: Vec<Question>,
    /// Answer section.
    pub answers: Vec<Record>,
    /// Authority section.
    pub authority: Vec<Record>,
    /// Additional section.
    pub additional: Vec<Record>,
}

impl Message {
    /// Returns the opcode.
    pub fn opcode(&self) -> u16 {
        (self.flags >> 11) & 0x0f
    }

    /// Returns true for a response.
    pub fn is_response(&self) -> bool {
        self.flags & FLAG_QR != 0
    }

    /// Returns true when the TC flag is set.
    pub fn is_truncated(&self) -> bool {
        self.flags & FLAG_TC != 0
    }

    /// Returns the response code, when the module knows it.
    pub fn rcode(&self) -> Option<Rcode> {
        Rcode::from_flags(self.flags)
    }

    /// Parses a message from untrusted bytes. Bytes after the last record
    /// are ignored, as most resolvers ignore them.
    pub fn parse(data: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader { data, pos: 0 };
        let id = r.u16("header")?;
        let flags = r.u16("header")?;
        let qd = r.u16("header")?;
        let an = r.u16("header")?;
        let ns = r.u16("header")?;
        let ar = r.u16("header")?;
        let mut m = Message {
            id,
            flags,
            ..Message::default()
        };
        for _ in 0..qd {
            let name = r.name()?;
            let qtype = r.u16("question")?;
            let qclass = r.u16("question")?;
            m.questions.push(Question {
                name,
                qtype,
                qclass,
            });
        }
        for (count, section) in [
            (an, &mut m.answers),
            (ns, &mut m.authority),
            (ar, &mut m.additional),
        ] {
            for _ in 0..count {
                section.push(r.record()?);
            }
        }
        Ok(m)
    }

    /// Encodes the message with name compression.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut out = Vec::with_capacity(UDP_LIMIT);
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// Encodes the message after the two byte length prefix of TCP.
    pub fn encode_tcp(&self) -> Result<Vec<u8>, WireError> {
        let mut out = Vec::with_capacity(UDP_LIMIT);
        out.extend_from_slice(&[0, 0]);
        self.encode_into(&mut out)?;
        let len = out.len() - 2;
        let prefix = u16::try_from(len).map_err(|_| WireError::TooLong { len })?;
        out[..2].copy_from_slice(&prefix.to_be_bytes());
        Ok(out)
    }

    /// Appends the encoded message to `out`. Compression pointers count
    /// from the length of `out` at the call, so a caller may write a prefix
    /// first.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), WireError> {
        let mut w = Writer {
            base: out.len(),
            out,
            names: Vec::new(),
        };
        let count = |n: usize| u16::try_from(n).map_err(|_| WireError::TooLong { len: n });
        w.u16(self.id);
        w.u16(self.flags);
        w.u16(count(self.questions.len())?);
        w.u16(count(self.answers.len())?);
        w.u16(count(self.authority.len())?);
        w.u16(count(self.additional.len())?);
        for q in &self.questions {
            w.name(&q.name);
            w.u16(q.qtype);
            w.u16(q.qclass);
        }
        for r in self
            .answers
            .iter()
            .chain(&self.authority)
            .chain(&self.additional)
        {
            w.record(r)?;
        }
        let len = w.out.len() - w.base;
        if len > TCP_LIMIT {
            return Err(WireError::TooLong { len });
        }
        Ok(())
    }
}

/// A bounds checked cursor over untrusted bytes.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, field: &'static str) -> Result<&'a [u8], WireError> {
        let data: &'a [u8] = self.data;
        let s = self
            .pos
            .checked_add(n)
            .and_then(|end| data.get(self.pos..end))
            .ok_or(WireError::Truncated {
                offset: self.pos,
                field,
            })?;
        self.pos += n;
        Ok(s)
    }

    fn u16(&mut self, field: &'static str) -> Result<u16, WireError> {
        let b = self.take(2, field)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, WireError> {
        let b = self.take(4, field)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads a possibly compressed name at the cursor.
    fn name(&mut self) -> Result<Name, WireError> {
        let (name, end) = read_name(self.data, self.pos)?;
        self.pos = end;
        Ok(name)
    }

    fn record(&mut self) -> Result<Record, WireError> {
        let name = self.name()?;
        let rtype = self.u16("record header")?;
        let class = self.u16("record header")?;
        let ttl = self.u32("record header")?;
        let len = usize::from(self.u16("record header")?);
        let start = self.pos;
        let rdata = self.take(len, "record data")?;
        let bad = WireError::BadRecordData {
            rtype,
            offset: start,
        };
        // Names inside record data may point anywhere before them, so they
        // are read against the whole message and must end inside the data.
        let name_in = |at: usize| -> Result<(Name, usize), WireError> {
            let (n, end) = read_name(self.data, at)?;
            if end > start + len {
                return Err(bad.clone());
            }
            Ok((n, end))
        };
        let data = match RecordType::from_code(rtype) {
            Some(RecordType::A) => {
                let b: [u8; 4] = rdata.try_into().map_err(|_| bad.clone())?;
                RData::A(Ipv4Addr::from(b))
            }
            Some(RecordType::Aaaa) => {
                let b: [u8; 16] = rdata.try_into().map_err(|_| bad.clone())?;
                RData::Aaaa(Ipv6Addr::from(b))
            }
            Some(t @ (RecordType::Ns | RecordType::Cname)) => {
                let (n, end) = name_in(start)?;
                if end != start + len {
                    return Err(bad);
                }
                if t == RecordType::Ns {
                    RData::Ns(n)
                } else {
                    RData::Cname(n)
                }
            }
            Some(RecordType::Mx) => {
                let p = rdata.get(..2).ok_or(bad.clone())?;
                let preference = u16::from_be_bytes([p[0], p[1]]);
                let (exchange, end) = name_in(start + 2)?;
                if end != start + len {
                    return Err(bad);
                }
                RData::Mx {
                    preference,
                    exchange,
                }
            }
            Some(RecordType::Txt) => {
                let mut strings = Vec::new();
                let mut rest = rdata;
                while let Some((&n, tail)) = rest.split_first() {
                    let s = tail.get(..usize::from(n)).ok_or(bad.clone())?;
                    strings.push(s.to_vec());
                    rest = &tail[usize::from(n)..];
                }
                if strings.is_empty() {
                    return Err(bad);
                }
                RData::Txt(strings)
            }
            Some(RecordType::Soa) => {
                let (mname, end) = name_in(start)?;
                let (rname, end) = name_in(end)?;
                let tail = self.data.get(end..start + len).ok_or(bad.clone())?;
                if tail.len() != 20 {
                    return Err(bad);
                }
                let n =
                    |i: usize| u32::from_be_bytes([tail[i], tail[i + 1], tail[i + 2], tail[i + 3]]);
                RData::Soa {
                    mname,
                    rname,
                    serial: n(0),
                    refresh: n(4),
                    retry: n(8),
                    expire: n(12),
                    minimum: n(16),
                }
            }
            None => RData::Other(rtype, rdata.to_vec()),
        };
        Ok(Record {
            name,
            class,
            ttl,
            data,
        })
    }
}

/// Reads a name at `at` and returns it with the offset after its last
/// in-line byte. Every pointer must point before the label that holds it,
/// so a malicious message cannot loop.
fn read_name(data: &[u8], at: usize) -> Result<(Name, usize), WireError> {
    let mut labels = Vec::new();
    let mut pos = at;
    let mut end = None;
    let mut wire = 1;
    let mut floor = at;
    loop {
        let len = *data.get(pos).ok_or(WireError::Truncated {
            offset: pos,
            field: "name",
        })?;
        match len & 0xc0 {
            0x00 => {
                if len == 0 {
                    let end = end.unwrap_or(pos + 1);
                    return Ok((Name { labels }, end));
                }
                let n = usize::from(len);
                let label = data.get(pos + 1..pos + 1 + n).ok_or(WireError::Truncated {
                    offset: pos,
                    field: "name",
                })?;
                wire += n + 1;
                if wire > MAX_NAME {
                    return Err(WireError::NameTooLong { offset: at });
                }
                labels.push(label.to_vec());
                pos += n + 1;
            }
            0xc0 => {
                let lo = *data.get(pos + 1).ok_or(WireError::Truncated {
                    offset: pos,
                    field: "name",
                })?;
                let target = usize::from(u16::from_be_bytes([len & 0x3f, lo]));
                if target >= floor {
                    return Err(WireError::BadPointer {
                        offset: pos,
                        target,
                    });
                }
                end.get_or_insert(pos + 2);
                floor = target;
                pos = target;
            }
            _ => {
                return Err(WireError::BadLabel {
                    offset: pos,
                    bits: len,
                })
            }
        }
    }
}

/// Writes a message and remembers the offsets of written names for
/// compression.
struct Writer<'a> {
    out: &'a mut Vec<u8>,
    base: usize,
    /// Lowercase suffixes already written and their offsets from `base`.
    names: Vec<(Vec<Vec<u8>>, u16)>,
}

impl Writer<'_> {
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }

    fn name(&mut self, name: &Name) {
        let labels = name.labels();
        for i in 0..labels.len() {
            let suffix: Vec<Vec<u8>> = labels[i..].iter().map(|l| l.to_ascii_lowercase()).collect();
            if let Some((_, off)) = self.names.iter().find(|(s, _)| *s == suffix) {
                self.u16(0xc000 | off);
                return;
            }
            let off = self.out.len() - self.base;
            // Pointers hold 14 bits, so later names cannot point past 16 KiB.
            if let Ok(off) = u16::try_from(off) {
                if off < 0x4000 {
                    self.names.push((suffix, off));
                }
            }
            // Name::parse and read_name bound labels to 63 bytes.
            self.out.push(labels[i].len() as u8);
            self.out.extend_from_slice(&labels[i]);
        }
        self.out.push(0);
    }

    fn record(&mut self, r: &Record) -> Result<(), WireError> {
        self.name(&r.name);
        self.u16(r.data.type_code());
        self.u16(r.class);
        self.u32(r.ttl);
        let len_at = self.out.len();
        self.u16(0);
        match &r.data {
            RData::A(a) => self.out.extend_from_slice(&a.octets()),
            RData::Aaaa(a) => self.out.extend_from_slice(&a.octets()),
            RData::Ns(n) | RData::Cname(n) => self.name(n),
            RData::Mx {
                preference,
                exchange,
            } => {
                self.u16(*preference);
                self.name(exchange);
            }
            RData::Txt(strings) => {
                for s in strings {
                    let n = u8::try_from(s.len()).map_err(|_| WireError::BadRecordData {
                        rtype: RecordType::Txt.code(),
                        offset: len_at,
                    })?;
                    self.out.push(n);
                    self.out.extend_from_slice(s);
                }
            }
            RData::Soa {
                mname,
                rname,
                serial,
                refresh,
                retry,
                expire,
                minimum,
            } => {
                self.name(mname);
                self.name(rname);
                for v in [serial, refresh, retry, expire, minimum] {
                    self.u32(*v);
                }
            }
            RData::Other(_, data) => self.out.extend_from_slice(data),
        }
        let len = self.out.len() - len_at - 2;
        let len16 = u16::try_from(len).map_err(|_| WireError::TooLong { len })?;
        self.out[len_at..len_at + 2].copy_from_slice(&len16.to_be_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, RngCore};

    /// The query of the contract example `lookup.yaml`.
    const QUERY: &str = "12340100000100000000000003777777076578616d706c6504746573740000010001";
    /// The response of the contract example `lookup.yaml`.
    const RESPONSE: &str = "12348180000100010000000003777777076578616d706c6504746573740000010001c00c000100010000012c0004c0000201";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn www() -> Name {
        Name::parse("www.example.test").unwrap()
    }

    #[test]
    fn parses_the_contract_example_query_and_response() {
        let q = Message::parse(&unhex(QUERY)).unwrap();
        assert_eq!(q.id, 0x1234);
        assert!(!q.is_response());
        assert_eq!(q.questions[0].name, www());
        assert_eq!(q.questions[0].qtype, 1);
        let r = Message::parse(&unhex(RESPONSE)).unwrap();
        assert_eq!(r.rcode(), Some(Rcode::NoError));
        assert_eq!(r.answers[0].name, www());
        assert_eq!(r.answers[0].ttl, 300);
        assert_eq!(r.answers[0].data, RData::A(Ipv4Addr::new(192, 0, 2, 1)));
    }

    #[test]
    fn encodes_the_contract_example_response_byte_for_byte() {
        let m = Message {
            id: 0x1234,
            flags: 0x8180,
            questions: vec![Question {
                name: www(),
                qtype: 1,
                qclass: CLASS_IN,
            }],
            answers: vec![Record {
                name: www(),
                class: CLASS_IN,
                ttl: 300,
                data: RData::A(Ipv4Addr::new(192, 0, 2, 1)),
            }],
            ..Message::default()
        };
        assert_eq!(m.encode().unwrap(), unhex(RESPONSE));
        let tcp = m.encode_tcp().unwrap();
        assert_eq!(&tcp[..2], &(unhex(RESPONSE).len() as u16).to_be_bytes());
        assert_eq!(&tcp[2..], &unhex(RESPONSE)[..]);
    }

    #[test]
    fn every_record_type_round_trips_with_compression() {
        let origin = Name::parse("example.test").unwrap();
        let rec = |name: &Name, data| Record {
            name: name.clone(),
            class: CLASS_IN,
            ttl: 60,
            data,
        };
        let mail = Name::parse("mail.example.test").unwrap();
        let m = Message {
            id: 7,
            flags: FLAG_QR | FLAG_AA,
            questions: vec![Question {
                name: origin.clone(),
                qtype: 255,
                qclass: CLASS_IN,
            }],
            answers: vec![
                rec(&origin, RData::A(Ipv4Addr::new(192, 0, 2, 1))),
                rec(&origin, RData::Aaaa("2001:db8::1".parse().unwrap())),
                rec(&www(), RData::Cname(origin.clone())),
                rec(
                    &origin,
                    RData::Mx {
                        preference: 10,
                        exchange: mail.clone(),
                    },
                ),
                rec(&origin, RData::Txt(vec![b"v=spf1 -all".to_vec(), vec![]])),
                rec(&origin, RData::Ns(Name::parse("ns1.example.test").unwrap())),
                rec(&origin, RData::Other(99, vec![1, 2, 3])),
            ],
            authority: vec![rec(
                &origin,
                RData::Soa {
                    mname: Name::parse("ns1.example.test").unwrap(),
                    rname: Name::parse("hostmaster.example.test").unwrap(),
                    serial: 1,
                    refresh: 2,
                    retry: 3,
                    expire: 4,
                    minimum: 5,
                },
            )],
            additional: vec![rec(&mail, RData::A(Ipv4Addr::new(192, 0, 2, 25)))],
        };
        let bytes = m.encode().unwrap();
        assert_eq!(Message::parse(&bytes).unwrap(), m);
        // The origin appears once in full and as a pointer everywhere else.
        let full = b"\x07example\x04test\x00";
        let copies = bytes.windows(full.len()).filter(|w| w == full).count();
        assert_eq!(copies, 1);
    }

    #[test]
    fn compression_ignores_case_and_the_question_keeps_it() {
        let mixed = Name::parse("WwW.Example.TEST").unwrap();
        let m = Message {
            questions: vec![Question {
                name: mixed.clone(),
                qtype: 1,
                qclass: 1,
            }],
            answers: vec![Record {
                name: www(),
                class: 1,
                ttl: 1,
                data: RData::A(Ipv4Addr::LOCALHOST),
            }],
            ..Message::default()
        };
        let bytes = m.encode().unwrap();
        assert_eq!(&bytes[HEADER_LEN..HEADER_LEN + 4], b"\x03WwW");
        let back = Message::parse(&bytes).unwrap();
        assert_eq!(back.questions[0].name, mixed);
        assert!(back.answers[0].name.eq_ignore_case(&www()));
    }

    #[test]
    fn rejects_forward_and_self_pointers_and_reserved_labels() {
        let mut q = unhex(QUERY);
        // Replace the question name by a pointer to itself.
        q.truncate(HEADER_LEN);
        q.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1]);
        assert!(matches!(
            Message::parse(&q),
            Err(WireError::BadPointer { offset: 12, .. })
        ));
        q[HEADER_LEN] = 0x80;
        assert!(matches!(
            Message::parse(&q),
            Err(WireError::BadLabel { offset: 12, .. })
        ));
    }

    #[test]
    fn rejects_truncated_messages_at_every_length() {
        let r = unhex(RESPONSE);
        for n in 0..r.len() {
            assert!(Message::parse(&r[..n]).is_err(), "prefix of {n} bytes");
        }
    }

    #[test]
    fn rejects_overlong_names() {
        let long = vec!["a".repeat(63); 4].join(".");
        assert_eq!(
            Name::parse(&long),
            Err(NameError::TooLong(long.clone())),
            "four 63 byte labels need 257 bytes"
        );
        assert!(matches!(
            Name::parse(&"a".repeat(64)),
            Err(NameError::LabelTooLong(_))
        ));
        assert!(matches!(Name::parse("a..b"), Err(NameError::EmptyLabel(_))));
        assert!(matches!(Name::parse("a b"), Err(NameError::BadByte(_))));
        assert_eq!(Name::parse(".").unwrap(), Name::root());
        assert_eq!(Name::parse("a.b.").unwrap(), Name::parse("a.b").unwrap());
    }

    #[test]
    fn names_display_with_escapes_and_compare_within_zones() {
        let n = Name {
            labels: vec![b"a.b".to_vec(), vec![0x01], b"test".to_vec()],
        };
        assert_eq!(n.to_string(), "a\\.b.\\001.test");
        assert!(www().is_within(&Name::parse("EXAMPLE.test").unwrap()));
        assert!(!www().is_within(&Name::parse("other.test").unwrap()));
        assert!(www().is_within(&Name::root()));
        assert_eq!(www().key(), "www.example.test");
    }

    #[test]
    fn random_bytes_never_panic_the_parser() {
        let mut rng = rand_pcg::Pcg64Mcg::new(42);
        let seed = unhex(RESPONSE);
        for _ in 0..20_000 {
            let mut b = if rng.gen_bool(0.5) {
                seed.clone()
            } else {
                let mut v = vec![0u8; rng.gen_range(0..64)];
                rng.fill_bytes(&mut v);
                v
            };
            for _ in 0..rng.gen_range(0..4) {
                if !b.is_empty() {
                    let i = rng.gen_range(0..b.len());
                    b[i] = rng.gen();
                }
            }
            if let Ok(m) = Message::parse(&b) {
                // Whatever parses encodes again.
                let _ = m.encode();
            }
        }
    }

    #[test]
    fn record_types_parse_and_print() {
        for t in RecordType::ALL {
            assert_eq!(RecordType::parse(&t.as_str().to_lowercase()), Some(t));
            assert_eq!(RecordType::from_code(t.code()), Some(t));
        }
        assert_eq!(RecordType::parse("SRV"), None);
    }
}
