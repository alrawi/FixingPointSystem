// SPDX-License-Identifier: GPL-3.0-only
//! Binding of call arguments to catalog parameters.
//!
//! The validator checks calls against the manifest before a run, but a value
//! that a `set` action bound has type `any` until render time, so the module
//! checks every argument again and fails only the calling session.

use fps_flow::{ArgValue, CallArgs, FnError, FnErrorKind, ValueType};

use crate::catalog::{DefaultValue, FunctionSpec};

/// One bound argument value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bound<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
    Int(i64),
    Boolean(bool),
}

/// The arguments of one call, one slot per catalog parameter.
#[derive(Debug)]
pub(crate) struct Args<'a> {
    function: &'static str,
    names: Vec<&'static str>,
    values: Vec<Bound<'a>>,
}

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn from_default(d: DefaultValue) -> Bound<'static> {
    match d {
        DefaultValue::Text(t) => Bound::Text(t),
        DefaultValue::Int(i) => Bound::Int(i),
        DefaultValue::Boolean(b) => Bound::Boolean(b),
        DefaultValue::Bytes(b) => Bound::Bytes(b),
    }
}

/// Converts `value` to the parameter type `ty` with the binding rules of the
/// contract section "Types". Bytes that are valid UTF-8 also bind a text
/// parameter, because values that a `set` action copied out of `last` arrive
/// as bytes.
fn coerce<'a>(
    function: &str,
    param: &str,
    ty: ValueType,
    value: ArgValue<'a>,
) -> Result<Bound<'a>, FnError> {
    let bound = match (ty, value) {
        (ValueType::Text, ArgValue::Text(t)) => Some(Bound::Text(t)),
        (ValueType::Text, ArgValue::Bytes(b)) => std::str::from_utf8(b).ok().map(Bound::Text),
        (ValueType::Bytes, ArgValue::Bytes(b)) => Some(Bound::Bytes(b)),
        (ValueType::Bytes, ArgValue::Text(t)) => Some(Bound::Bytes(t.as_bytes())),
        (ValueType::Int, ArgValue::Int(i)) => Some(Bound::Int(i)),
        (ValueType::Boolean, ArgValue::Boolean(b)) => Some(Bound::Boolean(b)),
        _ => None,
    };
    bound.ok_or_else(|| {
        invalid(format!(
            "argument '{param}' of '{function}' must be {ty}, not {}",
            value.value_type()
        ))
    })
}

/// Binds the arguments of a call to the parameters of `spec` and fills in the
/// defaults.
pub(crate) fn bind<'a>(spec: &FunctionSpec, args: &CallArgs<'a>) -> Result<Args<'a>, FnError> {
    let fname = spec.name;
    if args.positional.len() > spec.params.len() {
        return Err(invalid(format!(
            "'{fname}' takes at most {} arguments, got {}",
            spec.params.len(),
            args.positional.len()
        )));
    }
    let mut slots: Vec<Option<Bound<'a>>> = vec![None; spec.params.len()];
    for (i, v) in args.positional.iter().enumerate() {
        let p = &spec.params[i];
        slots[i] = Some(coerce(fname, p.name, p.ty, *v)?);
    }
    for named in args.named {
        let Some(i) = spec.params.iter().position(|p| p.name == named.name) else {
            return Err(invalid(format!(
                "'{fname}' has no parameter '{}'",
                named.name
            )));
        };
        if slots[i].is_some() {
            return Err(invalid(format!(
                "parameter '{}' of '{fname}' is given twice",
                named.name
            )));
        }
        let p = &spec.params[i];
        slots[i] = Some(coerce(fname, p.name, p.ty, named.value)?);
    }
    let mut values = Vec::with_capacity(slots.len());
    for (slot, p) in slots.into_iter().zip(spec.params) {
        match (slot, p.default) {
            (Some(v), _) => values.push(v),
            (None, Some(d)) => values.push(from_default(d)),
            (None, None) => {
                return Err(invalid(format!(
                    "'{fname}' needs the argument '{}'",
                    p.name
                )))
            }
        }
    }
    Ok(Args {
        function: fname,
        names: spec.params.iter().map(|p| p.name).collect(),
        values,
    })
}

impl<'a> Args<'a> {
    fn get(&self, name: &str) -> Result<Bound<'a>, FnError> {
        self.names
            .iter()
            .position(|n| *n == name)
            .and_then(|i| self.values.get(i).copied())
            .ok_or_else(|| {
                FnError::new(
                    FnErrorKind::Internal,
                    format!("'{}' has no parameter '{name}'", self.function),
                )
            })
    }

    fn mismatch(&self, name: &str) -> FnError {
        FnError::new(
            FnErrorKind::Internal,
            format!(
                "parameter '{name}' of '{}' bound the wrong type",
                self.function
            ),
        )
    }

    /// Returns a text argument.
    pub(crate) fn text(&self, name: &str) -> Result<&'a str, FnError> {
        match self.get(name)? {
            Bound::Text(t) => Ok(t),
            _ => Err(self.mismatch(name)),
        }
    }

    /// Returns a bytes argument.
    #[allow(dead_code)] // The catalog has no bytes parameter yet.
    pub(crate) fn bytes(&self, name: &str) -> Result<&'a [u8], FnError> {
        match self.get(name)? {
            Bound::Bytes(b) => Ok(b),
            _ => Err(self.mismatch(name)),
        }
    }

    /// Returns an integer argument.
    pub(crate) fn int(&self, name: &str) -> Result<i64, FnError> {
        match self.get(name)? {
            Bound::Int(i) => Ok(i),
            _ => Err(self.mismatch(name)),
        }
    }

    /// Returns a boolean argument.
    #[allow(dead_code)] // The catalog has no boolean parameter yet.
    pub(crate) fn boolean(&self, name: &str) -> Result<bool, FnError> {
        match self.get(name)? {
            Bound::Boolean(b) => Ok(b),
            _ => Err(self.mismatch(name)),
        }
    }
}
