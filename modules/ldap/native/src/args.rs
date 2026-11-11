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
/// as bytes, and decimal text binds an int parameter.
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
        // The result of a nested call is bytes, and a number that a call
        // renders is its decimal text, so a nested call can feed an int.
        (ValueType::Int, ArgValue::Bytes(b)) => std::str::from_utf8(b)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .map(Bound::Int),
        (ValueType::Int, ArgValue::Text(t)) => t.trim().parse().ok().map(Bound::Int),
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
    pub(crate) fn boolean(&self, name: &str) -> Result<bool, FnError> {
        match self.get(name)? {
            Bound::Boolean(b) => Ok(b),
            _ => Err(self.mismatch(name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{BIND_REQUEST, FIELD, SEARCH_REQUEST};
    use fps_flow::function::NamedArg;

    fn args<'a>(positional: &'a [ArgValue<'a>], named: &'a [NamedArg<'a>]) -> CallArgs<'a> {
        CallArgs { positional, named }
    }

    #[test]
    fn fills_defaults_for_absent_parameters() {
        let p = [
            ArgValue::Int(1),
            ArgValue::Text("cn=a"),
            ArgValue::Text("pw"),
        ];
        let a = bind(&BIND_REQUEST, &args(&p, &[])).unwrap();
        assert_eq!(a.int("version").unwrap(), 3);
        let p = [ArgValue::Int(2), ArgValue::Text("dc=x")];
        let a = bind(&SEARCH_REQUEST, &args(&p, &[])).unwrap();
        assert_eq!(a.text("scope").unwrap(), "sub");
        assert_eq!(a.text("filter").unwrap(), "(objectClass=*)");
        assert!(!a.boolean("types_only").unwrap());
    }

    #[test]
    fn rejects_unknown_repeated_and_surplus_arguments() {
        let unknown = [NamedArg {
            name: "colour",
            value: ArgValue::Text("x"),
        }];
        let repeated = [NamedArg {
            name: "name",
            value: ArgValue::Text("u"),
        }];
        let surplus = [ArgValue::Int(1); 6];
        let two = [ArgValue::Int(1), ArgValue::Text("u")];
        for (p, n) in [
            (&two[..], &unknown[..]),
            (&two[..], &repeated[..]),
            (&surplus[..], &[][..]),
        ] {
            let err = bind(&BIND_REQUEST, &args(p, n)).unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{err}");
        }
    }

    #[test]
    fn checks_types_and_accepts_utf8_bytes_as_text_and_decimal_text_as_int() {
        let p = [ArgValue::Boolean(true)];
        let err = bind(&FIELD, &args(&p, &[])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid_argument: argument 'name' of 'field' must be text, not boolean"
        );
        let ok = [ArgValue::Bytes(b"message_id")];
        assert_eq!(
            bind(&FIELD, &args(&ok, &[])).unwrap().text("name").unwrap(),
            "message_id"
        );
        let bad = [ArgValue::Bytes(&[0xff, 0xfe])];
        assert_eq!(
            bind(&FIELD, &args(&bad, &[])).unwrap_err().kind,
            FnErrorKind::InvalidArgument
        );
        for (v, want) in [
            (ArgValue::Bytes(b"42"), Some(42)),
            (ArgValue::Text(" 7 "), Some(7)),
            (ArgValue::Int(-1), Some(-1)),
            (ArgValue::Text("x"), None),
            (ArgValue::Bytes(&[0xff]), None),
            (ArgValue::Boolean(true), None),
        ] {
            let p = [v, ArgValue::Text("cn=a"), ArgValue::Text("pw")];
            match (bind(&BIND_REQUEST, &args(&p, &[])), want) {
                (Ok(a), Some(w)) => assert_eq!(a.int("message_id").unwrap(), w),
                (Err(e), None) => assert_eq!(e.kind, FnErrorKind::InvalidArgument),
                (r, w) => panic!("{r:?} for {w:?}"),
            }
        }
    }
}
