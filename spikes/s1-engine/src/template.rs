//! Template compiler and renderer for the contract v0.1 core grammar.
//!
//! Templates compile once per scenario into segments. Rendering appends bytes
//! to a caller supplied buffer, so a session reuses one allocation.

use std::collections::HashMap;
use std::fmt;

use anyhow::{anyhow, bail, Context, Result};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

/// A scenario variable value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Value {
    pub fn from_yaml(v: &serde_yaml::Value) -> Result<Self> {
        Ok(match v {
            serde_yaml::Value::Bool(b) => Value::Bool(*b),
            serde_yaml::Value::Number(n) => match n.as_i64() {
                Some(i) => Value::Int(i),
                None => Value::Float(
                    n.as_f64()
                        .ok_or_else(|| anyhow!("unsupported number {n}"))?,
                ),
            },
            serde_yaml::Value::String(s) => Value::Str(s.clone()),
            other => bail!("unsupported variable value {other:?}"),
        })
    }

    /// Parse a command line override. Integers stay integers.
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

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Str(s) => s.trim().parse().ok(),
            Value::Float(f) if f.fract() == 0.0 => Some(*f as i64),
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

pub type Vars = HashMap<String, Value>;

/// Deterministic printable filler. `fill(n)` copies from a random offset, so
/// payloads differ between sessions without per byte random number cost.
pub struct Filler {
    buf: Vec<u8>,
}

impl Filler {
    pub const SIZE: usize = 1 << 20;

    pub fn new(seed: u64) -> Self {
        const ALPHABET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .,;:-_<>/=\"'\n";
        let mut rng = SmallRng::seed_from_u64(seed);
        let buf = (0..Self::SIZE)
            .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
            .collect();
        Filler { buf }
    }

    pub fn fill(&self, out: &mut Vec<u8>, mut n: usize, rng: &mut SmallRng) {
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
pub struct RenderCtx<'a> {
    pub vars: &'a Vars,
    /// Variables bound by `set` during this session. They shadow `vars`.
    pub overlay: &'a Vars,
    pub session_id: u64,
    pub flow: &'a str,
    pub rng: &'a mut SmallRng,
    pub filler: &'a Filler,
}

impl RenderCtx<'_> {
    fn lookup(&self, name: &str) -> Result<&Value> {
        self.overlay
            .get(name)
            .or_else(|| self.vars.get(name))
            .ok_or_else(|| anyhow!("undefined variable '{name}'"))
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
    pub fn literal(bytes: impl Into<Vec<u8>>) -> Self {
        Template {
            segs: vec![Seg::Lit(bytes.into())],
        }
    }

    pub fn compile(src: &str) -> Result<Self> {
        let mut segs = Vec::new();
        let mut rest = src;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                push_lit(&mut segs, &rest.as_bytes()[..open]);
            }
            let after = &rest[open + 2..];
            let close = after
                .find("}}")
                .ok_or_else(|| anyhow!("unbalanced '{{{{' in template"))?;
            let expr = after[..close].trim();
            compile_expr(expr, &mut segs).with_context(|| format!("in expression '{expr}'"))?;
            rest = &after[close + 2..];
        }
        if rest.contains("}}") {
            bail!("unbalanced '}}}}' in template");
        }
        if !rest.is_empty() {
            push_lit(&mut segs, rest.as_bytes());
        }
        Ok(Template { segs })
    }

    /// Compile a YAML scalar that is either an integer or a template.
    pub fn from_yaml_count(v: &serde_yaml::Value) -> Result<Self> {
        match v {
            serde_yaml::Value::Number(n) => {
                let i = n
                    .as_i64()
                    .ok_or_else(|| anyhow!("expected an integer, got {n}"))?;
                Ok(Template::literal(i.to_string()))
            }
            serde_yaml::Value::String(s) => Template::compile(s),
            other => bail!("expected an integer or template, got {other:?}"),
        }
    }

    pub fn is_static(&self) -> bool {
        self.segs.iter().all(|s| matches!(s, Seg::Lit(_)))
    }

    pub fn variables(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for seg in &self.segs {
            match seg {
                Seg::Var(n) => out.push(n.as_str()),
                Seg::Fill(Arg::Var(n)) => out.push(n.as_str()),
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

    pub fn render_into(&self, ctx: &mut RenderCtx<'_>, out: &mut Vec<u8>) -> Result<()> {
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
                    if n < 0 {
                        bail!("fill() needs a non negative size, got {n}");
                    }
                    let filler = ctx.filler;
                    filler.fill(out, n as usize, ctx.rng);
                }
                Seg::RandInt(a, b) => {
                    let (lo, hi) = (arg_int(a, ctx)?, arg_int(b, ctx)?);
                    if lo > hi {
                        bail!("rand_int({lo}, {hi}) has an empty range");
                    }
                    let v = ctx.rng.gen_range(lo..=hi);
                    out.extend_from_slice(v.to_string().as_bytes());
                }
            }
        }
        Ok(())
    }

    pub fn render_to_vec(&self, ctx: &mut RenderCtx<'_>) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.render_into(ctx, &mut out)?;
        Ok(out)
    }

    pub fn render_int(&self, ctx: &mut RenderCtx<'_>) -> Result<i64> {
        let bytes = self.render_to_vec(ctx)?;
        let text = std::str::from_utf8(&bytes)?.trim();
        text.parse::<i64>()
            .map_err(|_| anyhow!("'{text}' is not an integer"))
    }
}

fn push_lit(segs: &mut Vec<Seg>, bytes: &[u8]) {
    if let Some(Seg::Lit(prev)) = segs.last_mut() {
        prev.extend_from_slice(bytes);
    } else {
        segs.push(Seg::Lit(bytes.to_vec()));
    }
}

fn arg_int(arg: &Arg, ctx: &RenderCtx<'_>) -> Result<i64> {
    match arg {
        Arg::Int(i) => Ok(*i),
        Arg::Var(name) => ctx
            .lookup(name)?
            .as_i64()
            .ok_or_else(|| anyhow!("variable '{name}' is not an integer")),
        Arg::SessionId => Ok(ctx.session_id as i64),
        Arg::Str(s) => s.parse().map_err(|_| anyhow!("'{s}' is not an integer")),
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_ref(text: &str) -> Result<Seg> {
    let parts: Vec<&str> = text.split('.').collect();
    if !parts.iter().all(|p| is_ident(p)) {
        bail!("cannot parse expression '{text}'");
    }
    match parts.as_slice() {
        ["vars", name] => Ok(Seg::Var(name.to_string())),
        ["session", "id"] => Ok(Seg::SessionId),
        ["session", "flow"] => Ok(Seg::SessionFlow),
        ["session", other] => bail!("unknown session field '{other}'"),
        [root, ..] => bail!("unknown reference '{root}'"),
        [] => bail!("empty reference"),
    }
}

fn parse_arg(text: &str) -> Result<Arg> {
    let text = text.trim();
    if let Ok(i) = text.parse::<i64>() {
        return Ok(Arg::Int(i));
    }
    if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
        return Ok(Arg::Str(text[1..text.len() - 1].to_string()));
    }
    match parse_ref(text)? {
        Seg::Var(n) => Ok(Arg::Var(n)),
        Seg::SessionId => Ok(Arg::SessionId),
        _ => bail!("'{text}' cannot be a function argument"),
    }
}

fn split_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in text.chars() {
        if ch == '\'' {
            quoted = !quoted;
        }
        if ch == ',' && !quoted {
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

fn compile_expr(expr: &str, segs: &mut Vec<Seg>) -> Result<()> {
    if expr.is_empty() {
        bail!("empty placeholder");
    }
    if let Ok(i) = expr.parse::<i64>() {
        push_lit(segs, i.to_string().as_bytes());
        return Ok(());
    }
    if expr.len() >= 2 && expr.starts_with('\'') && expr.ends_with('\'') {
        push_lit(segs, &expr.as_bytes()[1..expr.len() - 1]);
        return Ok(());
    }
    if let Some(open) = expr.find('(') {
        if !expr.ends_with(')') {
            bail!("cannot parse expression '{expr}'");
        }
        let name = expr[..open].trim();
        let args = split_args(&expr[open + 1..expr.len() - 1]);
        match (name, args.len()) {
            ("fill", 1) => segs.push(Seg::Fill(parse_arg(&args[0])?)),
            ("rand_int", 2) => segs.push(Seg::RandInt(parse_arg(&args[0])?, parse_arg(&args[1])?)),
            ("hex", 1) => match parse_arg(&args[0])? {
                Arg::Str(s) => push_lit(segs, &decode_hex(&s)?),
                _ => bail!("hex() takes a quoted literal in this spike"),
            },
            (n, _) if n.contains('.') => {
                bail!("module helper '{n}' needs an extension, which spike S1 does not run")
            }
            ("fill" | "hex" | "rand_int", k) => bail!("function '{name}' got {k} argument(s)"),
            (n, _) => bail!("unknown function '{n}'"),
        }
        return Ok(());
    }
    segs.push(parse_ref(expr)?);
    Ok(())
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) {
        bail!("hex string has an odd number of digits");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!("bad hex: {e}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(t: &str, vars: &Vars) -> Result<Vec<u8>> {
        let filler = Filler::new(1);
        let mut rng = SmallRng::seed_from_u64(9);
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
        assert!(Template::compile("{{ nope(1) }}").is_err());
        assert!(Template::compile("{{ fill(1, 2) }}").is_err());
        assert!(Template::compile("{{ vars.x ").is_err());
        assert!(Template::compile("x }}").is_err());
        assert!(Template::compile("{{ hex('abc') }}").is_err());
        assert!(Template::compile("{{ http1.request('GET') }}").is_err());
        assert!(render("{{ vars.missing }}", &vars()).is_err());
    }

    #[test]
    fn render_int() {
        let filler = Filler::new(1);
        let mut rng = SmallRng::seed_from_u64(1);
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
}
