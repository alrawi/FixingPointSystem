// SPDX-License-Identifier: GPL-3.0-only
//! Wire encoding of the MQTT control packets of versions 3.1.1 and 5.0.
//!
//! The functions follow the OASIS standards "MQTT Version 3.1.1" and "MQTT
//! Version 5.0". Every encoder appends one complete control packet, the fixed
//! header included, to a buffer. The encoders check what the standards require
//! of a sender, such as the fixed flags of PUBREL and SUBSCRIBE, the nonzero
//! packet identifier, the topic name without wildcards, and the reason codes
//! that a packet may carry, so a scenario cannot write a packet that a parser
//! rejects.

use std::fmt;

/// The largest value of the Remaining Length field, four bytes of seven bits.
pub const MAX_REMAINING_LENGTH: u32 = 268_435_455;

/// An error of an encoder or a reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError(String);

impl WireError {
    /// Creates an error with a message.
    pub fn new(message: impl Into<String>) -> Self {
        WireError(message.into())
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WireError {}

/// The result type of the encoders and readers.
pub type Result<T> = std::result::Result<T, WireError>;

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(WireError::new(message))
}

/// The protocol version of a session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Version {
    /// MQTT 3.1.1, protocol level 4.
    #[default]
    V311,
    /// MQTT 5.0, protocol level 5.
    V5,
}

impl Version {
    /// The protocol level that CONNECT carries.
    pub fn level(self) -> u8 {
        match self {
            Version::V311 => 4,
            Version::V5 => 5,
        }
    }

    /// The version of a protocol level, or `None` for a level that the module
    /// does not write.
    pub fn from_level(level: u8) -> Option<Self> {
        match level {
            4 => Some(Version::V311),
            5 => Some(Version::V5),
            _ => None,
        }
    }

    /// True for MQTT 5.0.
    pub fn is_v5(self) -> bool {
        self == Version::V5
    }
}

/// The control packet types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Client request to connect.
    Connect = 1,
    /// Connect acknowledgment.
    Connack = 2,
    /// Publish message.
    Publish = 3,
    /// Publish acknowledgment, QoS 1.
    Puback = 4,
    /// Publish received, QoS 2 part 1.
    Pubrec = 5,
    /// Publish release, QoS 2 part 2.
    Pubrel = 6,
    /// Publish complete, QoS 2 part 3.
    Pubcomp = 7,
    /// Subscribe request.
    Subscribe = 8,
    /// Subscribe acknowledgment.
    Suback = 9,
    /// Unsubscribe request.
    Unsubscribe = 10,
    /// Unsubscribe acknowledgment.
    Unsuback = 11,
    /// Ping request.
    Pingreq = 12,
    /// Ping response.
    Pingresp = 13,
    /// Disconnect notification.
    Disconnect = 14,
}

impl Kind {
    /// Every kind that the module writes and reads.
    pub const ALL: [Kind; 14] = [
        Kind::Connect,
        Kind::Connack,
        Kind::Publish,
        Kind::Puback,
        Kind::Pubrec,
        Kind::Pubrel,
        Kind::Pubcomp,
        Kind::Subscribe,
        Kind::Suback,
        Kind::Unsubscribe,
        Kind::Unsuback,
        Kind::Pingreq,
        Kind::Pingresp,
        Kind::Disconnect,
    ];

    /// The lower case name of the kind.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Connect => "connect",
            Kind::Connack => "connack",
            Kind::Publish => "publish",
            Kind::Puback => "puback",
            Kind::Pubrec => "pubrec",
            Kind::Pubrel => "pubrel",
            Kind::Pubcomp => "pubcomp",
            Kind::Subscribe => "subscribe",
            Kind::Suback => "suback",
            Kind::Unsubscribe => "unsubscribe",
            Kind::Unsuback => "unsuback",
            Kind::Pingreq => "pingreq",
            Kind::Pingresp => "pingresp",
            Kind::Disconnect => "disconnect",
        }
    }

    /// The kind with a given name.
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The kind with a given type number.
    pub fn from_type(t: u8) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| *k as u8 == t)
    }

    /// The flags of the fixed header that the standard fixes for the kind, or
    /// `None` for PUBLISH, whose flags carry DUP, QoS, and RETAIN.
    pub fn fixed_flags(self) -> Option<u8> {
        match self {
            Kind::Publish => None,
            Kind::Pubrel | Kind::Subscribe | Kind::Unsubscribe => Some(0b0010),
            _ => Some(0),
        }
    }
}

// ----------------------------------------------------------------------------
// Variable length integers, strings, and binary data.

/// The number of bytes that `value` takes as a variable byte integer.
pub fn varint_len(value: u32) -> usize {
    match value {
        0..=127 => 1,
        128..=16_383 => 2,
        16_384..=2_097_151 => 3,
        _ => 4,
    }
}

/// Appends `value` as a variable byte integer in the least number of bytes,
/// which is how the standard writes the Remaining Length, the property length,
/// and the subscription identifier.
pub fn put_varint(out: &mut Vec<u8>, value: u32) -> Result<()> {
    if value > MAX_REMAINING_LENGTH {
        return err(format!(
            "{value} does not fit a variable byte integer, the largest value is {MAX_REMAINING_LENGTH}"
        ));
    }
    let mut v = value;
    loop {
        let mut byte = (v % 128) as u8;
        v /= 128;
        if v > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            return Ok(());
        }
    }
}

/// Reads a variable byte integer from the start of `buf` and returns it with
/// the number of bytes it took. The reader rejects an integer of more than four
/// bytes and an integer that does not use the least number of bytes, as the
/// standard demands.
pub fn get_varint(buf: &[u8]) -> Result<(u32, usize)> {
    let mut value = 0u32;
    let mut multiplier = 1u32;
    for i in 0..4 {
        let Some(&byte) = buf.get(i) else {
            return err("a variable byte integer ends before its last byte");
        };
        value += u32::from(byte & 0x7F) * multiplier;
        if byte & 0x80 == 0 {
            if i > 0 && byte == 0 {
                return err("a variable byte integer does not use the least number of bytes");
            }
            return Ok((value, i + 1));
        }
        multiplier *= 128;
    }
    err("a variable byte integer is longer than four bytes")
}

pub(crate) fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub(crate) fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Appends a UTF-8 string with its two byte length. The standard forbids the
/// null character and a string above 65535 bytes.
pub fn put_str(out: &mut Vec<u8>, s: &str) -> Result<()> {
    if s.contains('\0') {
        return err("an MQTT string must not contain the null character");
    }
    put_bin(out, s.as_bytes()).map_err(|_| {
        WireError::new(format!(
            "an MQTT string is limited to 65535 bytes, not {}",
            s.len()
        ))
    })
}

/// Appends binary data with its two byte length.
pub fn put_bin(out: &mut Vec<u8>, b: &[u8]) -> Result<()> {
    let n = u16::try_from(b.len()).map_err(|_| {
        WireError::new(format!(
            "binary data is limited to 65535 bytes, not {}",
            b.len()
        ))
    })?;
    put_u16(out, n);
    out.extend_from_slice(b);
    Ok(())
}

/// Appends a complete packet: the fixed header byte, the Remaining Length, and
/// the body.
fn finish(out: &mut Vec<u8>, kind: Kind, flags: u8, body: &[u8]) -> Result<()> {
    let n = u32::try_from(body.len())
        .ok()
        .filter(|n| *n <= MAX_REMAINING_LENGTH)
        .ok_or_else(|| {
            WireError::new(format!(
                "a packet body of {} bytes exceeds the Remaining Length limit of {MAX_REMAINING_LENGTH}",
                body.len()
            ))
        })?;
    out.push(((kind as u8) << 4) | (flags & 0x0F));
    put_varint(out, n)?;
    out.extend_from_slice(body);
    Ok(())
}

// ----------------------------------------------------------------------------
// Properties of MQTT 5.0.

/// The data type of a property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropType {
    /// One byte.
    Byte,
    /// Two byte integer.
    U16,
    /// Four byte integer.
    U32,
    /// Variable byte integer.
    Varint,
    /// UTF-8 string.
    Str,
    /// Binary data.
    Bin,
    /// A pair of UTF-8 strings.
    Pair,
}

/// The properties of the standard: identifier, snake case name, and type.
pub const PROPERTIES: &[(u8, &str, PropType)] = &[
    (0x01, "payload_format_indicator", PropType::Byte),
    (0x02, "message_expiry_interval", PropType::U32),
    (0x03, "content_type", PropType::Str),
    (0x08, "response_topic", PropType::Str),
    (0x09, "correlation_data", PropType::Bin),
    (0x0B, "subscription_identifier", PropType::Varint),
    (0x11, "session_expiry_interval", PropType::U32),
    (0x12, "assigned_client_identifier", PropType::Str),
    (0x13, "server_keep_alive", PropType::U16),
    (0x15, "authentication_method", PropType::Str),
    (0x16, "authentication_data", PropType::Bin),
    (0x17, "request_problem_information", PropType::Byte),
    (0x18, "will_delay_interval", PropType::U32),
    (0x19, "request_response_information", PropType::Byte),
    (0x1A, "response_information", PropType::Str),
    (0x1C, "server_reference", PropType::Str),
    (0x1F, "reason_string", PropType::Str),
    (0x21, "receive_maximum", PropType::U16),
    (0x22, "topic_alias_maximum", PropType::U16),
    (0x23, "topic_alias", PropType::U16),
    (0x24, "maximum_qos", PropType::Byte),
    (0x25, "retain_available", PropType::Byte),
    (0x26, "user_property", PropType::Pair),
    (0x27, "maximum_packet_size", PropType::U32),
    (0x28, "wildcard_subscription_available", PropType::Byte),
    (0x29, "subscription_identifier_available", PropType::Byte),
    (0x2A, "shared_subscription_available", PropType::Byte),
];

/// The identifiers of the properties that the encoders write.
pub mod prop {
    /// Payload Format Indicator.
    pub const PAYLOAD_FORMAT: u8 = 0x01;
    /// Message Expiry Interval.
    pub const MESSAGE_EXPIRY: u8 = 0x02;
    /// Content Type.
    pub const CONTENT_TYPE: u8 = 0x03;
    /// Response Topic.
    pub const RESPONSE_TOPIC: u8 = 0x08;
    /// Session Expiry Interval.
    pub const SESSION_EXPIRY: u8 = 0x11;
    /// Assigned Client Identifier.
    pub const ASSIGNED_CLIENT_ID: u8 = 0x12;
    /// Server Keep Alive.
    pub const SERVER_KEEP_ALIVE: u8 = 0x13;
    /// Reason String.
    pub const REASON_STRING: u8 = 0x1F;
    /// Receive Maximum.
    pub const RECEIVE_MAXIMUM: u8 = 0x21;
    /// Topic Alias Maximum.
    pub const TOPIC_ALIAS_MAXIMUM: u8 = 0x22;
    /// Maximum QoS.
    pub const MAXIMUM_QOS: u8 = 0x24;
    /// Retain Available.
    pub const RETAIN_AVAILABLE: u8 = 0x25;
    /// User Property.
    pub const USER_PROPERTY: u8 = 0x26;
}

/// A property list under construction.
#[derive(Clone, Debug, Default)]
pub struct Props {
    buf: Vec<u8>,
}

impl Props {
    /// An empty list.
    pub fn new() -> Self {
        Props::default()
    }

    /// True when no property was added.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Adds a one byte property.
    pub fn byte(&mut self, id: u8, v: u8) {
        self.buf.push(id);
        self.buf.push(v);
    }

    /// Adds a two byte property.
    pub fn u16(&mut self, id: u8, v: u16) {
        self.buf.push(id);
        put_u16(&mut self.buf, v);
    }

    /// Adds a four byte property.
    pub fn u32(&mut self, id: u8, v: u32) {
        self.buf.push(id);
        put_u32(&mut self.buf, v);
    }

    /// Adds a string property.
    pub fn str(&mut self, id: u8, v: &str) -> Result<()> {
        self.buf.push(id);
        put_str(&mut self.buf, v)
    }

    /// Adds a user property, a pair of strings.
    pub fn pair(&mut self, name: &str, value: &str) -> Result<()> {
        self.buf.push(prop::USER_PROPERTY);
        put_str(&mut self.buf, name)?;
        put_str(&mut self.buf, value)
    }

    /// Appends the property length and the properties.
    pub fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        let n = u32::try_from(self.buf.len()).map_err(|_| WireError::new("too many properties"))?;
        put_varint(out, n)?;
        out.extend_from_slice(&self.buf);
        Ok(())
    }
}

// ----------------------------------------------------------------------------
// Names and filters.

/// Checks a topic name, which a PUBLISH carries: not empty and without
/// wildcards.
pub fn check_topic_name(topic: &str) -> Result<()> {
    if topic.is_empty() {
        return err("a topic name must not be empty");
    }
    if topic.contains(['+', '#']) {
        return err(format!(
            "the topic name '{topic}' contains a wildcard, which only a topic filter may hold"
        ));
    }
    Ok(())
}

/// Checks a topic filter: not empty, '+' alone in its level, and '#' alone as
/// the last level.
pub fn check_topic_filter(filter: &str) -> Result<()> {
    if filter.is_empty() {
        return err("a topic filter must not be empty");
    }
    let levels: Vec<&str> = filter.split('/').collect();
    for (i, level) in levels.iter().enumerate() {
        let bad = (level.contains('#') && (*level != "#" || i + 1 != levels.len()))
            || (level.contains('+') && *level != "+");
        if bad {
            return err(format!(
                "the topic filter '{filter}' uses a wildcard in a way that the standard forbids"
            ));
        }
    }
    Ok(())
}

fn check_qos(qos: u8) -> Result<()> {
    if qos > 2 {
        return err(format!("QoS must be 0, 1, or 2, not {qos}"));
    }
    Ok(())
}

fn check_id(id: u16) -> Result<()> {
    if id == 0 {
        return err("a packet identifier must not be 0");
    }
    Ok(())
}

fn v5_only(version: Version, what: &str) -> Result<()> {
    if version.is_v5() {
        Ok(())
    } else {
        err(format!("{what} needs MQTT 5.0, and the session uses 3.1.1"))
    }
}

// ----------------------------------------------------------------------------
// CONNECT and CONNACK.

/// The will message of a CONNECT.
#[derive(Clone, Copy, Debug)]
pub struct Will<'a> {
    /// Topic of the will message.
    pub topic: &'a str,
    /// Payload of the will message.
    pub payload: &'a [u8],
    /// QoS of the will message.
    pub qos: u8,
    /// Retain flag of the will message.
    pub retain: bool,
}

/// The fields of a CONNECT packet.
#[derive(Clone, Copy, Debug)]
pub struct Connect<'a> {
    /// Protocol version.
    pub version: Version,
    /// Client identifier.
    pub client_id: &'a str,
    /// Keep alive in seconds.
    pub keepalive: u16,
    /// Clean Session (3.1.1) or Clean Start (5.0).
    pub clean: bool,
    /// User name.
    pub user: Option<&'a str>,
    /// Password.
    pub password: Option<&'a [u8]>,
    /// Will message.
    pub will: Option<Will<'a>>,
    /// Session Expiry Interval property, 5.0 only.
    pub session_expiry: Option<u32>,
    /// Receive Maximum property, 5.0 only.
    pub receive_max: Option<u16>,
    /// Topic Alias Maximum property, 5.0 only.
    pub topic_alias_max: Option<u16>,
}

/// Appends a CONNECT packet.
pub fn connect(out: &mut Vec<u8>, c: &Connect<'_>) -> Result<()> {
    if !c.version.is_v5()
        && (c.session_expiry.is_some() || c.receive_max.is_some() || c.topic_alias_max.is_some())
    {
        return err("CONNECT properties need MQTT 5.0, and the session uses 3.1.1");
    }
    if c.version == Version::V311 && c.client_id.is_empty() && !c.clean {
        return err("MQTT 3.1.1 accepts an empty client identifier only with a clean session");
    }
    if c.version == Version::V311 && c.password.is_some() && c.user.is_none() {
        return err("MQTT 3.1.1 sends a password only with a user name");
    }
    if c.receive_max == Some(0) {
        return err("Receive Maximum must not be 0");
    }
    let mut flags = 0u8;
    if c.clean {
        flags |= 0x02;
    }
    if let Some(w) = &c.will {
        check_qos(w.qos)?;
        check_topic_name(w.topic)?;
        flags |= 0x04 | (w.qos << 3) | if w.retain { 0x20 } else { 0 };
    }
    if c.user.is_some() {
        flags |= 0x80;
    }
    if c.password.is_some() {
        flags |= 0x40;
    }
    let mut body = Vec::with_capacity(64);
    put_str(&mut body, "MQTT")?;
    body.push(c.version.level());
    body.push(flags);
    put_u16(&mut body, c.keepalive);
    if c.version.is_v5() {
        let mut p = Props::new();
        if let Some(v) = c.session_expiry {
            p.u32(prop::SESSION_EXPIRY, v);
        }
        if let Some(v) = c.receive_max {
            p.u16(prop::RECEIVE_MAXIMUM, v);
        }
        if let Some(v) = c.topic_alias_max {
            p.u16(prop::TOPIC_ALIAS_MAXIMUM, v);
        }
        p.write(&mut body)?;
    }
    put_str(&mut body, c.client_id)?;
    if let Some(w) = &c.will {
        if c.version.is_v5() {
            body.push(0); // no will properties
        }
        put_str(&mut body, w.topic)?;
        put_bin(&mut body, w.payload)?;
    }
    if let Some(u) = c.user {
        put_str(&mut body, u)?;
    }
    if let Some(p) = c.password {
        put_bin(&mut body, p)?;
    }
    finish(out, Kind::Connect, 0, &body)
}

/// The reason codes that a CONNACK of MQTT 5.0 may carry.
const CONNACK_REASONS: &[u8] = &[
    0, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 144, 149, 151, 153, 154, 155, 156,
    157, 159, 160, 161, 162,
];

/// The fields of a CONNACK packet.
#[derive(Clone, Copy, Debug)]
pub struct Connack<'a> {
    /// Protocol version.
    pub version: Version,
    /// Session Present flag.
    pub session_present: bool,
    /// Connect return code (3.1.1, 0 to 5) or reason code (5.0).
    pub code: u8,
    /// Session Expiry Interval property.
    pub session_expiry: Option<u32>,
    /// Receive Maximum property.
    pub receive_max: Option<u16>,
    /// Topic Alias Maximum property.
    pub topic_alias_max: Option<u16>,
    /// Server Keep Alive property.
    pub server_keepalive: Option<u16>,
    /// Assigned Client Identifier property.
    pub assigned_client_id: Option<&'a str>,
    /// Maximum QoS property, 0 or 1.
    pub max_qos: Option<u8>,
    /// Retain Available property.
    pub retain_available: Option<bool>,
}

/// Appends a CONNACK packet.
pub fn connack(out: &mut Vec<u8>, c: &Connack<'_>) -> Result<()> {
    let mut body = Vec::with_capacity(32);
    if c.session_present && c.code != 0 {
        return err("a CONNACK with a refusal must not set Session Present");
    }
    body.push(u8::from(c.session_present));
    body.push(c.code);
    if c.version.is_v5() {
        if !CONNACK_REASONS.contains(&c.code) {
            return err(format!("{} is not a reason code of a CONNACK", c.code));
        }
        let mut p = Props::new();
        if let Some(v) = c.session_expiry {
            p.u32(prop::SESSION_EXPIRY, v);
        }
        if let Some(v) = c.receive_max {
            if v == 0 {
                return err("Receive Maximum must not be 0");
            }
            p.u16(prop::RECEIVE_MAXIMUM, v);
        }
        if let Some(v) = c.max_qos {
            if v > 1 {
                return err("Maximum QoS is 0 or 1, and its absence means 2");
            }
            p.byte(prop::MAXIMUM_QOS, v);
        }
        if let Some(v) = c.retain_available {
            p.byte(prop::RETAIN_AVAILABLE, u8::from(v));
        }
        if let Some(v) = c.assigned_client_id {
            p.str(prop::ASSIGNED_CLIENT_ID, v)?;
        }
        if let Some(v) = c.topic_alias_max {
            p.u16(prop::TOPIC_ALIAS_MAXIMUM, v);
        }
        if let Some(v) = c.server_keepalive {
            p.u16(prop::SERVER_KEEP_ALIVE, v);
        }
        p.write(&mut body)?;
    } else {
        if c.code > 5 {
            return err(format!(
                "{} is not a connect return code of MQTT 3.1.1, which allows 0 to 5",
                c.code
            ));
        }
        let any = c.session_expiry.is_some()
            || c.receive_max.is_some()
            || c.topic_alias_max.is_some()
            || c.server_keepalive.is_some()
            || c.assigned_client_id.is_some()
            || c.max_qos.is_some()
            || c.retain_available.is_some();
        if any {
            return err("CONNACK properties need MQTT 5.0, and the session uses 3.1.1");
        }
    }
    finish(out, Kind::Connack, 0, &body)
}

// ----------------------------------------------------------------------------
// PUBLISH and its acknowledgments.

/// The fields of a PUBLISH packet.
#[derive(Clone, Copy, Debug)]
pub struct Publish<'a> {
    /// Protocol version.
    pub version: Version,
    /// Topic name.
    pub topic: &'a str,
    /// Application payload.
    pub payload: &'a [u8],
    /// Quality of service, 0 to 2.
    pub qos: u8,
    /// Retain flag.
    pub retain: bool,
    /// Duplicate flag, for a resend of QoS 1 and 2.
    pub dup: bool,
    /// Packet identifier, nonzero when `qos` is above 0.
    pub packet_id: u16,
    /// Message Expiry Interval property.
    pub message_expiry: Option<u32>,
    /// Content Type property.
    pub content_type: Option<&'a str>,
    /// Payload Format Indicator property, 0 or 1.
    pub payload_format: Option<u8>,
    /// Response Topic property.
    pub response_topic: Option<&'a str>,
    /// User Property.
    pub user_property: Option<(&'a str, &'a str)>,
}

/// Appends a PUBLISH packet.
pub fn publish(out: &mut Vec<u8>, p: &Publish<'_>) -> Result<()> {
    check_qos(p.qos)?;
    check_topic_name(p.topic)?;
    if p.dup && p.qos == 0 {
        return err("a PUBLISH of QoS 0 must not set DUP");
    }
    let has_props = p.message_expiry.is_some()
        || p.content_type.is_some()
        || p.payload_format.is_some()
        || p.response_topic.is_some()
        || p.user_property.is_some();
    if has_props {
        v5_only(p.version, "PUBLISH properties")?;
    }
    let mut body = Vec::with_capacity(p.topic.len() + p.payload.len() + 16);
    put_str(&mut body, p.topic)?;
    if p.qos > 0 {
        check_id(p.packet_id)?;
        put_u16(&mut body, p.packet_id);
    }
    if p.version.is_v5() {
        let mut props = Props::new();
        if let Some(v) = p.payload_format {
            if v > 1 {
                return err("Payload Format Indicator is 0 or 1");
            }
            props.byte(prop::PAYLOAD_FORMAT, v);
        }
        if let Some(v) = p.message_expiry {
            props.u32(prop::MESSAGE_EXPIRY, v);
        }
        if let Some(v) = p.response_topic {
            check_topic_name(v)?;
            props.str(prop::RESPONSE_TOPIC, v)?;
        }
        if let Some(v) = p.content_type {
            props.str(prop::CONTENT_TYPE, v)?;
        }
        if let Some((n, v)) = p.user_property {
            props.pair(n, v)?;
        }
        props.write(&mut body)?;
    }
    body.extend_from_slice(p.payload);
    let flags = (u8::from(p.dup) << 3) | (p.qos << 1) | u8::from(p.retain);
    finish(out, Kind::Publish, flags, &body)
}

/// The four acknowledgments of the publish flows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckKind {
    /// PUBACK, the end of the QoS 1 flow.
    Puback,
    /// PUBREC, the first answer of the QoS 2 flow.
    Pubrec,
    /// PUBREL, the release of the QoS 2 flow.
    Pubrel,
    /// PUBCOMP, the end of the QoS 2 flow.
    Pubcomp,
}

impl AckKind {
    /// The packet kind.
    pub fn kind(self) -> Kind {
        match self {
            AckKind::Puback => Kind::Puback,
            AckKind::Pubrec => Kind::Pubrec,
            AckKind::Pubrel => Kind::Pubrel,
            AckKind::Pubcomp => Kind::Pubcomp,
        }
    }

    /// The reason codes that MQTT 5.0 allows for the kind.
    pub fn reasons(self) -> &'static [u8] {
        match self {
            AckKind::Puback | AckKind::Pubrec => &[0, 16, 128, 131, 135, 144, 145, 151, 153],
            AckKind::Pubrel | AckKind::Pubcomp => &[0, 146],
        }
    }
}

/// Appends PUBACK, PUBREC, PUBREL, or PUBCOMP. MQTT 5.0 leaves out the reason
/// code of a success without properties, which shortens the packet to the two
/// bytes of the identifier.
pub fn ack(
    out: &mut Vec<u8>,
    version: Version,
    kind: AckKind,
    packet_id: u16,
    reason: u8,
    reason_string: Option<&str>,
) -> Result<()> {
    check_id(packet_id)?;
    let mut body = Vec::with_capacity(8);
    put_u16(&mut body, packet_id);
    if version.is_v5() {
        if !kind.reasons().contains(&reason) {
            return err(format!(
                "{reason} is not a reason code of a {}",
                kind.kind().name().to_ascii_uppercase()
            ));
        }
        if reason != 0 || reason_string.is_some() {
            body.push(reason);
            if let Some(s) = reason_string {
                let mut p = Props::new();
                p.str(prop::REASON_STRING, s)?;
                p.write(&mut body)?;
            }
        }
    } else if reason != 0 || reason_string.is_some() {
        return err("reason codes and reason strings need MQTT 5.0, and the session uses 3.1.1");
    }
    let flags = kind.kind().fixed_flags().unwrap_or(0);
    finish(out, kind.kind(), flags, &body)
}

// ----------------------------------------------------------------------------
// SUBSCRIBE, UNSUBSCRIBE, and their acknowledgments.

/// One topic filter of a SUBSCRIBE with its options.
#[derive(Clone, Copy, Debug)]
pub struct Subscription<'a> {
    /// Topic filter.
    pub filter: &'a str,
    /// Maximum QoS that the client accepts.
    pub qos: u8,
    /// No Local, 5.0 only.
    pub no_local: bool,
    /// Retain As Published, 5.0 only.
    pub retain_as_published: bool,
    /// Retain Handling, 0 to 2, 5.0 only.
    pub retain_handling: u8,
}

/// Appends a SUBSCRIBE packet.
pub fn subscribe(
    out: &mut Vec<u8>,
    version: Version,
    packet_id: u16,
    subs: &[Subscription<'_>],
) -> Result<()> {
    check_id(packet_id)?;
    if subs.is_empty() {
        return err("a SUBSCRIBE needs at least one topic filter");
    }
    let mut body = Vec::with_capacity(32);
    put_u16(&mut body, packet_id);
    if version.is_v5() {
        body.push(0); // no properties
    }
    for s in subs {
        check_topic_filter(s.filter)?;
        check_qos(s.qos)?;
        if s.retain_handling > 2 {
            return err("Retain Handling is 0, 1, or 2");
        }
        let v5_options = s.no_local || s.retain_as_published || s.retain_handling != 0;
        if v5_options {
            v5_only(version, "the subscription options No Local, Retain As Published, and Retain Handling")?;
        }
        put_str(&mut body, s.filter)?;
        body.push(
            s.qos
                | (u8::from(s.no_local) << 2)
                | (u8::from(s.retain_as_published) << 3)
                | (s.retain_handling << 4),
        );
    }
    finish(out, Kind::Subscribe, 0b0010, &body)
}

/// The return codes of a SUBACK of MQTT 3.1.1 and the reason codes of 5.0.
fn suback_code_ok(version: Version, code: u8) -> bool {
    match version {
        Version::V311 => matches!(code, 0..=2 | 0x80),
        Version::V5 => matches!(
            code,
            0..=2 | 0x80 | 0x83 | 0x87 | 0x8F | 0x91 | 0x97 | 0x9E | 0xA1 | 0xA2
        ),
    }
}

/// Appends a SUBACK packet with one code for each filter of the SUBSCRIBE.
pub fn suback(out: &mut Vec<u8>, version: Version, packet_id: u16, codes: &[u8]) -> Result<()> {
    check_id(packet_id)?;
    if codes.is_empty() {
        return err("a SUBACK needs at least one code");
    }
    if let Some(bad) = codes.iter().find(|c| !suback_code_ok(version, **c)) {
        return err(format!("{bad} is not a code of a SUBACK in this version"));
    }
    let mut body = Vec::with_capacity(codes.len() + 4);
    put_u16(&mut body, packet_id);
    if version.is_v5() {
        body.push(0);
    }
    body.extend_from_slice(codes);
    finish(out, Kind::Suback, 0, &body)
}

/// Appends an UNSUBSCRIBE packet.
pub fn unsubscribe(
    out: &mut Vec<u8>,
    version: Version,
    packet_id: u16,
    filters: &[&str],
) -> Result<()> {
    check_id(packet_id)?;
    if filters.is_empty() {
        return err("an UNSUBSCRIBE needs at least one topic filter");
    }
    let mut body = Vec::with_capacity(32);
    put_u16(&mut body, packet_id);
    if version.is_v5() {
        body.push(0);
    }
    for f in filters {
        check_topic_filter(f)?;
        put_str(&mut body, f)?;
    }
    finish(out, Kind::Unsubscribe, 0b0010, &body)
}

/// Appends an UNSUBACK packet. MQTT 3.1.1 carries only the identifier, and 5.0
/// adds one reason code for each filter of the UNSUBSCRIBE, 0 for success and
/// 17 for a filter without a subscription.
pub fn unsuback(
    out: &mut Vec<u8>,
    version: Version,
    packet_id: u16,
    codes: &[u8],
) -> Result<()> {
    check_id(packet_id)?;
    let mut body = Vec::with_capacity(codes.len() + 4);
    put_u16(&mut body, packet_id);
    if version.is_v5() {
        if codes.is_empty() {
            return err("an UNSUBACK of MQTT 5.0 needs one code for each filter");
        }
        if let Some(bad) = codes
            .iter()
            .find(|c| !matches!(**c, 0 | 17 | 128 | 131 | 135 | 143 | 145))
        {
            return err(format!("{bad} is not a reason code of an UNSUBACK"));
        }
        body.push(0);
        body.extend_from_slice(codes);
    }
    finish(out, Kind::Unsuback, 0, &body)
}

// ----------------------------------------------------------------------------
// PINGREQ, PINGRESP, and DISCONNECT.

/// Appends a PINGREQ packet.
pub fn pingreq(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0xC0, 0x00]);
}

/// Appends a PINGRESP packet.
pub fn pingresp(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0xD0, 0x00]);
}

/// The reason codes that a DISCONNECT of MQTT 5.0 may carry.
const DISCONNECT_REASONS: &[u8] = &[
    0, 4, 128, 129, 130, 131, 135, 137, 139, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151,
    152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162,
];

/// Appends a DISCONNECT packet. MQTT 3.1.1 has no reason code. MQTT 5.0 leaves
/// the body out for a normal disconnection without properties.
pub fn disconnect(
    out: &mut Vec<u8>,
    version: Version,
    reason: u8,
    reason_string: Option<&str>,
) -> Result<()> {
    let mut body = Vec::with_capacity(8);
    if version.is_v5() {
        if !DISCONNECT_REASONS.contains(&reason) {
            return err(format!("{reason} is not a reason code of a DISCONNECT"));
        }
        if let Some(s) = reason_string {
            body.push(reason);
            let mut p = Props::new();
            p.str(prop::REASON_STRING, s)?;
            p.write(&mut body)?;
        } else if reason != 0 {
            body.push(reason);
        }
    } else if reason != 0 || reason_string.is_some() {
        return err("reason codes and reason strings need MQTT 5.0, and the session uses 3.1.1");
    }
    finish(out, Kind::Disconnect, 0, &body)
}
