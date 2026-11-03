// SPDX-License-Identifier: GPL-3.0-only
//! Template compiler and renderer for the contract v0.2 grammar.
//!
//! Templates compile once per scenario into segments. A segment is either a
//! literal byte string or an expression slot. The compiler folds every part
//! that renders the same bytes in every session into the literal segments, so
//! a template without session dependent parts renders with one copy.
//! [`Template::fold_vars`] repeats the folding once the run fixes the scenario
//! variables.
//!
//! Rendering appends bytes to a caller supplied buffer, so a session reuses
//! one allocation. Module calls go through the [`TemplateFn`] seam.

mod check;
mod dump;
mod parse;

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use rand::{Rng, RngCore};
use rand_pcg::Pcg64Mcg;
use serde_yaml::Value as Yaml;

use crate::function::{ArgValue, CallArgs, FnCall, FnErrorKind, NamedArg, TemplateFn, ValueType};

pub use check::{CheckError, CheckScope, CheckWarning};
pub use parse::MAX_CALL_DEPTH;

/// Errors that template compilation or rendering reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// A `{{` has no matching `}}`, or a `}}` has no matching `{{`.
    #[error("unbalanced '{0}' in template")]
    Unbalanced(&'static str),
    /// A placeholder holds no expression.
    #[error("empty placeholder")]
    EmptyPlaceholder,
    /// An expression does not follow the grammar.
    #[error("cannot parse expression '{expr}': {reason}")]
    Syntax {
        /// The expression text.
        expr: String,
        /// What the parser expected.
        reason: String,
    },
    /// Calls nest deeper than [`MAX_CALL_DEPTH`].
    #[error("calls nest deeper than {max} levels")]
    TooDeep {
        /// The limit.
        max: usize,
    },
    /// A reference names an unknown root or session field.
    #[error("unknown reference '{0}'")]
    UnknownReference(String),
    /// A call without module prefix names a function outside the core grammar.
    #[error("unknown function '{0}'")]
    UnknownFunction(String),
    /// A call uses a reserved root as its module name.
    #[error("'{0}' is reserved and cannot name a module")]
    ReservedModule(String),
    /// A call has more positional arguments than the function has parameters.
    #[error("function '{name}' takes at most {max} positional argument(s), got {got}")]
    Arity {
        /// Function name.
        name: String,
        /// Number of parameters.
        max: usize,
        /// Number of positional arguments given.
        got: usize,
    },
    /// A keyword names no parameter of the function.
    #[error("function '{function}' has no parameter '{param}'")]
    UnknownParameter {
        /// Function name.
        function: String,
        /// The keyword.
        param: String,
    },
    /// A call names one parameter twice.
    #[error("function '{function}' got parameter '{param}' twice")]
    DuplicateParameter {
        /// Function name, with the module prefix for module calls.
        function: String,
        /// The repeated parameter.
        param: String,
    },
    /// A call omits a required parameter.
    #[error("function '{function}' misses required parameter '{param}'")]
    MissingParameter {
        /// Function name.
        function: String,
        /// The missing parameter.
        param: String,
    },
    /// An argument has a type that cannot bind the parameter.
    #[error("parameter '{param}' of '{function}' takes {expected}, got {got}")]
    ArgumentType {
        /// Function name.
        function: String,
        /// Parameter name.
        param: String,
        /// Declared parameter type.
        expected: ValueType,
        /// Type of the argument.
        got: ValueType,
    },
    /// A `hex()` literal is malformed.
    #[error("bad hex literal: {0}")]
    Hex(String),
    /// A rendered variable is not defined in the scenario or the session.
    #[error("undefined variable '{0}'")]
    UndefinedVariable(String),
    /// A value that must be an integer is not one.
    #[error("'{0}' is not an integer")]
    NotInteger(String),
    /// `fill()` received a negative size.
    #[error("fill() needs a non negative size, got {0}")]
    NegativeFill(i64),
    /// `rand_int()` received a lower bound above its upper bound.
    #[error("rand_int({lo}, {hi}) has an empty range")]
    EmptyRange {
        /// Lower bound.
        lo: i64,
        /// Upper bound.
        hi: i64,
    },
    /// A YAML value has a type that a variable or count cannot take.
    #[error("unsupported value {0}")]
    UnsupportedValue(String),
    /// A module function failed.
    #[error("{module}.{function}: {kind}: {message}")]
    Function {
        /// Module name.
        module: String,
        /// Function name.
        function: String,
        /// Failure class, which names the failure label.
        kind: FnErrorKind,
        /// Message of the module.
        message: String,
    },
}

/// A scenario variable value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An integer.
    Int(i64),
    /// A floating point number.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// A string.
    Str(String),
}

impl Value {
    /// Converts a YAML scalar into a value.
    pub fn from_yaml(v: &Yaml) -> Result<Self, TemplateError> {
        Ok(match v {
            Yaml::Bool(b) => Value::Bool(*b),
            Yaml::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => Value::Int(i),
                (None, Some(f)) => Value::Float(f),
                (None, None) => return Err(TemplateError::UnsupportedValue(n.to_string())),
            },
            Yaml::String(s) => Value::Str(s.clone()),
            other => return Err(TemplateError::UnsupportedValue(format!("{other:?}"))),
        })
    }

    /// Parses a command line override or a `set` result. Integers stay
    /// integers, then floats, then booleans, and anything else is a string.
    pub fn parse_override(text: &str) -> Self {
        if let Ok(i) = text.parse::<i64>() {
            Value::Int(i)
        } else if let Ok(f) = text.parse::<f64>() {
            Value::Float(f)
        } else if text == "true" || text == "false" {
            Value::Bool(text == "true")
        } else {
            Value::Str(text.to_string())
        }
    }

    /// Returns the value as an integer when it has an exact integer form.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Str(s) => s.trim().parse().ok(),
            Value::Float(f) => float_as_i64(*f),
            Value::Bool(_) => None,
        }
    }
}

/// Converts a float with an exact integer form.
fn float_as_i64(f: f64) -> Option<i64> {
    // The range check keeps the cast exact.
    (f.fract() == 0.0 && f.abs() < 9.0e15).then_some(f as i64)
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => f.write_str(s),
        }
    }
}

/// Variables by name.
pub type Vars = HashMap<String, Value>;

/// Deterministic printable filler for `fill()`.
///
/// The filler holds [`Filler::SIZE`] bytes generated from a seed, and
/// [`Filler::fill`] copies from a random offset, so payloads differ between
/// sessions without a random number per byte.
pub struct Filler {
    buf: Vec<u8>,
}

impl Filler {
    /// Size of the filler buffer in bytes.
    pub const SIZE: usize = 1 << 20;

    /// Builds the filler from a seed. The same seed always gives the same bytes.
    pub fn new(seed: u64) -> Self {
        const ALPHABET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .,;:-_<>/=\"'\n";
        let mut rng = Pcg64Mcg::new(u128::from(seed));
        let buf = (0..Self::SIZE)
            .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
            .collect();
        Filler { buf }
    }

    /// Appends `n` filler bytes to `out`, starting at an offset drawn from `rng`.
    pub fn fill<R: Rng + ?Sized>(&self, out: &mut Vec<u8>, mut n: usize, rng: &mut R) {
        let mut off = rng.gen_range(0..self.buf.len());
        out.reserve(n);
        while n > 0 {
            let take = n.min(self.buf.len() - off);
            out.extend_from_slice(&self.buf[off..off + take]);
            n -= take;
            off = 0;
        }
    }
}

/// Everything a render needs besides the template itself.
pub struct RenderCtx<'a, R: RngCore> {
    /// Scenario variables.
    pub vars: &'a Vars,
    /// Variables bound by `set` during this session. They shadow `vars`.
    pub overlay: &'a Vars,
    /// Session id that `{{ session.id }}` renders.
    pub session_id: u64,
    /// Flow id that `{{ session.flow }}` renders.
    pub flow: &'a str,
    /// The bytes that the previous `expect` of this side consumed.
    pub last: &'a [u8],
    /// Session random number generator.
    pub rng: &'a mut R,
    /// Shared filler for `fill()`.
    pub filler: &'a Filler,
    /// Module functions of the session.
    pub functions: &'a mut dyn TemplateFn,
}

impl<'a, R: RngCore> RenderCtx<'a, R> {
    fn lookup(&self, name: &str) -> Result<&'a Value, TemplateError> {
        let (overlay, vars): (&'a Vars, &'a Vars) = (self.overlay, self.vars);
        overlay
            .get(name)
            .or_else(|| vars.get(name))
            .ok_or_else(|| TemplateError::UndefinedVariable(name.to_string()))
    }
}

/// A value that the compiler knows without a session.
#[derive(Clone, Debug, PartialEq)]
enum Const {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
}

impl Const {
    fn from_value(v: &Value) -> Self {
        match v {
            Value::Int(i) => Const::Int(*i),
            Value::Float(f) => Const::Float(*f),
            Value::Bool(b) => Const::Bool(*b),
            Value::Str(s) => Const::Text(s.clone()),
        }
    }

    fn value_type(&self) -> ValueType {
        match self {
            Const::Int(_) => ValueType::Int,
            Const::Float(_) => ValueType::Float,
            Const::Bool(_) => ValueType::Boolean,
            Const::Text(_) => ValueType::Text,
            Const::Bytes(_) => ValueType::Bytes,
        }
    }

    fn val(&self) -> Val<'_> {
        match self {
            Const::Int(i) => Val::Int(*i),
            Const::Float(f) => Val::Float(*f),
            Const::Bool(b) => Val::Bool(*b),
            Const::Text(s) => Val::Text(Cow::Borrowed(s)),
            Const::Bytes(b) => Val::Bytes(Cow::Borrowed(b)),
        }
    }
}

/// A module function call slot.
#[derive(Clone, Debug, PartialEq)]
struct ModuleCall {
    module: String,
    function: String,
    positional: Vec<Expr>,
    named: Vec<(String, Expr)>,
}

/// A checked expression. Core function arguments are bound to their
/// parameters in declaration order.
#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Const(Const),
    Var(String),
    SessionId,
    SessionFlow,
    Last,
    Fill(Box<Expr>),
    Hex(Box<Expr>),
    RandInt(Box<Expr>, Box<Expr>),
    Call(Box<ModuleCall>),
}

impl Expr {
    /// The static type, with [`ValueType::Any`] for variables.
    fn value_type(&self) -> ValueType {
        match self {
            Expr::Const(c) => c.value_type(),
            Expr::Var(_) => ValueType::Any,
            Expr::SessionId | Expr::RandInt(..) => ValueType::Int,
            Expr::SessionFlow | Expr::Fill(_) => ValueType::Text,
            Expr::Last | Expr::Hex(_) | Expr::Call(_) => ValueType::Bytes,
        }
    }

    /// Folds the parts that need no session. `constants` holds the variables
    /// that may be substituted.
    fn fold(self, constants: Option<&Vars>) -> Result<Expr, TemplateError> {
        let fold_box = |e: Box<Expr>| e.fold(constants).map(Box::new);
        Ok(match self {
            Expr::Var(name) => match constants.and_then(|c| c.get(&name)) {
                Some(v) => Expr::Const(Const::from_value(v)),
                None => Expr::Var(name),
            },
            Expr::Hex(s) => match *fold_box(s)? {
                Expr::Const(Const::Text(s)) => Expr::Const(Const::Bytes(decode_hex(&s)?)),
                other => Expr::Hex(Box::new(other)),
            },
            Expr::Fill(n) => Expr::Fill(fold_box(n)?),
            Expr::RandInt(a, b) => Expr::RandInt(fold_box(a)?, fold_box(b)?),
            Expr::Call(call) => {
                let ModuleCall {
                    module,
                    function,
                    positional,
                    named,
                } = *call;
                Expr::Call(Box::new(ModuleCall {
                    module,
                    function,
                    positional: positional
                        .into_iter()
                        .map(|e| e.fold(constants))
                        .collect::<Result<_, _>>()?,
                    named: named
                        .into_iter()
                        .map(|(k, e)| e.fold(constants).map(|e| (k, e)))
                        .collect::<Result<_, _>>()?,
                }))
            }
            other @ (Expr::Const(_) | Expr::SessionId | Expr::SessionFlow | Expr::Last) => other,
        })
    }

    fn visit<'e>(&'e self, f: &mut dyn FnMut(&'e Expr)) {
        f(self);
        match self {
            Expr::Fill(e) | Expr::Hex(e) => e.visit(f),
            Expr::RandInt(a, b) => {
                a.visit(f);
                b.visit(f);
            }
            Expr::Call(c) => {
                for e in c.positional.iter().chain(c.named.iter().map(|(_, e)| e)) {
                    e.visit(f);
                }
            }
            Expr::Const(_) | Expr::Var(_) | Expr::SessionId | Expr::SessionFlow | Expr::Last => {}
        }
    }

    /// Names the expression in error messages.
    fn label(&self) -> String {
        match self {
            Expr::Var(n) => format!("vars.{n}"),
            Expr::SessionId => "session.id".into(),
            Expr::SessionFlow => "session.flow".into(),
            Expr::Last => "last".into(),
            Expr::Fill(_) => "fill()".into(),
            Expr::Hex(_) => "hex()".into(),
            Expr::RandInt(..) => "rand_int()".into(),
            Expr::Call(c) => format!("{}.{}()", c.module, c.function),
            Expr::Const(c) => {
                let mut out = Vec::new();
                c.val().write_to(&mut out);
                String::from_utf8_lossy(&out).into_owned()
            }
        }
    }
}

/// A value during rendering, borrowed where possible.
#[derive(Debug)]
enum Val<'v> {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(Cow<'v, str>),
    Bytes(Cow<'v, [u8]>),
}

impl Val<'_> {
    fn from_value(v: &Value) -> Val<'_> {
        match v {
            Value::Int(i) => Val::Int(*i),
            Value::Float(f) => Val::Float(*f),
            Value::Bool(b) => Val::Bool(*b),
            Value::Str(s) => Val::Text(Cow::Borrowed(s)),
        }
    }

    fn arg(&self) -> ArgValue<'_> {
        match self {
            Val::Int(i) => ArgValue::Int(*i),
            Val::Float(f) => ArgValue::Float(*f),
            Val::Bool(b) => ArgValue::Boolean(*b),
            Val::Text(s) => ArgValue::Text(s),
            Val::Bytes(b) => ArgValue::Bytes(b),
        }
    }

    fn write_to(&self, out: &mut Vec<u8>) {
        use std::io::Write;
        // Writing into a Vec cannot fail.
        let _ = match self {
            Val::Int(i) => write!(out, "{i}"),
            Val::Float(f) => write!(out, "{f}"),
            Val::Bool(b) => write!(out, "{b}"),
            Val::Text(s) => out.write_all(s.as_bytes()),
            Val::Bytes(b) => out.write_all(b),
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Lit(Vec<u8>),
    Expr(Expr),
}

/// A compiled template.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    segs: Vec<Seg>,
}

/// One module call that a template makes, for checks against the function
/// catalog before a run starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSite<'t> {
    /// Module name.
    pub module: &'t str,
    /// Function name without the module prefix.
    pub function: &'t str,
    /// Number of positional arguments.
    pub positional: usize,
    /// Keyword names in call order.
    pub keywords: Vec<&'t str>,
}

impl Template {
    /// Returns a template that renders `bytes` unchanged.
    pub fn literal(bytes: impl Into<Vec<u8>>) -> Self {
        Template {
            segs: vec![Seg::Lit(bytes.into())],
        }
    }

    /// Compiles template source text and folds every part that needs no
    /// session.
    pub fn compile(src: &str) -> Result<Self, TemplateError> {
        let mut t = Template { segs: Vec::new() };
        let mut rest = src;
        while let Some(open) = rest.find("{{") {
            let (text, after) = rest.split_at(open);
            t.push_lit(text.as_bytes());
            let after = after.get(2..).unwrap_or_default();
            let close = after.find("}}").ok_or(TemplateError::Unbalanced("{{"))?;
            let (expr, tail) = after.split_at(close);
            let expr = expr.trim();
            if expr.is_empty() {
                return Err(TemplateError::EmptyPlaceholder);
            }
            t.push_expr(parse::parse_expression(expr)?.fold(None)?);
            rest = tail.get(2..).unwrap_or_default();
        }
        if rest.contains("}}") {
            return Err(TemplateError::Unbalanced("}}"));
        }
        t.push_lit(rest.as_bytes());
        Ok(t)
    }

    /// Compiles a YAML scalar that is either an integer or a template.
    pub fn from_yaml_count(v: &Yaml) -> Result<Self, TemplateError> {
        match v {
            Yaml::Number(n) => n
                .as_i64()
                .map(|i| Template::literal(i.to_string()))
                .ok_or_else(|| TemplateError::NotInteger(n.to_string())),
            Yaml::String(s) => Template::compile(s),
            other => Err(TemplateError::UnsupportedValue(format!("{other:?}"))),
        }
    }

    /// Returns a copy in which every reference to a variable in `constants`
    /// is replaced by its value, folded again.
    ///
    /// The caller passes the scenario variables after test parameter
    /// overrides, without the variables that any `set` action binds, because
    /// those differ between sessions. The result renders the same bytes as
    /// the original for every session and draws the same random numbers.
    pub fn fold_vars(&self, constants: &Vars) -> Result<Self, TemplateError> {
        let mut t = Template { segs: Vec::new() };
        for seg in &self.segs {
            match seg {
                Seg::Lit(b) => t.push_lit(b),
                Seg::Expr(e) => t.push_expr(e.clone().fold(Some(constants))?),
            }
        }
        Ok(t)
    }

    fn push_lit(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(Seg::Lit(prev)) = self.segs.last_mut() {
            prev.extend_from_slice(bytes);
        } else {
            self.segs.push(Seg::Lit(bytes.to_vec()));
        }
    }

    fn push_expr(&mut self, expr: Expr) {
        if let Expr::Const(c) = &expr {
            let mut bytes = Vec::new();
            c.val().write_to(&mut bytes);
            self.push_lit(&bytes);
        } else {
            self.segs.push(Seg::Expr(expr));
        }
    }

    /// Returns true when the template renders the same bytes in every session.
    pub fn is_static(&self) -> bool {
        self.segs.iter().all(|s| matches!(s, Seg::Lit(_)))
    }

    /// Returns the bytes of a static template, and `None` otherwise.
    pub fn as_static(&self) -> Option<&[u8]> {
        match self.segs.as_slice() {
            [] => Some(&[]),
            [Seg::Lit(b)] => Some(b),
            _ => None,
        }
    }

    /// Returns true when the template refers to `last`.
    pub fn uses_last(&self) -> bool {
        let mut found = false;
        self.visit(&mut |e| found |= matches!(e, Expr::Last));
        found
    }

    fn visit<'e>(&'e self, f: &mut dyn FnMut(&'e Expr)) {
        for seg in &self.segs {
            if let Seg::Expr(e) = seg {
                e.visit(f);
            }
        }
    }

    /// Returns the scenario variables that the template reads, in order.
    pub fn variables(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.visit(&mut |e| {
            if let Expr::Var(n) = e {
                out.push(n.as_str());
            }
        });
        out
    }

    /// Returns the module calls of the template, outer calls before the
    /// calls in their arguments.
    pub fn module_calls(&self) -> Vec<CallSite<'_>> {
        let mut out = Vec::new();
        self.visit(&mut |e| {
            if let Expr::Call(c) = e {
                out.push(CallSite {
                    module: &c.module,
                    function: &c.function,
                    positional: c.positional.len(),
                    keywords: c.named.iter().map(|(k, _)| k.as_str()).collect(),
                });
            }
        });
        out
    }

    /// Renders the template and appends the bytes to `out`.
    ///
    /// On error the bytes already appended are unspecified.
    pub fn render_into<R: RngCore>(
        &self,
        ctx: &mut RenderCtx<'_, R>,
        out: &mut Vec<u8>,
    ) -> Result<(), TemplateError> {
        for seg in &self.segs {
            match seg {
                Seg::Lit(b) => out.extend_from_slice(b),
                Seg::Expr(e) => render_expr(e, ctx, out)?,
            }
        }
        Ok(())
    }

    /// Renders the template into a new vector.
    pub fn render_to_vec<R: RngCore>(
        &self,
        ctx: &mut RenderCtx<'_, R>,
    ) -> Result<Vec<u8>, TemplateError> {
        let mut out = Vec::new();
        self.render_into(ctx, &mut out)?;
        Ok(out)
    }

    /// Renders the template and parses the result as an integer.
    pub fn render_int<R: RngCore>(&self, ctx: &mut RenderCtx<'_, R>) -> Result<i64, TemplateError> {
        let bytes = self.render_to_vec(ctx)?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim();
        text.parse::<i64>()
            .map_err(|_| TemplateError::NotInteger(text.to_string()))
    }
}

/// Renders a top level expression straight into `out`.
fn render_expr<R: RngCore>(
    e: &Expr,
    ctx: &mut RenderCtx<'_, R>,
    out: &mut Vec<u8>,
) -> Result<(), TemplateError> {
    match e {
        Expr::SessionId => Val::Text(Cow::Owned(ctx.session_id.to_string())).write_to(out),
        Expr::Fill(n) => fill(n, ctx, out)?,
        Expr::Call(c) => call_module(c, ctx, out)?,
        other => eval(other, ctx)?.write_to(out),
    }
    Ok(())
}

fn fill<R: RngCore>(
    n: &Expr,
    ctx: &mut RenderCtx<'_, R>,
    out: &mut Vec<u8>,
) -> Result<(), TemplateError> {
    let n = eval_int(n, ctx)?;
    let n = usize::try_from(n).map_err(|_| TemplateError::NegativeFill(n))?;
    let filler = ctx.filler;
    filler.fill(out, n, ctx.rng);
    Ok(())
}

/// Evaluates an argument expression.
fn eval<'v, 'a: 'v, R: RngCore>(
    e: &'v Expr,
    ctx: &mut RenderCtx<'a, R>,
) -> Result<Val<'v>, TemplateError> {
    Ok(match e {
        Expr::Const(c) => c.val(),
        Expr::Var(name) => Val::from_value(ctx.lookup(name)?),
        Expr::SessionId => Val::Int(
            i64::try_from(ctx.session_id)
                .map_err(|_| TemplateError::NotInteger("session.id".into()))?,
        ),
        Expr::SessionFlow => Val::Text(Cow::Borrowed(ctx.flow)),
        Expr::Last => Val::Bytes(Cow::Borrowed(ctx.last)),
        Expr::Fill(n) => {
            let mut buf = Vec::new();
            fill(n, ctx, &mut buf)?;
            // The filler alphabet is ASCII, so the conversion never fails.
            Val::Text(Cow::Owned(String::from_utf8(buf).unwrap_or_default()))
        }
        Expr::Hex(s) => {
            let v = eval(s, ctx)?;
            match v {
                Val::Text(s) => Val::Bytes(Cow::Owned(decode_hex(&s)?)),
                other => {
                    return Err(TemplateError::ArgumentType {
                        function: "hex".into(),
                        param: "s".into(),
                        expected: ValueType::Text,
                        got: other.arg().value_type(),
                    })
                }
            }
        }
        Expr::RandInt(a, b) => {
            let (lo, hi) = (eval_int(a, ctx)?, eval_int(b, ctx)?);
            if lo > hi {
                return Err(TemplateError::EmptyRange { lo, hi });
            }
            Val::Int(ctx.rng.gen_range(lo..=hi))
        }
        Expr::Call(c) => {
            let mut buf = Vec::new();
            call_module(c, ctx, &mut buf)?;
            Val::Bytes(Cow::Owned(buf))
        }
    })
}

fn eval_int<R: RngCore>(e: &Expr, ctx: &mut RenderCtx<'_, R>) -> Result<i64, TemplateError> {
    let v = eval(e, ctx)?;
    match v {
        Val::Int(i) => Some(i),
        Val::Text(s) => s.trim().parse().ok(),
        Val::Float(f) => float_as_i64(f),
        Val::Bool(_) | Val::Bytes(_) => None,
    }
    .ok_or_else(|| TemplateError::NotInteger(e.label()))
}

/// Evaluates the arguments of a module call in call order and dispatches it.
fn call_module<R: RngCore>(
    c: &ModuleCall,
    ctx: &mut RenderCtx<'_, R>,
    out: &mut Vec<u8>,
) -> Result<(), TemplateError> {
    let mut pos_vals = Vec::with_capacity(c.positional.len());
    for e in &c.positional {
        pos_vals.push(eval(e, ctx)?);
    }
    let mut named_vals = Vec::with_capacity(c.named.len());
    for (k, e) in &c.named {
        named_vals.push((k.as_str(), eval(e, ctx)?));
    }
    let positional: Vec<ArgValue<'_>> = pos_vals.iter().map(Val::arg).collect();
    let named: Vec<NamedArg<'_>> = named_vals
        .iter()
        .map(|(name, v)| NamedArg {
            name,
            value: v.arg(),
        })
        .collect();
    let call = FnCall {
        module: &c.module,
        function: &c.function,
        args: CallArgs {
            positional: &positional,
            named: &named,
        },
    };
    ctx.functions
        .render(&call, &mut *ctx.rng, out)
        .map_err(|e| TemplateError::Function {
            module: c.module.clone(),
            function: c.function.clone(),
            kind: e.kind,
            message: e.message,
        })
}

/// Decodes a hexadecimal string and ignores white space between digits.
pub fn decode_hex(s: &str) -> Result<Vec<u8>, TemplateError> {
    let digits: Vec<u8> = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| {
            c.to_digit(16)
                .and_then(|d| u8::try_from(d).ok())
                .ok_or_else(|| TemplateError::Hex(format!("'{c}' is not a hex digit")))
        })
        .collect::<Result<_, _>>()?;
    if !digits.len().is_multiple_of(2) {
        return Err(TemplateError::Hex("odd number of digits".into()));
    }
    let (pairs, _) = digits.as_chunks::<2>();
    Ok(pairs.iter().map(|[hi, lo]| (hi << 4) | lo).collect())
}

#[cfg(test)]
mod tests;
