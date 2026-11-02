// SPDX-License-Identifier: GPL-3.0-only
//! Template compiler and renderer for the contract v0.1 core grammar.
//!
//! Templates compile once per scenario into segments. Rendering appends bytes
//! to a caller supplied buffer, so a session reuses one allocation.

use std::collections::HashMap;
use std::fmt;

use rand::Rng;
use rand_pcg::Pcg64Mcg;
use serde_yaml::Value as Yaml;

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
    #[error("cannot parse expression '{0}'")]
    Syntax(String),
    /// A reference names an unknown root or session field.
    #[error("unknown reference '{0}'")]
    UnknownReference(String),
    /// A call names a function outside the core grammar.
    #[error("unknown function '{0}'")]
    UnknownFunction(String),
    /// A call names a module helper, which needs the extension host.
    #[error("module helper '{0}' needs an extension, which this engine version does not run")]
    ModuleHelper(String),
    /// A core function received the wrong number of arguments.
    #[error("function '{name}' got {got} argument(s)")]
    Arity {
        /// Function name.
        name: String,
        /// Number of arguments given.
        got: usize,
    },
    /// An argument has a form that the function does not take.
    #[error("'{0}' cannot be an argument here")]
    BadArgument(String),
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
            // The range check keeps the cast exact.
            Value::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(*f as i64),
            _ => None,
        }
    }
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
pub struct RenderCtx<'a, R: Rng + ?Sized> {
    /// Scenario variables.
    pub vars: &'a Vars,
    /// Variables bound by `set` during this session. They shadow `vars`.
    pub overlay: &'a Vars,
    /// Session id that `{{ session.id }}` renders.
    pub session_id: u64,
    /// Flow id that `{{ session.flow }}` renders.
    pub flow: &'a str,
    /// Session random number generator.
    pub rng: &'a mut R,
    /// Shared filler for `fill()`.
    pub filler: &'a Filler,
}

impl<R: Rng + ?Sized> RenderCtx<'_, R> {
    fn lookup(&self, name: &str) -> Result<&Value, TemplateError> {
        self.overlay
            .get(name)
            .or_else(|| self.vars.get(name))
            .ok_or_else(|| TemplateError::UndefinedVariable(name.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Arg {
    Int(i64),
    Str(String),
    Var(String),
    SessionId,
}

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Lit(Vec<u8>),
    Var(String),
    SessionId,
    SessionFlow,
    Fill(Arg),
    RandInt(Arg, Arg),
}

/// A compiled template.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    segs: Vec<Seg>,
}

impl Template {
    /// Returns a template that renders `bytes` unchanged.
    pub fn literal(bytes: impl Into<Vec<u8>>) -> Self {
        Template {
            segs: vec![Seg::Lit(bytes.into())],
        }
    }

    /// Compiles template source text.
    pub fn compile(src: &str) -> Result<Self, TemplateError> {
        let mut segs = Vec::new();
        let mut rest = src;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                push_lit(&mut segs, &rest.as_bytes()[..open]);
            }
            let after = &rest[open + 2..];
            let close = after.find("}}").ok_or(TemplateError::Unbalanced("{{"))?;
            compile_expr(after[..close].trim(), &mut segs)?;
            rest = &after[close + 2..];
        }
        if rest.contains("}}") {
            return Err(TemplateError::Unbalanced("}}"));
        }
        if !rest.is_empty() {
            push_lit(&mut segs, rest.as_bytes());
        }
        Ok(Template { segs })
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

    /// Returns true when the template renders the same bytes in every session.
    pub fn is_static(&self) -> bool {
        self.segs.iter().all(|s| matches!(s, Seg::Lit(_)))
    }

    /// Returns the scenario variables that the template reads, in order.
    pub fn variables(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for seg in &self.segs {
            match seg {
                Seg::Var(n) | Seg::Fill(Arg::Var(n)) => out.push(n.as_str()),
                Seg::RandInt(a, b) => {
                    for arg in [a, b] {
                        if let Arg::Var(n) = arg {
                            out.push(n.as_str());
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Renders the template and appends the bytes to `out`.
    pub fn render_into<R: Rng + ?Sized>(
        &self,
        ctx: &mut RenderCtx<'_, R>,
        out: &mut Vec<u8>,
    ) -> Result<(), TemplateError> {
        for seg in &self.segs {
            match seg {
                Seg::Lit(b) => out.extend_from_slice(b),
                Seg::Var(name) => {
                    let v = ctx.lookup(name)?;
                    out.extend_from_slice(v.to_string().as_bytes());
                }
                Seg::SessionId => out.extend_from_slice(ctx.session_id.to_string().as_bytes()),
                Seg::SessionFlow => out.extend_from_slice(ctx.flow.as_bytes()),
                Seg::Fill(arg) => {
                    let n = arg_int(arg, ctx)?;
                    let n = usize::try_from(n).map_err(|_| TemplateError::NegativeFill(n))?;
                    let filler = ctx.filler;
                    filler.fill(out, n, ctx.rng);
                }
                Seg::RandInt(a, b) => {
                    let (lo, hi) = (arg_int(a, ctx)?, arg_int(b, ctx)?);
                    if lo > hi {
                        return Err(TemplateError::EmptyRange { lo, hi });
                    }
                    let v = ctx.rng.gen_range(lo..=hi);
                    out.extend_from_slice(v.to_string().as_bytes());
                }
            }
        }
        Ok(())
    }

    /// Renders the template into a new vector.
    pub fn render_to_vec<R: Rng + ?Sized>(
        &self,
        ctx: &mut RenderCtx<'_, R>,
    ) -> Result<Vec<u8>, TemplateError> {
        let mut out = Vec::new();
        self.render_into(ctx, &mut out)?;
        Ok(out)
    }

    /// Renders the template and parses the result as an integer.
    pub fn render_int<R: Rng + ?Sized>(
        &self,
        ctx: &mut RenderCtx<'_, R>,
    ) -> Result<i64, TemplateError> {
        let bytes = self.render_to_vec(ctx)?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim();
        text.parse::<i64>()
            .map_err(|_| TemplateError::NotInteger(text.to_string()))
    }
}

fn push_lit(segs: &mut Vec<Seg>, bytes: &[u8]) {
    if let Some(Seg::Lit(prev)) = segs.last_mut() {
        prev.extend_from_slice(bytes);
    } else {
        segs.push(Seg::Lit(bytes.to_vec()));
    }
}

fn arg_int<R: Rng + ?Sized>(arg: &Arg, ctx: &RenderCtx<'_, R>) -> Result<i64, TemplateError> {
    match arg {
        Arg::Int(i) => Ok(*i),
        Arg::Var(name) => ctx
            .lookup(name)?
            .as_i64()
            .ok_or_else(|| TemplateError::NotInteger(format!("vars.{name}"))),
        Arg::SessionId => i64::try_from(ctx.session_id)
            .map_err(|_| TemplateError::NotInteger("session.id".into())),
        Arg::Str(s) => s.parse().map_err(|_| TemplateError::NotInteger(s.clone())),
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_ref(text: &str) -> Result<Seg, TemplateError> {
    let parts: Vec<&str> = text.split('.').collect();
    if !parts.iter().all(|p| is_ident(p)) {
        return Err(TemplateError::Syntax(text.to_string()));
    }
    match parts.as_slice() {
        ["vars", name] => Ok(Seg::Var((*name).to_string())),
        ["session", "id"] => Ok(Seg::SessionId),
        ["session", "flow"] => Ok(Seg::SessionFlow),
        _ => Err(TemplateError::UnknownReference(text.to_string())),
    }
}

fn parse_arg(text: &str) -> Result<Arg, TemplateError> {
    let text = text.trim();
    if let Ok(i) = text.parse::<i64>() {
        return Ok(Arg::Int(i));
    }
    if let Some(s) = quoted(text) {
        return Ok(Arg::Str(s.to_string()));
    }
    match parse_ref(text)? {
        Seg::Var(n) => Ok(Arg::Var(n)),
        Seg::SessionId => Ok(Arg::SessionId),
        _ => Err(TemplateError::BadArgument(text.to_string())),
    }
}

/// Returns the content of a single quoted literal.
fn quoted(text: &str) -> Option<&str> {
    text.strip_prefix('\'')?.strip_suffix('\'')
}

fn split_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in text.chars() {
        if ch == '\'' {
            in_quotes = !in_quotes;
        }
        if ch == ',' && !in_quotes {
            args.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        args.push(cur.trim().to_string());
    }
    args
}

fn compile_expr(expr: &str, segs: &mut Vec<Seg>) -> Result<(), TemplateError> {
    if expr.is_empty() {
        return Err(TemplateError::EmptyPlaceholder);
    }
    if let Ok(i) = expr.parse::<i64>() {
        push_lit(segs, i.to_string().as_bytes());
        return Ok(());
    }
    if let Some(s) = quoted(expr) {
        push_lit(segs, s.as_bytes());
        return Ok(());
    }
    if let Some(open) = expr.find('(') {
        let inner = expr[open + 1..]
            .strip_suffix(')')
            .ok_or_else(|| TemplateError::Syntax(expr.to_string()))?;
        let name = expr[..open].trim();
        let args = split_args(inner);
        match (name, args.as_slice()) {
            ("fill", [n]) => segs.push(Seg::Fill(parse_arg(n)?)),
            ("rand_int", [lo, hi]) => segs.push(Seg::RandInt(parse_arg(lo)?, parse_arg(hi)?)),
            ("hex", [s]) => match parse_arg(s)? {
                Arg::Str(s) => push_lit(segs, &decode_hex(&s)?),
                _ => return Err(TemplateError::BadArgument(s.clone())),
            },
            (n, _) if n.contains('.') => return Err(TemplateError::ModuleHelper(n.to_string())),
            ("fill" | "hex" | "rand_int", a) => {
                return Err(TemplateError::Arity {
                    name: name.to_string(),
                    got: a.len(),
                })
            }
            (n, _) => return Err(TemplateError::UnknownFunction(n.to_string())),
        }
        return Ok(());
    }
    segs.push(parse_ref(expr)?);
    Ok(())
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
mod tests {
    use super::*;

    fn render(t: &str, vars: &Vars) -> Result<Vec<u8>, TemplateError> {
        let filler = Filler::new(1);
        let mut rng = Pcg64Mcg::new(9);
        let overlay = Vars::new();
        let mut ctx = RenderCtx {
            vars,
            overlay: &overlay,
            session_id: 42,
            flow: "page",
            rng: &mut rng,
            filler: &filler,
        };
        Template::compile(t)?.render_to_vec(&mut ctx)
    }

    fn vars() -> Vars {
        let mut v = Vars::new();
        v.insert("host".into(), Value::Str("example.test".into()));
        v.insert("n".into(), Value::Int(5));
        v
    }

    #[test]
    fn literal_and_vars() {
        assert_eq!(
            render("GET / HTTP/1.1\r\nHost: {{ vars.host }}\r\n", &vars()).unwrap(),
            b"GET / HTTP/1.1\r\nHost: example.test\r\n"
        );
    }

    #[test]
    fn session_fields() {
        assert_eq!(
            render("{{session.id}}-{{ session.flow }}", &vars()).unwrap(),
            b"42-page"
        );
    }

    #[test]
    fn fill_length_and_printable() {
        let out = render("{{ fill(vars.n) }}|{{ fill(3000000) }}", &vars()).unwrap();
        assert_eq!(out.len(), 5 + 1 + 3_000_000);
        assert!(out.iter().all(|b| b.is_ascii()));
    }

    #[test]
    fn hex_decodes_at_compile_time() {
        let t = Template::compile("{{ hex('00ff10') }}").unwrap();
        assert!(t.is_static());
        assert_eq!(
            render("{{ hex('00ff10') }}", &vars()).unwrap(),
            vec![0, 255, 16]
        );
    }

    #[test]
    fn rand_int_in_range() {
        for _ in 0..50 {
            let out = render("{{ rand_int(3, 4) }}", &vars()).unwrap();
            assert!(out == b"3" || out == b"4");
        }
    }

    #[test]
    fn errors() {
        assert_eq!(
            Template::compile("{{ nope(1) }}"),
            Err(TemplateError::UnknownFunction("nope".into()))
        );
        assert_eq!(
            Template::compile("{{ fill(1, 2) }}"),
            Err(TemplateError::Arity {
                name: "fill".into(),
                got: 2
            })
        );
        assert_eq!(
            Template::compile("{{ vars.x "),
            Err(TemplateError::Unbalanced("{{"))
        );
        assert_eq!(
            Template::compile("x }}"),
            Err(TemplateError::Unbalanced("}}"))
        );
        assert!(matches!(
            Template::compile("{{ hex('abc') }}"),
            Err(TemplateError::Hex(_))
        ));
        assert_eq!(
            Template::compile("{{ http1.request('GET') }}"),
            Err(TemplateError::ModuleHelper("http1.request".into()))
        );
        assert_eq!(
            render("{{ vars.missing }}", &vars()),
            Err(TemplateError::UndefinedVariable("missing".into()))
        );
        assert_eq!(
            render("{{ fill(-1) }}", &vars()),
            Err(TemplateError::NegativeFill(-1))
        );
        assert_eq!(
            render("{{ rand_int(4, 3) }}", &vars()),
            Err(TemplateError::EmptyRange { lo: 4, hi: 3 })
        );
    }

    #[test]
    fn render_int() {
        let filler = Filler::new(1);
        let mut rng = Pcg64Mcg::new(1);
        let v = vars();
        let o = Vars::new();
        let mut ctx = RenderCtx {
            vars: &v,
            overlay: &o,
            session_id: 0,
            flow: "f",
            rng: &mut rng,
            filler: &filler,
        };
        assert_eq!(
            Template::compile("{{ vars.n }}")
                .unwrap()
                .render_int(&mut ctx)
                .unwrap(),
            5
        );
        assert!(Template::compile("{{ vars.host }}")
            .unwrap()
            .render_int(&mut ctx)
            .is_err());
    }

    #[test]
    fn overlay_shadows_scenario_vars() {
        let filler = Filler::new(1);
        let mut rng = Pcg64Mcg::new(1);
        let v = vars();
        let mut o = Vars::new();
        o.insert("n".into(), Value::Int(7));
        let mut ctx = RenderCtx {
            vars: &v,
            overlay: &o,
            session_id: 0,
            flow: "f",
            rng: &mut rng,
            filler: &filler,
        };
        let t = Template::compile("{{ vars.n }}").unwrap();
        assert_eq!(t.render_int(&mut ctx).unwrap(), 7);
    }
}
