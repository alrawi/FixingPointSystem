// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `mqtt` module.
//!
//! The entries are the `functions` list that `modules/mqtt/manifest.yaml`
//! carries, name for name, parameter for parameter, in the same order and with
//! the same types and defaults, as the contract section "Template function
//! catalog" requires. `cargo run -p fps-module-mqtt --example print_catalog`
//! prints the list in manifest syntax, and the catalog test compares the
//! manifest with it.

use fps_flow::ValueType;

/// The default of an optional parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    /// A text default.
    Text(&'static str),
    /// An integer default.
    Int(i64),
    /// A boolean default.
    Boolean(bool),
    /// A bytes default. The manifest writes it as hexadecimal digit pairs.
    Bytes(&'static [u8]),
}

/// One parameter of a template function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSpec {
    /// Parameter name.
    pub name: &'static str,
    /// Declared type.
    pub ty: ValueType,
    /// The default, or `None` for a required parameter.
    pub default: Option<DefaultValue>,
}

/// One template function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionSpec {
    /// Function name without the module prefix.
    pub name: &'static str,
    /// What the function renders, in one sentence, for the manifest.
    pub summary: &'static str,
    /// Parameters in positional order.
    pub params: &'static [ParamSpec],
}

const fn req(name: &'static str, ty: ValueType) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        default: None,
    }
}

const fn opt(name: &'static str, ty: ValueType, default: DefaultValue) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        default: Some(default),
    }
}

use DefaultValue::{Boolean as B, Bytes as Y, Int as I, Text as T};
use ValueType::{Boolean, Bytes, Int, Text};

/// The value of an integer parameter that leaves an optional property or field
/// out, and that lets the function choose a packet identifier.
pub const AUTO: i64 = -1;

const PACKET_ID: ParamSpec = opt("packet_id", Int, I(AUTO));
const REASON: ParamSpec = opt("reason", Int, I(0));

pub(crate) const CONNECT_SPEC: FunctionSpec = FunctionSpec {
    name: "connect",
    summary: "CONNECT packet. The version is 4 for MQTT 3.1.1 or 5 for 5.0, and later packets of the side follow it. Empty user, password, and will_topic are left out, and -1 leaves out a property of 5.0.",
    params: &[
        req("client_id", Bytes),
        opt("version", Int, I(4)),
        opt("keepalive", Int, I(60)),
        opt("clean", Boolean, B(true)),
        opt("user", Text, T("")),
        opt("password", Text, T("")),
        opt("will_topic", Text, T("")),
        opt("will_payload", Bytes, Y(&[])),
        opt("will_qos", Int, I(0)),
        opt("will_retain", Boolean, B(false)),
        opt("session_expiry", Int, I(AUTO)),
        opt("receive_max", Int, I(AUTO)),
        opt("topic_alias_max", Int, I(AUTO)),
    ],
};

pub(crate) const CONNACK_SPEC: FunctionSpec = FunctionSpec {
    name: "connack",
    summary: "CONNACK packet with the return code of MQTT 3.1.1 or the reason code of 5.0. A value of -1 leaves a property of 5.0 out, and the defaults are those of Mosquitto 2.",
    params: &[
        opt("code", Int, I(0)),
        opt("session_present", Boolean, B(false)),
        opt("receive_max", Int, I(20)),
        opt("topic_alias_max", Int, I(10)),
        opt("server_keepalive", Int, I(AUTO)),
        opt("assigned_client_id", Text, T("")),
        opt("max_qos", Int, I(AUTO)),
        opt("session_expiry", Int, I(AUTO)),
    ],
};

pub(crate) const PUBLISH_SPEC: FunctionSpec = FunctionSpec {
    name: "publish",
    summary: "PUBLISH packet. A packet_id of -1 takes the next identifier of this side, and QoS 0 has none. The last five parameters are properties of MQTT 5.0, and -1 or empty text leaves one out.",
    params: &[
        req("topic", Text),
        req("payload", Bytes),
        opt("qos", Int, I(0)),
        opt("retain", Boolean, B(false)),
        PACKET_ID,
        opt("dup", Boolean, B(false)),
        opt("message_expiry", Int, I(AUTO)),
        opt("content_type", Text, T("")),
        opt("payload_format", Int, I(AUTO)),
        opt("response_topic", Text, T("")),
        opt("user_property", Text, T("")),
    ],
};

pub(crate) const PUBACK_SPEC: FunctionSpec = FunctionSpec {
    name: "puback",
    summary: "PUBACK packet, the end of a QoS 1 flow. A packet_id of -1 answers the packet that this side read last. MQTT 5.0 leaves out the reason of a success.",
    params: &[PACKET_ID, REASON],
};

pub(crate) const PUBREC_SPEC: FunctionSpec = FunctionSpec {
    name: "pubrec",
    summary: "PUBREC packet, the first answer of a QoS 2 flow. A packet_id of -1 answers the packet that this side read last.",
    params: &[PACKET_ID, REASON],
};

pub(crate) const PUBREL_SPEC: FunctionSpec = FunctionSpec {
    name: "pubrel",
    summary: "PUBREL packet, the release of a QoS 2 flow. A packet_id of -1 answers the packet that this side read last.",
    params: &[PACKET_ID, REASON],
};

pub(crate) const PUBCOMP_SPEC: FunctionSpec = FunctionSpec {
    name: "pubcomp",
    summary: "PUBCOMP packet, the end of a QoS 2 flow. A packet_id of -1 answers the packet that this side read last.",
    params: &[PACKET_ID, REASON],
};

pub(crate) const SUBSCRIBE_SPEC: FunctionSpec = FunctionSpec {
    name: "subscribe",
    summary: "SUBSCRIBE packet for the comma separated topic filters, all with the same maximum QoS. A packet_id of -1 takes the next identifier of this side. The last three parameters are options of MQTT 5.0.",
    params: &[
        req("filters", Text),
        opt("qos", Int, I(0)),
        PACKET_ID,
        opt("no_local", Boolean, B(false)),
        opt("retain_as_published", Boolean, B(false)),
        opt("retain_handling", Int, I(0)),
    ],
};

pub(crate) const SUBACK_SPEC: FunctionSpec = FunctionSpec {
    name: "suback",
    summary: "SUBACK packet with the comma separated codes, one for each filter, which are the granted QoS 0 to 2 or a failure such as 128. A packet_id of -1 answers the packet that this side read last.",
    params: &[req("codes", Bytes), PACKET_ID],
};

pub(crate) const UNSUBSCRIBE_SPEC: FunctionSpec = FunctionSpec {
    name: "unsubscribe",
    summary: "UNSUBSCRIBE packet for the comma separated topic filters. A packet_id of -1 takes the next identifier of this side.",
    params: &[req("filters", Text), PACKET_ID],
};

pub(crate) const UNSUBACK_SPEC: FunctionSpec = FunctionSpec {
    name: "unsuback",
    summary: "UNSUBACK packet for count filters. MQTT 5.0 carries one reason code for each filter, and MQTT 3.1.1 carries none. A packet_id of -1 answers the packet that this side read last.",
    params: &[
        opt("count", Int, I(1)),
        PACKET_ID,
        opt("reason", Int, I(0)),
    ],
};

pub(crate) const PINGREQ_SPEC: FunctionSpec = FunctionSpec {
    name: "pingreq",
    summary: "PINGREQ packet, which a client sends when its keep alive interval passes without another packet.",
    params: &[],
};

pub(crate) const PINGRESP_SPEC: FunctionSpec = FunctionSpec {
    name: "pingresp",
    summary: "PINGRESP packet, the answer of a broker to a PINGREQ.",
    params: &[],
};

pub(crate) const DISCONNECT_SPEC: FunctionSpec = FunctionSpec {
    name: "disconnect",
    summary: "DISCONNECT packet. MQTT 3.1.1 has no reason, and MQTT 5.0 leaves out the reason 0 and an empty reason string.",
    params: &[opt("reason", Int, I(0)), opt("reason_string", Text, T(""))],
};

pub(crate) const VARINT_SPEC: FunctionSpec = FunctionSpec {
    name: "varint",
    summary: "A variable byte integer in the least number of bytes, which the Remaining Length, the property length, and the subscription identifier use. The value stays at or below 268435455.",
    params: &[req("value", Int)],
};

pub(crate) const VARINT_VALUE_SPEC: FunctionSpec = FunctionSpec {
    name: "varint_value",
    summary: "The number that a variable byte integer at the start of the bytes holds, as text. Another byte count or a form that is not minimal is an error.",
    params: &[req("data", Bytes)],
};

pub(crate) const CLIENT_ID_SPEC: FunctionSpec = FunctionSpec {
    name: "client_id",
    summary: "A client identifier, the prefix and eight random lower case hexadecimal digits from the session generator, as the MQTT.js client draws them.",
    params: &[opt("prefix", Text, T("mqttjs_"))],
};

pub(crate) const READING_SPEC: FunctionSpec = FunctionSpec {
    name: "reading",
    summary: "A JSON sensor reading for the device with the next sequence number of this side and random values from the session generator, for the payload of a PUBLISH.",
    params: &[opt("device", Text, T("sensor-0412"))],
};

pub(crate) const FEED_SPEC: FunctionSpec = FunctionSpec {
    name: "feed",
    summary: "Hands the bytes of the last read to the packet reader and returns the size of the next read as text. A flow reads one byte, then six reads of this size, and then calls recv.",
    params: &[req("data", Bytes)],
};

pub(crate) const RECV_SPEC: FunctionSpec = FunctionSpec {
    name: "recv",
    summary: "Checks the collected packet against the standard and the kind in expect (any, connack, publish:1, and so on), stores its fields, and returns the kind. A CONNECT sets the version of the session.",
    params: &[opt("expect", Text, T("any"))],
};

pub(crate) const FIELD_SPEC: FunctionSpec = FunctionSpec {
    name: "field",
    summary: "A field of the packet that recv read last, as text, such as packet_id, qos, topic, payload, or filter_count, or a property of MQTT 5.0 by its snake case name. The README lists every name.",
    params: &[req("name", Text)],
};

pub(crate) const GRANTED_SPEC: FunctionSpec = FunctionSpec {
    name: "granted",
    summary: "The comma separated codes that grant the QoS of each filter of the SUBSCRIBE that recv read last, each capped at max_qos, for the codes of a SUBACK.",
    params: &[opt("max_qos", Int, I(2))],
};

pub(crate) const PACE_SPEC: FunctionSpec = FunctionSpec {
    name: "pace",
    summary: "Milliseconds to sleep until the next keep alive slot of this flow side, as text. Each call moves the due time on by keepalive seconds, so a late wake shortens the next sleep.",
    params: &[req("keepalive", Int)],
};

/// The template functions of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    CONNECT_SPEC,
    CONNACK_SPEC,
    PUBLISH_SPEC,
    PUBACK_SPEC,
    PUBREC_SPEC,
    PUBREL_SPEC,
    PUBCOMP_SPEC,
    SUBSCRIBE_SPEC,
    SUBACK_SPEC,
    UNSUBSCRIBE_SPEC,
    UNSUBACK_SPEC,
    PINGREQ_SPEC,
    PINGRESP_SPEC,
    DISCONNECT_SPEC,
    VARINT_SPEC,
    VARINT_VALUE_SPEC,
    CLIENT_ID_SPEC,
    READING_SPEC,
    FEED_SPEC,
    RECV_SPEC,
    FIELD_SPEC,
    GRANTED_SPEC,
    PACE_SPEC,
];

fn yaml_text(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Renders [`FUNCTIONS`] as the `functions` list of a module manifest. The
/// text between `functions:` and `scenarios:` of `modules/mqtt/manifest.yaml`
/// is this output, and the catalog test compares the two.
pub fn manifest_functions() -> String {
    let mut out = String::from("functions:\n");
    for f in FUNCTIONS {
        out.push_str(&format!("  - name: {}\n", f.name));
        out.push_str(&format!("    summary: {}\n", yaml_text(f.summary)));
        if f.params.is_empty() {
            out.push_str("    params: []\n");
            continue;
        }
        out.push_str("    params:\n");
        for p in f.params {
            let mut line = format!("      - {{ name: {}, type: {}", p.name, p.ty);
            match p.default {
                None => {}
                Some(DefaultValue::Text(t)) => {
                    line.push_str(&format!(", default: {}", yaml_text(t)))
                }
                Some(DefaultValue::Int(i)) => line.push_str(&format!(", default: {i}")),
                Some(DefaultValue::Boolean(b)) => line.push_str(&format!(", default: {b}")),
                Some(DefaultValue::Bytes(b)) => {
                    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
                    line.push_str(&format!(", default: {}", yaml_text(&hex)));
                }
            }
            line.push_str(" }\n");
            out.push_str(&line);
        }
    }
    out
}
