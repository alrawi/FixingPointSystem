// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `imap` module.
//!
//! The entries mirror the `functions` list of `modules/imap/manifest.yaml`
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

/// The largest number of messages that one reply covers.
pub const MAX_MESSAGES: i64 = 10_000;

/// The unique identifier of the message with sequence number 1. The module
/// numbers a mailbox so that UID n + `UID_BASE` names the message n, and
/// `select_data` predicts the next UID from the same rule.
pub const UID_BASE: i64 = 1000;

/// `imap.credential`, a synthetic user name or password.
pub const CREDENTIAL: FunctionSpec = FunctionSpec {
    name: "credential",
    params: &[
        req("field", ValueType::Text),
        opt("seed", ValueType::Int, DefaultValue::Int(-1)),
    ],
};

/// `imap.address`, a synthetic mailbox address.
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

/// `imap.auth_plain`, the base64 token of an `AUTHENTICATE PLAIN` answer.
pub const AUTH_PLAIN: FunctionSpec = FunctionSpec {
    name: "auth_plain",
    params: &[
        req("user", ValueType::Text),
        req("password", ValueType::Text),
        opt("authzid", ValueType::Text, DefaultValue::Text("")),
    ],
};

/// `imap.message`, an RFC 5322 message with a MIME text part and an optional
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

/// `imap.next`, the tag or number that follows a given one.
pub const NEXT: FunctionSpec = FunctionSpec {
    name: "next",
    params: &[
        req("value", ValueType::Any),
        opt("step", ValueType::Int, DefaultValue::Int(1)),
    ],
};

/// `imap.literal_marker`, the `{n}` that announces a literal of the data.
pub const LITERAL_MARKER: FunctionSpec = FunctionSpec {
    name: "literal_marker",
    params: &[req("data", ValueType::Bytes)],
};

/// `imap.literal_size`, the size that a command line announces for its literal.
pub const LITERAL_SIZE: FunctionSpec = FunctionSpec {
    name: "literal_size",
    params: &[req("command", ValueType::Bytes)],
};

/// `imap.tagged`, the tagged completion line of a command.
pub const TAGGED: FunctionSpec = FunctionSpec {
    name: "tagged",
    params: &[
        req("command", ValueType::Bytes),
        opt("status", ValueType::Text, DefaultValue::Text("OK")),
        opt("text", ValueType::Text, DefaultValue::Text("Completed")),
        opt("code", ValueType::Text, DefaultValue::Text("")),
        opt("timing", ValueType::Boolean, DefaultValue::Boolean(false)),
    ],
};

/// `imap.select_data`, the untagged responses to `SELECT` and `EXAMINE`.
pub const SELECT_DATA: FunctionSpec = FunctionSpec {
    name: "select_data",
    params: &[
        req("exists", ValueType::Int),
        opt("recent", ValueType::Int, DefaultValue::Int(0)),
        opt("unseen", ValueType::Int, DefaultValue::Int(0)),
        opt("uidvalidity", ValueType::Int, DefaultValue::Int(-1)),
        opt("read_only", ValueType::Boolean, DefaultValue::Boolean(false)),
    ],
};

/// `imap.fetch`, the untagged `FETCH` responses to a `FETCH` command.
pub const FETCH: FunctionSpec = FunctionSpec {
    name: "fetch",
    params: &[
        req("command", ValueType::Bytes),
        opt("message", ValueType::Bytes, DefaultValue::Bytes(b"")),
        opt("exists", ValueType::Int, DefaultValue::Int(0)),
    ],
};

/// `imap.search_reply`, the untagged `SEARCH` response.
pub const SEARCH_REPLY: FunctionSpec = FunctionSpec {
    name: "search_reply",
    params: &[req("exists", ValueType::Int), req("hits", ValueType::Int)],
};

/// `imap.search_hit`, one message number out of a `SEARCH` response.
pub const SEARCH_HIT: FunctionSpec = FunctionSpec {
    name: "search_hit",
    params: &[
        req("response", ValueType::Bytes),
        opt("index", ValueType::Int, DefaultValue::Int(1)),
    ],
};

/// `imap.store_reply`, the untagged `FETCH` responses to a `STORE` command.
pub const STORE_REPLY: FunctionSpec = FunctionSpec {
    name: "store_reply",
    params: &[req("command", ValueType::Bytes)],
};

/// `imap.expunge_reply`, the untagged `EXPUNGE` responses.
pub const EXPUNGE_REPLY: FunctionSpec = FunctionSpec {
    name: "expunge_reply",
    params: &[req("stored", ValueType::Bytes)],
};

/// Every template function of the module, in manifest order.
pub const FUNCTIONS: &[FunctionSpec] = &[
    CREDENTIAL,
    ADDRESS,
    AUTH_PLAIN,
    MESSAGE,
    NEXT,
    LITERAL_MARKER,
    LITERAL_SIZE,
    TAGGED,
    SELECT_DATA,
    FETCH,
    SEARCH_REPLY,
    SEARCH_HIT,
    STORE_REPLY,
    EXPUNGE_REPLY,
];
