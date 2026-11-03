// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `http1` module.
//!
//! The entries mirror the `functions` list of `modules/http1/manifest.yaml`
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

/// The `User-Agent` value that requests carry unless the call names another.
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) FixingPointSystem/0.1";

/// The `Server` value that responses carry unless the call names another.
pub const DEFAULT_SERVER: &str = "fps/0.1";

/// The default size of one chunk of a chunked body.
pub const DEFAULT_CHUNK_SIZE: i64 = 8192;

/// `http1.request`, a request line, the header block, the empty line, and the
/// body.
pub const REQUEST: FunctionSpec = FunctionSpec {
    name: "request",
    params: &[
        req("method", ValueType::Text),
        req("path", ValueType::Text),
        req("host", ValueType::Text),
        opt("accept", ValueType::Text, DefaultValue::Text("*/*")),
        opt("version", ValueType::Text, DefaultValue::Text("1.1")),
        opt("body", ValueType::Bytes, DefaultValue::Bytes(b"")),
        opt("content_type", ValueType::Text, DefaultValue::Text("")),
        opt("chunked", ValueType::Boolean, DefaultValue::Boolean(false)),
        opt(
            "chunk_size",
            ValueType::Int,
            DefaultValue::Int(DEFAULT_CHUNK_SIZE),
        ),
        opt("connection", ValueType::Text, DefaultValue::Text("")),
        opt(
            "user_agent",
            ValueType::Text,
            DefaultValue::Text(DEFAULT_USER_AGENT),
        ),
    ],
};

/// `http1.response`, a status line, the header block, the empty line, and the
/// body.
pub const RESPONSE: FunctionSpec = FunctionSpec {
    name: "response",
    params: &[
        req("status", ValueType::Int),
        opt("body", ValueType::Bytes, DefaultValue::Bytes(b"")),
        opt(
            "content_type",
            ValueType::Text,
            DefaultValue::Text("text/html"),
        ),
        opt("version", ValueType::Text, DefaultValue::Text("1.1")),
        opt("chunked", ValueType::Boolean, DefaultValue::Boolean(false)),
        opt(
            "chunk_size",
            ValueType::Int,
            DefaultValue::Int(DEFAULT_CHUNK_SIZE),
        ),
        opt("connection", ValueType::Text, DefaultValue::Text("")),
        opt(
            "server",
            ValueType::Text,
            DefaultValue::Text(DEFAULT_SERVER),
        ),
    ],
};

/// Every template function of the module, in manifest order.
pub const FUNCTIONS: &[FunctionSpec] = &[REQUEST, RESPONSE];
