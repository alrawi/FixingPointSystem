// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `smtp` module.
//!
//! The entries mirror the `functions` list of `modules/smtp/manifest.yaml`
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

/// The domain of every generated address. It lies under the reserved `.test`
/// top level domain of RFC 2606.
pub const DEFAULT_DOMAIN: &str = "example.test";

/// The `X-Mailer` value of a message unless the call names another.
pub const DEFAULT_MAILER: &str = "FixingPointSystem Mail 0.1";

/// The `SIZE` limit that an EHLO reply advertises unless the call names another.
pub const DEFAULT_SIZE: i64 = 35_882_577;

/// The longest line of a message body in bytes, without the line break, that
/// the module writes. RFC 5321 allows 998.
pub const MAX_LINE: usize = 76;

/// The largest number of recipients that one call renders.
pub const MAX_RECIPIENTS: i64 = 1000;

/// `smtp.credential`, a synthetic user name or password.
pub const CREDENTIAL: FunctionSpec = FunctionSpec {
    name: "credential",
    params: &[
        req("field", ValueType::Text),
        opt("seed", ValueType::Int, DefaultValue::Int(-1)),
    ],
};

/// `smtp.address`, a synthetic mailbox address.
pub const ADDRESS: FunctionSpec = FunctionSpec {
    name: "address",
    params: &[
        req("seed", ValueType::Int),
        opt("index", ValueType::Int, DefaultValue::Int(1)),
        opt(
            "domain",
            ValueType::Text,
            DefaultValue::Text(DEFAULT_DOMAIN),
        ),
    ],
};

/// `smtp.recipients`, addresses separated by a comma and a space, for a
/// `To` header.
pub const RECIPIENTS: FunctionSpec = FunctionSpec {
    name: "recipients",
    params: &[
        req("seed", ValueType::Int),
        req("count", ValueType::Int),
        opt(
            "domain",
            ValueType::Text,
            DefaultValue::Text(DEFAULT_DOMAIN),
        ),
    ],
};

/// `smtp.rcpt_to`, `RCPT TO` command lines, one per recipient.
pub const RCPT_TO: FunctionSpec = FunctionSpec {
    name: "rcpt_to",
    params: &[
        req("seed", ValueType::Int),
        opt("count", ValueType::Int, DefaultValue::Int(1)),
        opt("start", ValueType::Int, DefaultValue::Int(1)),
        opt(
            "domain",
            ValueType::Text,
            DefaultValue::Text(DEFAULT_DOMAIN),
        ),
    ],
};

/// `smtp.auth_plain`, the base64 token of an `AUTH PLAIN` initial response.
pub const AUTH_PLAIN: FunctionSpec = FunctionSpec {
    name: "auth_plain",
    params: &[
        req("user", ValueType::Text),
        req("password", ValueType::Text),
        opt("authzid", ValueType::Text, DefaultValue::Text("")),
    ],
};

/// `smtp.base64`, base64 text with an optional line length.
pub const BASE64: FunctionSpec = FunctionSpec {
    name: "base64",
    params: &[
        req("data", ValueType::Bytes),
        opt("wrap", ValueType::Int, DefaultValue::Int(0)),
    ],
};

/// `smtp.message`, an RFC 5322 message with a MIME text part and an optional
/// base64 attachment.
pub const MESSAGE: FunctionSpec = FunctionSpec {
    name: "message",
    params: &[
        req("sender", ValueType::Bytes),
        req("to", ValueType::Bytes),
        req("subject", ValueType::Text),
        req("body", ValueType::Bytes),
        opt("cc", ValueType::Bytes, DefaultValue::Bytes(b"")),
        opt("attachment", ValueType::Bytes, DefaultValue::Bytes(b"")),
        opt(
            "filename",
            ValueType::Text,
            DefaultValue::Text("attachment.bin"),
        ),
        opt(
            "content_type",
            ValueType::Text,
            DefaultValue::Text("application/octet-stream"),
        ),
        opt(
            "mailer",
            ValueType::Text,
            DefaultValue::Text(DEFAULT_MAILER),
        ),
    ],
};

/// `smtp.data`, a message in transparent form with the end of data line.
pub const DATA: FunctionSpec = FunctionSpec {
    name: "data",
    params: &[req("message", ValueType::Bytes)],
};

/// `smtp.ehlo_reply`, the multi line 250 reply to `EHLO`.
pub const EHLO_REPLY: FunctionSpec = FunctionSpec {
    name: "ehlo_reply",
    params: &[
        req("domain", ValueType::Text),
        req("client", ValueType::Text),
        opt("auth", ValueType::Text, DefaultValue::Text("")),
        opt(
            "pipelining",
            ValueType::Boolean,
            DefaultValue::Boolean(true),
        ),
        opt("size", ValueType::Int, DefaultValue::Int(DEFAULT_SIZE)),
    ],
};

/// `smtp.repeat_line`, one reply line written several times, for the answers
/// that a server gives to a pipelined batch of `RCPT TO` commands.
pub const REPEAT_LINE: FunctionSpec = FunctionSpec {
    name: "repeat_line",
    params: &[req("line", ValueType::Text), req("count", ValueType::Int)],
};

/// Every template function of the module, in manifest order.
pub const FUNCTIONS: &[FunctionSpec] = &[
    CREDENTIAL,
    ADDRESS,
    RECIPIENTS,
    RCPT_TO,
    AUTH_PLAIN,
    BASE64,
    MESSAGE,
    DATA,
    EHLO_REPLY,
    REPEAT_LINE,
];
