// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `dns` module.
//!
//! The entries mirror the `functions` list of `modules/dns/manifest.yaml`
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

/// The TTL of an answer that the `address` argument of `dns.response`
/// gives, as the contract example manifest declares it.
pub const DEFAULT_TTL: i64 = 300;

/// `dns.query`, a query message with one question and a transaction id from
/// the session generator.
pub const QUERY: FunctionSpec = FunctionSpec {
    name: "query",
    params: &[
        req("name", ValueType::Text),
        opt("qtype", ValueType::Text, DefaultValue::Text("A")),
        opt("tcp", ValueType::Boolean, DefaultValue::Boolean(false)),
    ],
};

/// `dns.answer_ip`, the first address in the answer section of a response,
/// as text.
pub const ANSWER_IP: FunctionSpec = FunctionSpec {
    name: "answer_ip",
    params: &[req("message", ValueType::Bytes)],
};

/// `dns.response`, the response to a query message, from the zone or from
/// one given address.
pub const RESPONSE: FunctionSpec = FunctionSpec {
    name: "response",
    params: &[
        req("query", ValueType::Bytes),
        opt("address", ValueType::Text, DefaultValue::Text("")),
        opt("ttl", ValueType::Int, DefaultValue::Int(DEFAULT_TTL)),
        opt("tcp", ValueType::Boolean, DefaultValue::Boolean(false)),
        opt("udp_size", ValueType::Int, DefaultValue::Int(512)),
    ],
};

/// `dns.tcp_length`, the message length that a two byte TCP length prefix
/// announces, as text.
pub const TCP_LENGTH: FunctionSpec = FunctionSpec {
    name: "tcp_length",
    params: &[req("prefix", ValueType::Bytes)],
};

/// Every template function of the module, in manifest order.
pub const FUNCTIONS: &[FunctionSpec] = &[QUERY, ANSWER_IP, RESPONSE, TCP_LENGTH];
