// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `content` module.
//!
//! The entries mirror the `functions` list of `modules/content/manifest.yaml`
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

const fn opt(name: &'static str, ty: ValueType, default: DefaultValue) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        default: Some(default),
    }
}

/// Largest `size` that a generator accepts, 16 MiB.
pub const MAX_SIZE: i64 = 16 * 1024 * 1024;

/// The default of the `seed` parameter. A negative seed draws the seed from
/// the session generator, so every session gets other content.
pub const NO_SEED: i64 = -1;

/// `content.html`, an HTML 5 page.
pub const HTML: FunctionSpec = FunctionSpec {
    name: "html",
    params: &[
        opt("size", ValueType::Int, DefaultValue::Int(4096)),
        opt("seed", ValueType::Int, DefaultValue::Int(NO_SEED)),
    ],
};

/// `content.json`, a JSON document in the shape of a paged API response.
pub const JSON: FunctionSpec = FunctionSpec {
    name: "json",
    params: &[
        opt("size", ValueType::Int, DefaultValue::Int(2048)),
        opt("seed", ValueType::Int, DefaultValue::Int(NO_SEED)),
    ],
};

/// `content.text`, plain prose in one of the supported languages.
pub const TEXT: FunctionSpec = FunctionSpec {
    name: "text",
    params: &[
        opt("lang", ValueType::Text, DefaultValue::Text("en")),
        opt("size", ValueType::Int, DefaultValue::Int(1024)),
        opt("seed", ValueType::Int, DefaultValue::Int(NO_SEED)),
    ],
};

/// `content.email`, the body of an email message.
pub const EMAIL: FunctionSpec = FunctionSpec {
    name: "email",
    params: &[
        opt("lang", ValueType::Text, DefaultValue::Text("en")),
        opt("size", ValueType::Int, DefaultValue::Int(1024)),
        opt("seed", ValueType::Int, DefaultValue::Int(NO_SEED)),
    ],
};

/// `content.binary`, a file of one binary type.
pub const BINARY: FunctionSpec = FunctionSpec {
    name: "binary",
    params: &[
        opt("kind", ValueType::Text, DefaultValue::Text("png")),
        opt("size", ValueType::Int, DefaultValue::Int(4096)),
        opt("seed", ValueType::Int, DefaultValue::Int(NO_SEED)),
    ],
};

/// Every template function of the module, in manifest order.
pub const FUNCTIONS: &[FunctionSpec] = &[HTML, JSON, TEXT, EMAIL, BINARY];
