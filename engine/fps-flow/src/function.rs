// SPDX-License-Identifier: GPL-3.0-only
//! Dispatch of module template functions.
//!
//! A template such as `{{ http1.response(200, body=fill(vars.n)) }}` compiles
//! into a call slot. At render time the engine evaluates the arguments and
//! hands the call to a [`TemplateFn`], which appends the rendered bytes to the
//! session send buffer. The types mirror the `render` hook and the `call-args`
//! record of `contracts/extension/fps-extension.wit`, so a native extension
//! (decision D10) or the WebAssembly adapter (ENG-11) implements the trait with
//! a thin wrapper around `ExtensionSession::render`.

use std::fmt;

use rand::RngCore;

/// The declared type of a template value or function parameter.
///
/// The cases match the `value-type` enum of the extension interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueType {
    /// A signed 64 bit integer.
    Int,
    /// A 64 bit floating point number.
    Float,
    /// A boolean.
    Boolean,
    /// UTF-8 text.
    Text,
    /// Arbitrary bytes.
    Bytes,
    /// Any of the five value cases, checked at render time.
    Any,
}

impl ValueType {
    /// Returns true when a value of type `arg` may bind a parameter of type
    /// `self`, with the rules of the contract section "Types".
    pub fn accepts(self, arg: ValueType) -> bool {
        self == arg
            || self == ValueType::Any
            || arg == ValueType::Any
            || matches!(
                (self, arg),
                (ValueType::Float, ValueType::Int) | (ValueType::Bytes, ValueType::Text)
            )
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ValueType::Int => "int",
            ValueType::Float => "float",
            ValueType::Boolean => "boolean",
            ValueType::Text => "text",
            ValueType::Bytes => "bytes",
            ValueType::Any => "any",
        })
    }
}

/// One evaluated argument of a module call, borrowed from the template, the
/// scenario variables, or the session buffers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArgValue<'a> {
    /// An integer.
    Int(i64),
    /// A floating point number.
    Float(f64),
    /// A boolean.
    Boolean(bool),
    /// Text.
    Text(&'a str),
    /// Bytes, for example `last` or the result of a nested module call.
    Bytes(&'a [u8]),
}

impl ArgValue<'_> {
    /// Returns the type of the value.
    pub fn value_type(&self) -> ValueType {
        match self {
            ArgValue::Int(_) => ValueType::Int,
            ArgValue::Float(_) => ValueType::Float,
            ArgValue::Boolean(_) => ValueType::Boolean,
            ArgValue::Text(_) => ValueType::Text,
            ArgValue::Bytes(_) => ValueType::Bytes,
        }
    }
}

/// A keyword argument of a module call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NamedArg<'a> {
    /// Parameter name as the template wrote it.
    pub name: &'a str,
    /// The evaluated value.
    pub value: ArgValue<'a>,
}

/// The arguments of one module call. Positional arguments come first in call
/// order, and keyword arguments keep their call order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CallArgs<'a> {
    /// Positional arguments.
    pub positional: &'a [ArgValue<'a>],
    /// Keyword arguments.
    pub named: &'a [NamedArg<'a>],
}

/// One module function call at render time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FnCall<'a> {
    /// Module name, the prefix before the dot.
    pub module: &'a str,
    /// Function name without the module prefix, as the `render` hook takes it.
    pub function: &'a str,
    /// The evaluated arguments.
    pub args: CallArgs<'a>,
}

/// Classes of function failure. The cases match `hook-error-kind`, so the
/// interpreter records the failure label `extension_<kind>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FnErrorKind {
    /// The arguments do not match the catalog entry of the function.
    InvalidArgument,
    /// Bytes from the peer, such as `last`, violate the protocol.
    Protocol,
    /// No module implements the function.
    Unsupported,
    /// Any other failure inside the module.
    Internal,
}

impl fmt::Display for FnErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FnErrorKind::InvalidArgument => "invalid_argument",
            FnErrorKind::Protocol => "protocol",
            FnErrorKind::Unsupported => "unsupported",
            FnErrorKind::Internal => "internal",
        })
    }
}

/// A failed module function call. It fails only the calling session.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct FnError {
    /// The failure class.
    pub kind: FnErrorKind,
    /// Lowercase sentence without trailing punctuation.
    pub message: String,
}

impl FnError {
    /// Builds an error of the given kind.
    pub fn new(kind: FnErrorKind, message: impl Into<String>) -> Self {
        FnError {
            kind,
            message: message.into(),
        }
    }
}

/// The dispatch seam between the template renderer and module code.
///
/// One value serves one session. The interpreter implements it over the
/// session's module table, which forwards each call to the `render` hook of
/// the named module. Native first party modules implement that hook in Rust
/// (decision D10), and community modules run behind the WebAssembly adapter.
pub trait TemplateFn {
    /// Renders `call` and appends the bytes to `out`.
    ///
    /// `rng` is the session generator that the rest of the template draws
    /// from, so a replay with the same run seed and session id reproduces the
    /// bytes. On error the bytes already appended to `out` are unspecified,
    /// because the session fails.
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError>;
}

/// A [`TemplateFn`] without modules, for sessions whose templates call no
/// module function. Every call fails with [`FnErrorKind::Unsupported`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFunctions;

impl TemplateFn for NoFunctions {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        _rng: &mut dyn RngCore,
        _out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        Err(FnError::new(
            FnErrorKind::Unsupported,
            format!("no module provides '{}.{}'", call.module, call.function),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_compatibility_follows_the_contract() {
        use ValueType::*;
        for (param, arg, ok) in [
            (Int, Int, true),
            (Float, Int, true),
            (Int, Float, false),
            (Bytes, Text, true),
            (Text, Bytes, false),
            (Any, Bytes, true),
            (Boolean, Any, true),
            (Boolean, Int, false),
        ] {
            assert_eq!(param.accepts(arg), ok, "{param} <- {arg}");
        }
    }

    #[test]
    fn no_functions_rejects_every_call() {
        let call = FnCall {
            module: "dns",
            function: "query",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let err = NoFunctions
            .render(&call, &mut rng, &mut Vec::new())
            .unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
        assert_eq!(
            err.to_string(),
            "unsupported: no module provides 'dns.query'"
        );
    }
}
