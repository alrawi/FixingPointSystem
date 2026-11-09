// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `pop3` module.
//!
//! The entries mirror the `functions` list of `modules/pop3/manifest.yaml`
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

/// The longest line of a message body in bytes, without the line break, that
/// the module writes. RFC 5322 recommends 78.
pub const MAX_LINE: usize = 76;

/// The largest mailbox that a reply covers, in messages.
pub const MAX_MESSAGES: i64 = 10_000;

/// Message n has the unique identifier `UID_BASE + n`, which `uidl_reply`
/// writes in the Dovecot format, so a mailbox looks alike over POP3 and IMAP.
pub const UID_BASE: i64 = 1000;

/// `pop3.credential`, a synthetic user name or password.
pub const CREDENTIAL: FunctionSpec = FunctionSpec {
    name: "credential",
    params: &[
        req("field", ValueType::Text),
        opt("seed", ValueType::Int, DefaultValue::Int(-1)),
    ],
};

/// `pop3.address`, a synthetic mailbox address.
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

/// `pop3.message`, an RFC 5322 message with a MIME text part and an optional
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

/// `pop3.next`, the number that follows a given one.
pub const NEXT: FunctionSpec = FunctionSpec {
    name: "next",
    params: &[
        req("value", ValueType::Any),
        opt("step", ValueType::Int, DefaultValue::Int(1)),
    ],
};

/// `pop3.banner`, the timestamp banner of the server greeting.
pub const BANNER: FunctionSpec = FunctionSpec {
    name: "banner",
    params: &[
        req("host", ValueType::Text),
        opt("seed", ValueType::Int, DefaultValue::Int(-1)),
    ],
};

/// `pop3.apop_digest`, the digest of an `APOP` command.
pub const APOP_DIGEST: FunctionSpec = FunctionSpec {
    name: "apop_digest",
    params: &[
        req("greeting", ValueType::Bytes),
        req("secret", ValueType::Text),
    ],
};

/// `pop3.dot_stuff`, a multi line payload with its terminator.
pub const DOT_STUFF: FunctionSpec = FunctionSpec {
    name: "dot_stuff",
    params: &[req("data", ValueType::Bytes)],
};

/// `pop3.stat_reply`, the answer to `STAT`.
pub const STAT_REPLY: FunctionSpec = FunctionSpec {
    name: "stat_reply",
    params: &[req("count", ValueType::Int), req("box", ValueType::Int)],
};

/// `pop3.list_reply`, the answer to `LIST` and `LIST n`.
pub const LIST_REPLY: FunctionSpec = FunctionSpec {
    name: "list_reply",
    params: &[
        req("command", ValueType::Bytes),
        req("count", ValueType::Int),
        req("box", ValueType::Int),
    ],
};

/// `pop3.uidl_reply`, the answer to `UIDL` and `UIDL n`.
pub const UIDL_REPLY: FunctionSpec = FunctionSpec {
    name: "uidl_reply",
    params: &[
        req("command", ValueType::Bytes),
        req("count", ValueType::Int),
        req("box", ValueType::Int),
    ],
};

/// `pop3.retr_reply`, the answer to `RETR n`.
pub const RETR_REPLY: FunctionSpec = FunctionSpec {
    name: "retr_reply",
    params: &[
        req("command", ValueType::Bytes),
        req("message", ValueType::Bytes),
        req("count", ValueType::Int),
    ],
};

/// `pop3.top_reply`, the answer to `TOP n lines`.
pub const TOP_REPLY: FunctionSpec = FunctionSpec {
    name: "top_reply",
    params: &[
        req("command", ValueType::Bytes),
        req("message", ValueType::Bytes),
        req("count", ValueType::Int),
    ],
};

/// `pop3.dele_reply`, the answer to `DELE n`.
pub const DELE_REPLY: FunctionSpec = FunctionSpec {
    name: "dele_reply",
    params: &[
        req("command", ValueType::Bytes),
        req("count", ValueType::Int),
    ],
};

/// Every template function of the module, in manifest order.
pub const FUNCTIONS: &[FunctionSpec] = &[
    CREDENTIAL,
    ADDRESS,
    MESSAGE,
    NEXT,
    BANNER,
    APOP_DIGEST,
    DOT_STUFF,
    STAT_REPLY,
    LIST_REPLY,
    UIDL_REPLY,
    RETR_REPLY,
    TOP_REPLY,
    DELE_REPLY,
];
