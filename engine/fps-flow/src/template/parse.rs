// SPDX-License-Identifier: GPL-3.0-only
//! Parser and checker of one placeholder expression in the contract v0.2
//! grammar.
//!
//! The tokenizer and the recursive descent parser follow the reference
//! implementation in `tools/contracts/validate.py`, so the engine and the
//! validator accept the same templates.

use super::{Const, Expr, ModuleCall, TemplateError};
use crate::function::ValueType;

/// Calls nest at most this deep, counting the outermost call as 1.
pub const MAX_CALL_DEPTH: usize = 8;

/// Roots that a module may not take as its name.
const RESERVED_ROOTS: [&str; 5] = ["vars", "session", "last", "true", "false"];

/// Parameters of the core functions, in positional order.
const FILL: &[(&str, ValueType)] = &[("n", ValueType::Int)];
const HEX: &[(&str, ValueType)] = &[("s", ValueType::Text)];
const RAND_INT: &[(&str, ValueType)] = &[("a", ValueType::Int), ("b", ValueType::Int)];

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Open,
    Close,
    Dot,
    Comma,
    Equals,
    Str(String),
    Int(i64),
    Float(f64),
    Ident(String),
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Open => "(".into(),
            Tok::Close => ")".into(),
            Tok::Dot => ".".into(),
            Tok::Comma => ",".into(),
            Tok::Equals => "=".into(),
            Tok::Str(s) => format!("'{s}'"),
            Tok::Int(i) => i.to_string(),
            Tok::Float(f) => f.to_string(),
            Tok::Ident(s) => s.clone(),
        }
    }
}

/// A parsed expression before name resolution.
#[derive(Debug)]
enum Node {
    Lit(Const),
    Ref(Vec<String>),
    Call {
        module: Option<String>,
        name: String,
        positional: Vec<Node>,
        keywords: Vec<(String, Node)>,
    },
}

fn tokenize(text: &str) -> Result<Vec<Tok>, String> {
    let bytes = text.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while let Some(&c) = bytes.get(i) {
        let single = match c {
            b'(' => Some(Tok::Open),
            b')' => Some(Tok::Close),
            b'.' => Some(Tok::Dot),
            b',' => Some(Tok::Comma),
            b'=' => Some(Tok::Equals),
            _ => None,
        };
        if let Some(tok) = single {
            toks.push(tok);
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'\'' {
            let rest = text.get(i + 1..).unwrap_or_default();
            let end = rest.find('\'').ok_or("unterminated string literal")?;
            toks.push(Tok::Str(rest.get(..end).unwrap_or_default().to_string()));
            i += end + 2;
        } else if c.is_ascii_digit()
            || (c == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i;
            i += 1;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            let is_float =
                bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
            if is_float {
                i += 1;
                while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            let lit = text.get(start..i).unwrap_or_default();
            toks.push(if is_float {
                Tok::Float(lit.parse().map_err(|_| format!("bad float '{lit}'"))?)
            } else {
                Tok::Int(
                    lit.parse()
                        .map_err(|_| format!("integer '{lit}' is out of range"))?,
                )
            });
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while bytes
                .get(i)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                i += 1;
            }
            toks.push(Tok::Ident(
                text.get(start..i).unwrap_or_default().to_string(),
            ));
        } else {
            let ch = text.get(i..).and_then(|s| s.chars().next()).unwrap_or('?');
            return Err(format!("unexpected character '{ch}'"));
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

/// Parser failures before they receive the expression text.
enum Fail {
    Syntax(String),
    TooDeep,
}

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail::Syntax(s)
    }
}

impl From<&str> for Fail {
    fn from(s: &str) -> Self {
        Fail::Syntax(s.to_string())
    }
}

impl Parser {
    fn peek(&self, offset: usize) -> Option<&Tok> {
        self.toks.get(self.pos + offset)
    }

    fn bump(&mut self) -> Option<Tok> {
        let tok = self.toks.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    fn take(&mut self, want: &Tok) -> Result<(), Fail> {
        match self.bump() {
            Some(t) if &t == want => Ok(()),
            Some(t) => {
                Err(format!("expected '{}', got '{}'", want.describe(), t.describe()).into())
            }
            None => Err(format!("expected '{}', got end of expression", want.describe()).into()),
        }
    }

    fn ident(&mut self) -> Result<String, Fail> {
        match self.bump() {
            Some(Tok::Ident(s)) => Ok(s),
            Some(t) => Err(format!("expected a name, got '{}'", t.describe()).into()),
            None => Err("expected a name, got end of expression".into()),
        }
    }

    fn parse(&mut self) -> Result<Node, Fail> {
        let node = self.expression(1)?;
        match self.peek(0) {
            None => Ok(node),
            Some(t) => Err(format!("unexpected '{}'", t.describe()).into()),
        }
    }

    fn expression(&mut self, depth: usize) -> Result<Node, Fail> {
        let first = match self.bump() {
            Some(Tok::Int(i)) => return Ok(Node::Lit(Const::Int(i))),
            Some(Tok::Float(f)) => return Ok(Node::Lit(Const::Float(f))),
            Some(Tok::Str(s)) => return Ok(Node::Lit(Const::Text(s))),
            Some(Tok::Ident(s)) => s,
            _ => return Err("expected a literal, a reference, or a call".into()),
        };
        let next = self.peek(0);
        if (first == "true" || first == "false")
            && next != Some(&Tok::Dot)
            && next != Some(&Tok::Open)
        {
            return Ok(Node::Lit(Const::Bool(first == "true")));
        }
        let mut parts = vec![first];
        while self.peek(0) == Some(&Tok::Dot) {
            self.pos += 1;
            parts.push(self.ident()?);
        }
        if self.peek(0) != Some(&Tok::Open) {
            return Ok(Node::Ref(parts));
        }
        if parts.len() > 2 {
            return Err(format!(
                "'{}' is not a function name; use <module>.<function>",
                parts.join(".")
            )
            .into());
        }
        if depth > MAX_CALL_DEPTH {
            return Err(Fail::TooDeep);
        }
        self.take(&Tok::Open)?;
        let name = parts.pop().unwrap_or_default();
        let module = parts.pop();
        let mut positional = Vec::new();
        let mut keywords = Vec::new();
        if self.peek(0) != Some(&Tok::Close) {
            loop {
                if matches!(self.peek(0), Some(Tok::Ident(_))) && self.peek(1) == Some(&Tok::Equals)
                {
                    let key = self.ident()?;
                    self.pos += 1;
                    keywords.push((key, self.expression(depth + 1)?));
                } else if !keywords.is_empty() {
                    return Err("a positional argument follows a keyword argument".into());
                } else {
                    positional.push(self.expression(depth + 1)?);
                }
                if self.peek(0) != Some(&Tok::Comma) {
                    break;
                }
                self.pos += 1;
            }
        }
        self.take(&Tok::Close)?;
        Ok(Node::Call {
            module,
            name,
            positional,
            keywords,
        })
    }
}

/// Parses and checks one placeholder expression.
pub(super) fn parse_expression(text: &str) -> Result<Expr, TemplateError> {
    let syntax = |reason: String| TemplateError::Syntax {
        expr: text.to_string(),
        reason,
    };
    let toks = tokenize(text).map_err(syntax)?;
    let mut parser = Parser { toks, pos: 0 };
    let node = parser.parse().map_err(|f| match f {
        Fail::Syntax(reason) => syntax(reason),
        Fail::TooDeep => TemplateError::TooDeep {
            max: MAX_CALL_DEPTH,
        },
    })?;
    lower(node)
}

/// Resolves names, binds core function parameters, and checks static types.
fn lower(node: Node) -> Result<Expr, TemplateError> {
    match node {
        Node::Lit(c) => Ok(Expr::Const(c)),
        Node::Ref(parts) => {
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            match refs.as_slice() {
                ["last"] => Ok(Expr::Last),
                ["vars", name] => Ok(Expr::Var((*name).to_string())),
                ["session", "id"] => Ok(Expr::SessionId),
                ["session", "flow"] => Ok(Expr::SessionFlow),
                _ => Err(TemplateError::UnknownReference(parts.join("."))),
            }
        }
        Node::Call {
            module: None,
            name,
            positional,
            keywords,
        } => {
            let params = match name.as_str() {
                "fill" => FILL,
                "hex" => HEX,
                "rand_int" => RAND_INT,
                _ => return Err(TemplateError::UnknownFunction(name)),
            };
            let mut args = bind_core(&name, params, positional, keywords)?.into_iter();
            let mut next = || Box::new(args.next().unwrap_or(Expr::Const(Const::Int(0))));
            Ok(match name.as_str() {
                "fill" => Expr::Fill(next()),
                "hex" => Expr::Hex(next()),
                _ => Expr::RandInt(next(), next()),
            })
        }
        Node::Call {
            module: Some(module),
            name,
            positional,
            keywords,
        } => {
            if RESERVED_ROOTS.contains(&module.as_str()) {
                return Err(TemplateError::ReservedModule(module));
            }
            let qualified = format!("{module}.{name}");
            let positional = positional
                .into_iter()
                .map(lower)
                .collect::<Result<Vec<_>, _>>()?;
            let mut named: Vec<(String, Expr)> = Vec::with_capacity(keywords.len());
            for (key, arg) in keywords {
                if named.iter().any(|(k, _)| *k == key) {
                    return Err(TemplateError::DuplicateParameter {
                        function: qualified,
                        param: key,
                    });
                }
                named.push((key, lower(arg)?));
            }
            Ok(Expr::Call(Box::new(ModuleCall {
                module,
                function: name,
                positional,
                named,
            })))
        }
    }
}

/// Binds the arguments of a core function to its parameters in declaration
/// order and checks their static types.
fn bind_core(
    name: &str,
    params: &[(&str, ValueType)],
    positional: Vec<Node>,
    keywords: Vec<(String, Node)>,
) -> Result<Vec<Expr>, TemplateError> {
    if positional.len() > params.len() {
        return Err(TemplateError::Arity {
            name: name.to_string(),
            max: params.len(),
            got: positional.len(),
        });
    }
    let mut slots: Vec<Option<Expr>> = vec![None; params.len()];
    for (slot, node) in slots.iter_mut().zip(positional) {
        *slot = Some(lower(node)?);
    }
    for (key, node) in keywords {
        let at = params.iter().position(|(p, _)| *p == key);
        let slot =
            at.and_then(|i| slots.get_mut(i))
                .ok_or_else(|| TemplateError::UnknownParameter {
                    function: name.to_string(),
                    param: key.clone(),
                })?;
        if slot.is_some() {
            return Err(TemplateError::DuplicateParameter {
                function: name.to_string(),
                param: key,
            });
        }
        *slot = Some(lower(node)?);
    }
    let mut out = Vec::with_capacity(params.len());
    for (slot, (param, ty)) in slots.into_iter().zip(params) {
        let expr = slot.ok_or_else(|| TemplateError::MissingParameter {
            function: name.to_string(),
            param: (*param).to_string(),
        })?;
        let got = expr.value_type();
        if !ty.accepts(got) {
            return Err(TemplateError::ArgumentType {
                function: name.to_string(),
                param: (*param).to_string(),
                expected: *ty,
                got,
            });
        }
        out.push(expr);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_follow_the_reference_tokenizer() {
        assert_eq!(
            tokenize("f(-7, 2.5, 'a b', x=y.z)").unwrap(),
            vec![
                Tok::Ident("f".into()),
                Tok::Open,
                Tok::Int(-7),
                Tok::Comma,
                Tok::Float(2.5),
                Tok::Comma,
                Tok::Str("a b".into()),
                Tok::Comma,
                Tok::Ident("x".into()),
                Tok::Equals,
                Tok::Ident("y".into()),
                Tok::Dot,
                Tok::Ident("z".into()),
                Tok::Close,
            ]
        );
        assert_eq!(tokenize("1.").unwrap(), vec![Tok::Int(1), Tok::Dot]);
        assert!(tokenize("- 1").is_err());
        assert!(tokenize("'open").is_err());
        assert!(tokenize("99999999999999999999").is_err());
        assert!(tokenize("é").is_err());
    }
}
