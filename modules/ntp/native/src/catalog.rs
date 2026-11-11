// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `ntp` module.
//!
//! The entries mirror the `functions` list of `modules/ntp/manifest.yaml`
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

/// The default NTP version of a message.
pub const DEFAULT_VERSION: i64 = 4;

/// The default `poll` exponent of a request, 2^6 = 64 seconds, the shortest
/// interval that a client holds in steady state.
pub const DEFAULT_POLL: i64 = 6;

/// `ntp.query`, a client request (mode 3) or the packet of a symmetric active
/// peer (mode 1), with a transmit timestamp that comes from `now_ms`.
pub const QUERY: FunctionSpec = FunctionSpec {
    name: "query",
    params: &[
        req("now_ms", ValueType::Int),
        opt(
            "version",
            ValueType::Int,
            DefaultValue::Int(DEFAULT_VERSION),
        ),
        opt("poll", ValueType::Int, DefaultValue::Int(DEFAULT_POLL)),
        opt("mode", ValueType::Int, DefaultValue::Int(3)),
        opt("stratum", ValueType::Int, DefaultValue::Int(0)),
        opt("reference", ValueType::Text, DefaultValue::Text("")),
    ],
};

/// `ntp.reply`, the server packet (mode 4) for a client request, or the packet
/// of a symmetric passive peer (mode 2) for a mode 1 packet.
pub const REPLY: FunctionSpec = FunctionSpec {
    name: "reply",
    params: &[
        req("request", ValueType::Bytes),
        req("now_ms", ValueType::Int),
        opt("stratum", ValueType::Int, DefaultValue::Int(2)),
        opt("reference", ValueType::Text, DefaultValue::Text("")),
        opt("kiss", ValueType::Text, DefaultValue::Text("")),
    ],
};

/// `ntp.field`, one field of a packet as text.
pub const FIELD: FunctionSpec = FunctionSpec {
    name: "field",
    params: &[
        req("message", ValueType::Bytes),
        req("name", ValueType::Text),
    ],
};

/// Every function of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[QUERY, REPLY, FIELD];
