// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `sip_rtp` module.
//!
//! The entries mirror the `functions` list of `modules/sip_rtp/manifest.yaml`
//! name for name, parameter for parameter, in the same order and with the same
//! types and defaults, as the contract section "Template function catalog"
//! requires.

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

/// The codecs that a server accepts unless a scenario says otherwise.
pub const DEFAULT_ACCEPT: &str = "PCMU,PCMA,G722,G729,opus";

/// `sip_rtp.port`, an even RTP port drawn from the session generator.
pub const PORT: FunctionSpec = FunctionSpec {
    name: "port",
    params: &[req("min", ValueType::Int), req("max", ValueType::Int)],
};

/// `sip_rtp.invite`, an INVITE request with an SDP offer.
pub const INVITE: FunctionSpec = FunctionSpec {
    name: "invite",
    params: &[
        req("server", ValueType::Text),
        req("client", ValueType::Text),
        req("rtp_port", ValueType::Int),
        opt("codecs", ValueType::Text, DefaultValue::Text("PCMU,PCMA")),
        opt("ptime", ValueType::Int, DefaultValue::Int(20)),
        opt("sip_port", ValueType::Int, DefaultValue::Int(5060)),
    ],
};

/// `sip_rtp.response`, a response to an INVITE or a BYE.
pub const RESPONSE: FunctionSpec = FunctionSpec {
    name: "response",
    params: &[
        req("request", ValueType::Bytes),
        req("code", ValueType::Int),
        opt("server", ValueType::Text, DefaultValue::Text("")),
        opt("rtp_port", ValueType::Int, DefaultValue::Int(0)),
        opt(
            "accept",
            ValueType::Text,
            DefaultValue::Text(DEFAULT_ACCEPT),
        ),
        opt("sip_port", ValueType::Int, DefaultValue::Int(5060)),
    ],
};

/// `sip_rtp.ack`, the ACK for a final response to an INVITE.
pub const ACK: FunctionSpec = FunctionSpec {
    name: "ack",
    params: &[req("response", ValueType::Bytes)],
};

/// `sip_rtp.bye`, the BYE that ends the dialog of a 200 response.
pub const BYE: FunctionSpec = FunctionSpec {
    name: "bye",
    params: &[req("response", ValueType::Bytes)],
};

/// `sip_rtp.sdp_port`, the audio port of an SDP body.
pub const SDP_PORT: FunctionSpec = FunctionSpec {
    name: "sdp_port",
    params: &[req("message", ValueType::Bytes)],
};

/// `sip_rtp.codec`, the codec that an offer or an answer selects.
pub const CODEC: FunctionSpec = FunctionSpec {
    name: "codec",
    params: &[
        req("message", ValueType::Bytes),
        opt("accept", ValueType::Text, DefaultValue::Text("")),
    ],
};

/// `sip_rtp.rtp`, the next RTP packet of the stream of this flow side.
pub const RTP: FunctionSpec = FunctionSpec {
    name: "rtp",
    params: &[
        req("codec", ValueType::Text),
        opt("ptime", ValueType::Int, DefaultValue::Int(20)),
    ],
};

/// `sip_rtp.packets`, the number of RTP packets of a call of a given length.
pub const PACKETS: FunctionSpec = FunctionSpec {
    name: "packets",
    params: &[
        req("talk_ms", ValueType::Int),
        opt("ptime", ValueType::Int, DefaultValue::Int(20)),
    ],
};

/// `sip_rtp.pace`, the sleep until the next RTP packet of the schedule.
pub const PACE: FunctionSpec = FunctionSpec {
    name: "pace",
    params: &[opt("ptime", ValueType::Int, DefaultValue::Int(20))],
};

/// `sip_rtp.hangup_ms`, the time from the start of the media to the BYE.
pub const HANGUP_MS: FunctionSpec = FunctionSpec {
    name: "hangup_ms",
    params: &[req("talk_ms", ValueType::Int)],
};

/// Every template function of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    PORT, INVITE, RESPONSE, ACK, BYE, SDP_PORT, CODEC, RTP, PACKETS, PACE, HANGUP_MS,
];
