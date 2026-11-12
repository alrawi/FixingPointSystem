// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `ldap` module.
//!
//! The entries mirror the `functions` list of `modules/ldap/manifest.yaml`
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

/// The largest message that the module reads or writes, which is the longest
/// value that two length octets hold, plus the 4 octet head.
pub const MAX_MESSAGE: i64 = 65_539;

macro_rules! spec {
    ($(#[$doc:meta])* $konst:ident, $name:literal, [$($p:expr),* $(,)?]) => {
        $(#[$doc])*
        pub const $konst: FunctionSpec = FunctionSpec {
            name: $name,
            params: &[$($p),*],
        };
    };
}

spec!(
    /// `ldap.size`, which reads the head of a message and returns the number of octets that follow.
    SIZE,
    "size",
    [req("head", ValueType::Bytes)]
);
spec!(
    /// `ldap.recv`, which decodes a message, checks its kind and result, and keeps it.
    RECV,
    "recv",
    [
        req("message", ValueType::Bytes),
        req("kind", ValueType::Text),
        opt("result", ValueType::Int, DefaultValue::Int(0)),
    ]
);
spec!(
    /// `ldap.field`, which reads a field of the message that `recv` kept.
    FIELD,
    "field",
    [req("name", ValueType::Text)]
);
spec!(
    /// `ldap.bind_request`, a simple bind.
    BIND_REQUEST,
    "bind_request",
    [
        req("message_id", ValueType::Int),
        req("name", ValueType::Text),
        req("password", ValueType::Text),
        opt("version", ValueType::Int, DefaultValue::Int(3)),
    ]
);
spec!(
    /// `ldap.bind_response`.
    BIND_RESPONSE,
    "bind_response",
    [
        req("message_id", ValueType::Int),
        opt("result_code", ValueType::Int, DefaultValue::Int(0)),
        opt("matched_dn", ValueType::Text, DefaultValue::Text("")),
        opt("diagnostic", ValueType::Text, DefaultValue::Text("")),
    ]
);
spec!(
    /// `ldap.search_request`.
    SEARCH_REQUEST,
    "search_request",
    [
        req("message_id", ValueType::Int),
        req("base", ValueType::Text),
        opt("scope", ValueType::Text, DefaultValue::Text("sub")),
        opt(
            "filter",
            ValueType::Text,
            DefaultValue::Text("(objectClass=*)")
        ),
        opt("attributes", ValueType::Text, DefaultValue::Text("")),
        opt("size_limit", ValueType::Int, DefaultValue::Int(0)),
        opt("time_limit", ValueType::Int, DefaultValue::Int(0)),
        opt("deref", ValueType::Text, DefaultValue::Text("never")),
        opt(
            "types_only",
            ValueType::Boolean,
            DefaultValue::Boolean(false)
        ),
    ]
);
spec!(
    /// `ldap.search_entry`.
    SEARCH_ENTRY,
    "search_entry",
    [
        req("message_id", ValueType::Int),
        req("dn", ValueType::Text),
        req("attributes", ValueType::Text),
    ]
);
spec!(
    /// `ldap.search_done`.
    SEARCH_DONE,
    "search_done",
    [
        req("message_id", ValueType::Int),
        opt("result_code", ValueType::Int, DefaultValue::Int(0)),
        opt("matched_dn", ValueType::Text, DefaultValue::Text("")),
        opt("diagnostic", ValueType::Text, DefaultValue::Text("")),
    ]
);
spec!(
    /// `ldap.unbind_request`.
    UNBIND_REQUEST,
    "unbind_request",
    [req("message_id", ValueType::Int)]
);
spec!(
    /// `ldap.filter_eq`, an equality filter with an escaped value.
    FILTER_EQ,
    "filter_eq",
    [
        req("attribute", ValueType::Text),
        req("value", ValueType::Text),
    ]
);
spec!(
    /// `ldap.account`, a field of an entry of the directory that has a password.
    ACCOUNT,
    "account",
    [
        req("directory", ValueType::Text),
        req("field", ValueType::Text),
        opt("seed", ValueType::Int, DefaultValue::Int(-1)),
    ]
);
spec!(
    /// `ldap.count`, the number of entries that a search returns.
    COUNT,
    "count",
    [
        req("directory", ValueType::Text),
        req("base", ValueType::Text),
        opt("scope", ValueType::Text, DefaultValue::Text("sub")),
        opt(
            "filter",
            ValueType::Text,
            DefaultValue::Text("(objectClass=*)")
        ),
        opt("size_limit", ValueType::Int, DefaultValue::Int(0)),
    ]
);
spec!(
    /// `ldap.bind_answer`, the response to the bind request that `recv` kept.
    BIND_ANSWER,
    "bind_answer",
    [req("directory", ValueType::Text)]
);
spec!(
    /// `ldap.matches`, which runs the search request that `recv` kept.
    MATCHES,
    "matches",
    [req("directory", ValueType::Text)]
);
spec!(
    /// `ldap.search_next`, the next entry of the search that `matches` ran.
    SEARCH_NEXT,
    "search_next",
    []
);
spec!(
    /// `ldap.search_end`, the `SearchResultDone` of the search that `matches` ran.
    SEARCH_END,
    "search_end",
    []
);

/// Every template function of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    SIZE,
    RECV,
    FIELD,
    BIND_REQUEST,
    BIND_RESPONSE,
    SEARCH_REQUEST,
    SEARCH_ENTRY,
    SEARCH_DONE,
    UNBIND_REQUEST,
    FILTER_EQ,
    ACCOUNT,
    COUNT,
    BIND_ANSWER,
    MATCHES,
    SEARCH_NEXT,
    SEARCH_END,
];
