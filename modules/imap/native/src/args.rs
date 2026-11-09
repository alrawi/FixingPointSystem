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
        (ValueType::Any, ArgValue::Text(t)) => Some(Bound::Text(t)),
        (ValueType::Any, ArgValue::Bytes(b)) => Some(Bound::Bytes(b)),
        (ValueType::Any, ArgValue::Int(i)) => Some(Bound::Int(i)),
        (ValueType::Any, ArgValue::Boolean(b)) => Some(Bound::Boolean(b)),
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

    /// Returns a bytes argument as text. A module call such as
    /// `smtp.address(...)` returns bytes, and the validator types it as bytes,
    /// so parameters that take addresses are bytes that must be UTF-8.
    pub(crate) fn utf8(&self, name: &str) -> Result<&'a str, FnError> {
        match self.get(name)? {
            Bound::Text(t) => Ok(t),
            Bound::Bytes(b) => std::str::from_utf8(b).map_err(|_| {
                invalid(format!(
                    "argument '{name}' of '{}' must be UTF-8 text",
                    self.function
                ))
            }),
            _ => Err(self.mismatch(name)),
        }
    }

    /// Returns an argument of type `any` as text. An integer becomes its
    /// decimal digits, and bytes must be UTF-8.
    pub(crate) fn any_text(&self, name: &str) -> Result<String, FnError> {
        match self.get(name)? {
            Bound::Text(t) => Ok(t.to_string()),
            Bound::Int(i) => Ok(i.to_string()),
            Bound::Bytes(b) => std::str::from_utf8(b).map(str::to_string).map_err(|_| {
                invalid(format!(
                    "argument '{name}' of '{}' must be UTF-8 text",
                    self.function
                ))
            }),
            Bound::Boolean(_) => Err(self.mismatch(name)),
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
    use crate::catalog::{AUTH_PLAIN, MESSAGE, TAGGED};
    use fps_flow::function::NamedArg;

    fn args<'a>(positional: &'a [ArgValue<'a>], named: &'a [NamedArg<'a>]) -> CallArgs<'a> {
        CallArgs { positional, named }
    }

    #[test]
    fn fills_defaults_for_absent_parameters() {
        let p = [ArgValue::Bytes(b"A001 NOOP\r\n")];
        let a = bind(&TAGGED, &args(&p, &[])).unwrap();
        assert_eq!(a.text("status").unwrap(), "OK");
        assert_eq!(a.text("code").unwrap(), "");
        assert!(!a.boolean("timing").unwrap());
    }

    #[test]
    fn rejects_unknown_repeated_and_surplus_arguments() {
        let unknown = [NamedArg {
            name: "colour",
            value: ArgValue::Text("x"),
        }];
        let repeated = [NamedArg {
            name: "user",
            value: ArgValue::Text("u"),
        }];
        let surplus = [ArgValue::Text("x"); 4];
        let two = [ArgValue::Text("u"), ArgValue::Text("p")];
        for (p, n) in [
            (&two[..], &unknown[..]),
            (&two[..1], &repeated[..]),
            (&surplus[..], &[][..]),
        ] {
            let err = bind(&AUTH_PLAIN, &args(p, n)).unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{err}");
        }
    }

    #[test]
    fn checks_types_and_accepts_text_as_bytes() {
        let p = [
            ArgValue::Text("a@example.test"),
            ArgValue::Text("b@example.test"),
            ArgValue::Text("s"),
            ArgValue::Int(1),
        ];
        let err = bind(&MESSAGE, &args(&p, &[])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid_argument: argument 'body' of 'message' must be bytes, not int"
        );
        let p = [
            ArgValue::Text("a@example.test"),
            ArgValue::Text("b@example.test"),
            ArgValue::Text("s"),
            ArgValue::Text("hi"),
        ];
        let a = bind(&MESSAGE, &args(&p, &[])).unwrap();
        assert_eq!(a.bytes("body").unwrap(), b"hi");
        assert_eq!(a.bytes("attachment").unwrap(), b"");
    }

    #[test]
    fn any_binds_integers_text_and_bytes() {
        use crate::catalog::NEXT;
        for (v, want) in [
            (ArgValue::Int(7), "7"),
            (ArgValue::Text("A001"), "A001"),
            (ArgValue::Bytes(b"12"), "12"),
        ] {
            let p = [v];
            let a = bind(&NEXT, &args(&p, &[])).unwrap();
            assert_eq!(a.any_text("value").unwrap(), want);
        }
        let p = [ArgValue::Boolean(true)];
        let a = bind(&NEXT, &args(&p, &[])).unwrap();
        assert!(a.any_text("value").is_err());
    }

    #[test]
    fn utf8_bytes_bind_text_and_other_bytes_do_not() {
        let ok = [ArgValue::Bytes(b"user"), ArgValue::Text("pw")];
        assert_eq!(
            bind(&AUTH_PLAIN, &args(&ok, &[]))
                .unwrap()
                .text("user")
                .unwrap(),
            "user"
        );
        let bad = [ArgValue::Bytes(&[0xff, 0xfe]), ArgValue::Text("pw")];
        assert_eq!(
            bind(&AUTH_PLAIN, &args(&bad, &[])).unwrap_err().kind,
            FnErrorKind::InvalidArgument
        );
    }
}
