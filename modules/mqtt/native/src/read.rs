// SPDX-License-Identifier: GPL-3.0-only
//! Readers of MQTT control packets.
//!
//! [`Assembler`] collects one packet from the pieces that a flow reads, and
//! [`parse`] checks a complete packet against the standard and returns its
//! fields. The reader is strict on purpose: a packet that breaks a rule of the
//! standard fails the session of the side that reads it, so a scenario whose
//! two sides disagree cannot pass unnoticed.

use crate::wire::{
    err, get_varint, Kind, PropType, Result, Version, WireError, MAX_REMAINING_LENGTH, PROPERTIES,
};

/// The largest body that the assembler accepts. A flow keeps the bytes of its
/// last `expect`, which the interpreter limits to 16 MiB, so the assembler
/// stops below that.
pub const MAX_BODY: u32 = 16 * 1024 * 1024 - 8;

/// The value of a property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropValue {
    /// One byte.
    Byte(u8),
    /// Two byte integer.
    U16(u16),
    /// Four byte integer.
    U32(u32),
    /// Variable byte integer.
    Varint(u32),
    /// UTF-8 string.
    Str(String),
    /// Binary data.
    Bin(Vec<u8>),
    /// A pair of UTF-8 strings.
    Pair(String, String),
}

/// One topic filter of a SUBSCRIBE or UNSUBSCRIBE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// The topic filter.
    pub filter: String,
    /// The subscription options byte, or 0 in an UNSUBSCRIBE.
    pub options: u8,
}

impl Filter {
    /// The maximum QoS that the filter requests.
    pub fn qos(&self) -> u8 {
        self.options & 0b11
    }
}

/// The will message of a CONNECT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WillMessage {
    /// Topic.
    pub topic: String,
    /// Payload.
    pub payload: Vec<u8>,
    /// QoS.
    pub qos: u8,
    /// Retain flag.
    pub retain: bool,
}

/// A parsed control packet. Fields that a kind does not carry keep their
/// defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    /// The kind.
    pub kind: Kind,
    /// The flags of the fixed header.
    pub flags: u8,
    /// The protocol version, which a CONNECT names and the reader is given
    /// for every other packet.
    pub version: Version,
    /// The packet identifier of a packet that carries one.
    pub packet_id: Option<u16>,
    /// DUP flag of a PUBLISH.
    pub dup: bool,
    /// QoS of a PUBLISH.
    pub qos: u8,
    /// RETAIN flag of a PUBLISH.
    pub retain: bool,
    /// Topic name of a PUBLISH.
    pub topic: String,
    /// Payload of a PUBLISH.
    pub payload: Vec<u8>,
    /// Client identifier of a CONNECT.
    pub client_id: String,
    /// Keep alive of a CONNECT, in seconds.
    pub keepalive: u16,
    /// Clean Session or Clean Start flag of a CONNECT.
    pub clean: bool,
    /// User name of a CONNECT.
    pub user: Option<String>,
    /// Password of a CONNECT.
    pub password: Option<Vec<u8>>,
    /// Will message of a CONNECT.
    pub will: Option<WillMessage>,
    /// Session Present flag of a CONNACK.
    pub session_present: bool,
    /// Return code of a CONNACK, or reason code of an acknowledgment or a
    /// DISCONNECT.
    pub code: u8,
    /// Codes of a SUBACK or an UNSUBACK.
    pub codes: Vec<u8>,
    /// Filters of a SUBSCRIBE or an UNSUBSCRIBE.
    pub filters: Vec<Filter>,
    /// Properties of MQTT 5.0, in packet order.
    pub props: Vec<(u8, PropValue)>,
}

impl Packet {
    fn new(kind: Kind, flags: u8, version: Version) -> Packet {
        Packet {
            kind,
            flags,
            version,
            packet_id: None,
            dup: false,
            qos: 0,
            retain: false,
            topic: String::new(),
            payload: Vec::new(),
            client_id: String::new(),
            keepalive: 0,
            clean: false,
            user: None,
            password: None,
            will: None,
            session_present: false,
            code: 0,
            codes: Vec::new(),
            filters: Vec::new(),
            props: Vec::new(),
        }
    }

    /// The first property with a given snake case name.
    pub fn property(&self, name: &str) -> Option<&PropValue> {
        let id = PROPERTIES.iter().find(|(_, n, _)| *n == name)?.0;
        self.props.iter().find(|(i, _)| *i == id).map(|(_, v)| v)
    }
}

// ----------------------------------------------------------------------------
// Assembling a packet from the pieces that a flow reads.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Type,
    Length,
    Body(u32),
    Done,
}

/// Collects one control packet from the reads of a flow.
///
/// A flow reads a packet with one `expect` of one byte, which is the type byte,
/// and then with `expect` actions whose size is `mqtt.feed(last)`. Each call
/// hands the bytes of the read before to the assembler, and the assembler
/// returns how many bytes the next read takes: one byte for each byte of the
/// Remaining Length, then the body, then 0. The Remaining Length takes up to
/// four bytes, so a flow runs six of these reads for every packet, and the
/// reads after the end of the packet take 0 bytes.
#[derive(Clone, Debug, Default)]
pub struct Assembler {
    buf: Vec<u8>,
    phase: Phase,
    length_bytes: usize,
}

impl Assembler {
    /// Takes the bytes of the last read and returns the size of the next.
    pub fn feed(&mut self, data: &[u8]) -> Result<usize> {
        match self.phase {
            Phase::Type => {
                if data.len() != 1 {
                    return err(format!(
                        "a packet starts with one type byte, and the last read holds {} bytes",
                        data.len()
                    ));
                }
                self.buf.clear();
                self.buf.push(data[0]);
                self.length_bytes = 0;
                self.phase = Phase::Length;
                Ok(1)
            }
            Phase::Length => {
                if data.len() != 1 {
                    return err(format!(
                        "the Remaining Length is read one byte at a time, and the last read holds {} bytes",
                        data.len()
                    ));
                }
                self.buf.push(data[0]);
                self.length_bytes += 1;
                if data[0] & 0x80 != 0 {
                    if self.length_bytes == 4 {
                        return err("the Remaining Length is longer than four bytes");
                    }
                    return Ok(1);
                }
                let (n, _) = get_varint(&self.buf[1..])?;
                if n > MAX_BODY.min(MAX_REMAINING_LENGTH) {
                    return err(format!(
                        "a packet body of {n} bytes exceeds the limit of {MAX_BODY} bytes"
                    ));
                }
                if n == 0 {
                    self.phase = Phase::Done;
                    Ok(0)
                } else {
                    self.phase = Phase::Body(n);
                    Ok(n as usize)
                }
            }
            Phase::Body(n) => {
                if data.len() != n as usize {
                    return err(format!(
                        "the body takes {n} bytes, and the last read holds {}",
                        data.len()
                    ));
                }
                self.buf.extend_from_slice(data);
                self.phase = Phase::Done;
                Ok(0)
            }
            Phase::Done => {
                if data.is_empty() {
                    Ok(0)
                } else {
                    err("bytes follow a complete packet that no recv has taken")
                }
            }
        }
    }

    /// Returns the complete packet and starts over.
    pub fn take(&mut self) -> Result<Vec<u8>> {
        if self.phase != Phase::Done {
            return err("the packet is not complete, so a read is missing before recv");
        }
        self.phase = Phase::Type;
        Ok(std::mem::take(&mut self.buf))
    }
}

// ----------------------------------------------------------------------------
// Parsing a complete packet.

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.b.len());
        let Some(end) = end else {
            return err(format!(
                "the packet ends {} bytes short",
                self.at + n - self.b.len()
            ));
        };
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn varint(&mut self) -> Result<u32> {
        let (v, n) = get_varint(&self.b[self.at..])?;
        self.at += n;
        Ok(v)
    }

    fn bin(&mut self) -> Result<Vec<u8>> {
        let n = usize::from(self.u16()?);
        Ok(self.take(n)?.to_vec())
    }

    fn str(&mut self) -> Result<String> {
        let raw = self.bin()?;
        let s = String::from_utf8(raw).map_err(|_| WireError::new("a string is not valid UTF-8"))?;
        if s.contains('\0') {
            return err("a string contains the null character");
        }
        Ok(s)
    }

    fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.at..];
        self.at = self.b.len();
        s
    }

    fn done(&self, what: &str) -> Result<()> {
        if self.at == self.b.len() {
            Ok(())
        } else {
            err(format!(
                "{} unexpected bytes follow the end of the {what}",
                self.b.len() - self.at
            ))
        }
    }
}

fn read_props(c: &mut Cursor<'_>) -> Result<Vec<(u8, PropValue)>> {
    let n = c.varint()? as usize;
    let mut inner = Cursor {
        b: c.take(n)?,
        at: 0,
    };
    let mut props: Vec<(u8, PropValue)> = Vec::new();
    while inner.at < inner.b.len() {
        let id = inner.u8()?;
        let Some((_, name, ty)) = PROPERTIES.iter().find(|(i, _, _)| *i == id) else {
            return err(format!("0x{id:02x} is not a property identifier"));
        };
        let value = match ty {
            PropType::Byte => PropValue::Byte(inner.u8()?),
            PropType::U16 => PropValue::U16(inner.u16()?),
            PropType::U32 => PropValue::U32(inner.u32()?),
            PropType::Varint => PropValue::Varint(inner.varint()?),
            PropType::Str => PropValue::Str(inner.str()?),
            PropType::Bin => PropValue::Bin(inner.bin()?),
            PropType::Pair => {
                let name = inner.str()?;
                PropValue::Pair(name, inner.str()?)
            }
        };
        let repeatable = matches!(id, 0x26 | 0x0B);
        if !repeatable && props.iter().any(|(i, _)| *i == id) {
            return err(format!("the property {name} occurs twice"));
        }
        props.push((id, value));
    }
    Ok(props)
}

fn check_flags(kind: Kind, flags: u8) -> Result<()> {
    match kind.fixed_flags() {
        Some(want) if want != flags => err(format!(
            "the flags of a {} must be {want:#06b}, not {flags:#06b}",
            kind.name().to_ascii_uppercase()
        )),
        _ => Ok(()),
    }
}

/// Parses one complete packet, the fixed header included. `version` is the
/// version of the session, which a CONNECT overrides with its own level.
pub fn parse(version: Version, packet: &[u8]) -> Result<Packet> {
    let Some(&first) = packet.first() else {
        return err("the packet is empty");
    };
    let Some(kind) = Kind::from_type(first >> 4) else {
        return err(format!(
            "{} is not a packet type that the module reads",
            first >> 4
        ));
    };
    let flags = first & 0x0F;
    check_flags(kind, flags)?;
    let (remaining, n) = get_varint(&packet[1..])?;
    let body = &packet[1 + n..];
    if body.len() != remaining as usize {
        return err(format!(
            "the Remaining Length is {remaining}, and {} bytes follow it",
            body.len()
        ));
    }
    let mut c = Cursor { b: body, at: 0 };
    let mut p = Packet::new(kind, flags, version);
    match kind {
        Kind::Connect => parse_connect(&mut c, &mut p)?,
        Kind::Connack => {
            let ack_flags = c.u8()?;
            if ack_flags > 1 {
                return err("the reserved bits of the CONNACK flags are not 0");
            }
            p.session_present = ack_flags == 1;
            p.code = c.u8()?;
            if version.is_v5() {
                p.props = read_props(&mut c)?;
            } else if p.code > 5 {
                return err(format!("{} is not a connect return code", p.code));
            }
        }
        Kind::Publish => parse_publish(&mut c, &mut p)?,
        Kind::Puback | Kind::Pubrec | Kind::Pubrel | Kind::Pubcomp => {
            p.packet_id = Some(nonzero_id(c.u16()?)?);
            if version.is_v5() && c.at < c.b.len() {
                p.code = c.u8()?;
                if c.at < c.b.len() {
                    p.props = read_props(&mut c)?;
                }
            }
        }
        Kind::Subscribe => {
            p.packet_id = Some(nonzero_id(c.u16()?)?);
            if version.is_v5() {
                p.props = read_props(&mut c)?;
            }
            while c.at < c.b.len() {
                let filter = c.str()?;
                let options = c.u8()?;
                let reserved = if version.is_v5() { 0b1100_0000 } else { 0b1111_1100 };
                if options & reserved != 0 || options & 0b11 == 3 || (options >> 4) & 0b11 == 3 {
                    return err(format!("the subscription options {options:#04x} are not valid"));
                }
                p.filters.push(Filter { filter, options });
            }
            if p.filters.is_empty() {
                return err("a SUBSCRIBE holds no topic filter");
            }
        }
        Kind::Suback => {
            p.packet_id = Some(nonzero_id(c.u16()?)?);
            if version.is_v5() {
                p.props = read_props(&mut c)?;
            }
            p.codes = c.rest().to_vec();
            if p.codes.is_empty() {
                return err("a SUBACK holds no code");
            }
        }
        Kind::Unsubscribe => {
            p.packet_id = Some(nonzero_id(c.u16()?)?);
            if version.is_v5() {
                p.props = read_props(&mut c)?;
            }
            while c.at < c.b.len() {
                p.filters.push(Filter {
                    filter: c.str()?,
                    options: 0,
                });
            }
            if p.filters.is_empty() {
                return err("an UNSUBSCRIBE holds no topic filter");
            }
        }
        Kind::Unsuback => {
            p.packet_id = Some(nonzero_id(c.u16()?)?);
            if version.is_v5() {
                p.props = read_props(&mut c)?;
                p.codes = c.rest().to_vec();
                if p.codes.is_empty() {
                    return err("an UNSUBACK of MQTT 5.0 holds no code");
                }
            }
        }
        Kind::Pingreq | Kind::Pingresp => {}
        Kind::Disconnect => {
            if version.is_v5() && c.at < c.b.len() {
                p.code = c.u8()?;
                if c.at < c.b.len() {
                    p.props = read_props(&mut c)?;
                }
            }
        }
    }
    c.done(&format!("{} packet", kind.name().to_ascii_uppercase()))?;
    Ok(p)
}

fn nonzero_id(id: u16) -> Result<u16> {
    if id == 0 {
        err("a packet identifier is 0")
    } else {
        Ok(id)
    }
}

fn parse_connect(c: &mut Cursor<'_>, p: &mut Packet) -> Result<()> {
    let name = c.str()?;
    if name != "MQTT" {
        return err(format!(
            "the protocol name is '{name}', and the module reads 'MQTT'"
        ));
    }
    let level = c.u8()?;
    let Some(version) = Version::from_level(level) else {
        return err(format!(
            "protocol level {level} is not 4 (3.1.1) or 5 (5.0)"
        ));
    };
    p.version = version;
    let flags = c.u8()?;
    if flags & 1 != 0 {
        return err("the reserved bit of the connect flags is set");
    }
    p.clean = flags & 0x02 != 0;
    let will = flags & 0x04 != 0;
    let will_qos = (flags >> 3) & 3;
    let will_retain = flags & 0x20 != 0;
    if will_qos == 3 {
        return err("the Will QoS is 3");
    }
    if !will && (will_qos != 0 || will_retain) {
        return err("Will QoS and Will Retain are set without the Will Flag");
    }
    let user = flags & 0x80 != 0;
    let password = flags & 0x40 != 0;
    if version == Version::V311 && password && !user {
        return err("MQTT 3.1.1 sets the password flag without the user name flag");
    }
    p.keepalive = c.u16()?;
    if version.is_v5() {
        p.props = read_props(c)?;
    }
    p.client_id = c.str()?;
    if will {
        if version.is_v5() {
            read_props(c)?;
        }
        p.will = Some(WillMessage {
            topic: c.str()?,
            payload: c.bin()?,
            qos: will_qos,
            retain: will_retain,
        });
    }
    if user {
        p.user = Some(c.str()?);
    }
    if password {
        p.password = Some(c.bin()?);
    }
    Ok(())
}

fn parse_publish(c: &mut Cursor<'_>, p: &mut Packet) -> Result<()> {
    p.dup = p.flags & 0b1000 != 0;
    p.qos = (p.flags >> 1) & 0b11;
    p.retain = p.flags & 1 != 0;
    if p.qos == 3 {
        return err("a PUBLISH has QoS 3");
    }
    if p.dup && p.qos == 0 {
        return err("a PUBLISH of QoS 0 sets DUP");
    }
    p.topic = c.str()?;
    if p.topic.contains(['+', '#']) {
        return err("the topic name of a PUBLISH holds a wildcard");
    }
    if p.qos > 0 {
        p.packet_id = Some(nonzero_id(c.u16()?)?);
    }
    if p.version.is_v5() {
        p.props = read_props(c)?;
    }
    if p.topic.is_empty() && p.property("topic_alias").is_none() {
        return err("the topic name of a PUBLISH is empty without a topic alias");
    }
    p.payload = c.rest().to_vec();
    Ok(())
}

// ----------------------------------------------------------------------------
// Fields of a parsed packet, as text for a flow.

fn prop_text(v: &PropValue) -> Vec<u8> {
    match v {
        PropValue::Byte(b) => b.to_string().into_bytes(),
        PropValue::U16(n) => n.to_string().into_bytes(),
        PropValue::U32(n) | PropValue::Varint(n) => n.to_string().into_bytes(),
        PropValue::Str(s) => s.clone().into_bytes(),
        PropValue::Bin(b) => b.clone(),
        PropValue::Pair(n, v) => format!("{n}={v}").into_bytes(),
    }
}

/// The names that [`field`] accepts besides the property names.
pub const FIELDS: &[&str] = &[
    "type",
    "version",
    "flags",
    "packet_id",
    "dup",
    "qos",
    "retain",
    "topic",
    "payload",
    "payload_length",
    "client_id",
    "keepalive",
    "clean",
    "user",
    "password",
    "will_topic",
    "will_payload",
    "session_present",
    "code",
    "codes",
    "filters",
    "requested_qos",
    "filter_count",
];

fn bool_text(b: bool) -> Vec<u8> {
    b.to_string().into_bytes()
}

fn join<T: ToString>(items: impl Iterator<Item = T>, sep: &str) -> Vec<u8> {
    items.map(|i| i.to_string()).collect::<Vec<_>>().join(sep).into_bytes()
}

/// Returns a field of a parsed packet, as the bytes of its text. The names are
/// those of [`FIELDS`] and the snake case names of the properties of MQTT 5.0.
/// A field that the packet does not carry is an error.
pub fn field(p: &Packet, name: &str) -> Result<Vec<u8>> {
    let missing = || WireError::new(format!("a {} packet has no field '{name}'", p.kind.name()));
    let value = match name {
        "type" => p.kind.name().as_bytes().to_vec(),
        "version" => p.version.level().to_string().into_bytes(),
        "flags" => p.flags.to_string().into_bytes(),
        "packet_id" => p.packet_id.ok_or_else(missing)?.to_string().into_bytes(),
        "dup" if p.kind == Kind::Publish => bool_text(p.dup),
        "qos" if p.kind == Kind::Publish => p.qos.to_string().into_bytes(),
        "retain" if p.kind == Kind::Publish => bool_text(p.retain),
        "topic" if p.kind == Kind::Publish => p.topic.clone().into_bytes(),
        "payload" if p.kind == Kind::Publish => p.payload.clone(),
        "payload_length" if p.kind == Kind::Publish => p.payload.len().to_string().into_bytes(),
        "client_id" if p.kind == Kind::Connect => p.client_id.clone().into_bytes(),
        "keepalive" if p.kind == Kind::Connect => p.keepalive.to_string().into_bytes(),
        "clean" if p.kind == Kind::Connect => bool_text(p.clean),
        "user" if p.kind == Kind::Connect => p.user.clone().ok_or_else(missing)?.into_bytes(),
        "password" if p.kind == Kind::Connect => p.password.clone().ok_or_else(missing)?,
        "will_topic" if p.kind == Kind::Connect => {
            p.will.as_ref().ok_or_else(missing)?.topic.clone().into_bytes()
        }
        "will_payload" if p.kind == Kind::Connect => {
            p.will.as_ref().ok_or_else(missing)?.payload.clone()
        }
        "session_present" if p.kind == Kind::Connack => bool_text(p.session_present),
        "code" => p.code.to_string().into_bytes(),
        "codes" if matches!(p.kind, Kind::Suback | Kind::Unsuback) => join(p.codes.iter(), ","),
        "filters" if matches!(p.kind, Kind::Subscribe | Kind::Unsubscribe) => {
            join(p.filters.iter().map(|f| f.filter.as_str()), ",")
        }
        "requested_qos" if p.kind == Kind::Subscribe => join(p.filters.iter().map(Filter::qos), ","),
        "filter_count" if matches!(p.kind, Kind::Subscribe | Kind::Unsubscribe) => {
            p.filters.len().to_string().into_bytes()
        }
        other => match p.property(other) {
            Some(v) => prop_text(v),
            None => return Err(missing()),
        },
    };
    Ok(value)
}
