// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `ftp` module.
//!
//! The entries mirror the `functions` list of `modules/ftp/manifest.yaml`
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

/// The largest number of entries that `listing` renders in one call.
pub const MAX_ENTRIES: i64 = 500;

/// `ftp.credential`, a synthetic user name or password.
pub const CREDENTIAL: FunctionSpec = FunctionSpec {
    name: "credential",
    params: &[
        req("field", ValueType::Text),
        opt("seed", ValueType::Int, DefaultValue::Int(-1)),
    ],
};

/// `ftp.file_name`, a synthetic file name with an extension.
pub const FILE_NAME: FunctionSpec = FunctionSpec {
    name: "file_name",
    params: &[
        req("seed", ValueType::Int),
        opt("ext", ValueType::Text, DefaultValue::Text("bin")),
    ],
};

/// `ftp.listing`, the lines of an `ls -l` directory listing.
pub const LISTING: FunctionSpec = FunctionSpec {
    name: "listing",
    params: &[req("seed", ValueType::Int), req("count", ValueType::Int)],
};

/// `ftp.length`, the number of bytes of its argument as a decimal number.
pub const LENGTH: FunctionSpec = FunctionSpec {
    name: "length",
    params: &[req("data", ValueType::Bytes)],
};

/// `ftp.number`, the last decimal number in a command or a reply.
pub const NUMBER: FunctionSpec = FunctionSpec {
    name: "number",
    params: &[req("line", ValueType::Bytes)],
};

/// `ftp.arg`, the argument of a command line.
pub const ARG: FunctionSpec = FunctionSpec {
    name: "arg",
    params: &[req("line", ValueType::Bytes)],
};

/// `ftp.pasv_reply`, the 227 reply that announces a passive data port.
pub const PASV_REPLY: FunctionSpec = FunctionSpec {
    name: "pasv_reply",
    params: &[req("host", ValueType::Text), req("port", ValueType::Int)],
};

/// `ftp.pasv_port`, the port that a 227 reply announces.
pub const PASV_PORT: FunctionSpec = FunctionSpec {
    name: "pasv_port",
    params: &[req("reply", ValueType::Bytes)],
};

/// `ftp.epsv_reply`, the 229 reply that announces a passive data port.
pub const EPSV_REPLY: FunctionSpec = FunctionSpec {
    name: "epsv_reply",
    params: &[req("port", ValueType::Int)],
};

/// `ftp.epsv_port`, the port that a 229 reply announces.
pub const EPSV_PORT: FunctionSpec = FunctionSpec {
    name: "epsv_port",
    params: &[req("reply", ValueType::Bytes)],
};

/// `ftp.feat_reply`, the multi line 211 reply to `FEAT`.
pub const FEAT_REPLY: FunctionSpec = FunctionSpec {
    name: "feat_reply",
    params: &[opt("utf8", ValueType::Boolean, DefaultValue::Boolean(true))],
};

/// Every template function of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    CREDENTIAL, FILE_NAME, LISTING, LENGTH, NUMBER, ARG, PASV_REPLY, PASV_PORT, EPSV_REPLY,
    EPSV_PORT, FEAT_REPLY,
];
