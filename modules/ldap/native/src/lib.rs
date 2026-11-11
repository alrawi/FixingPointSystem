// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `ldap` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `ldap` functions behind the [`TemplateFn`]
//! trait of `fps-flow`. It holds a BER codec (the [`ber`] module), the LDAPv3
//! bind, search, and unbind messages of RFC 4511 with the filters of RFC 4515,
//! and a server side that answers binds and searches from a directory
//! fixture, which is LDIF text that a scenario carries in a variable. An empty
//! variable selects the directory that ships with the module.
//!
//! The scenario language has no hook for received bytes, so a scenario reads
//! each message with three actions. The first `expect` reads the 4 octet head
//! of the message, which holds the tag and the length octets. `ldap.size`
//! works out from it how many octets follow, and a second `expect` reads them.
//! `ldap.recv` then decodes the message, requires the kind that the scenario
//! expects and, for a response, the result code, and keeps the message, so
//! that `ldap.field` and the server side functions read it.
//!
//! ```yaml
//! - expect: { bytes: 4, timeout_ms: 5000 }
//! - expect: { bytes: "{{ ldap.size(last) }}", timeout_ms: 5000 }
//! - set: { id: "{{ ldap.recv(last, 'bind_response') }}" }
//! ```
//!
//! The server answers from the directory. `ldap.bind_answer(directory)` checks
//! the bind request that `recv` kept against the `userPassword` of the entry
//! that it names, and `ldap.matches(directory)` runs the search request that
//! `recv` kept and returns the number of entries. The server then sends them
//! one by one with `ldap.search_next()` and ends with `ldap.search_end()`. The
//! client counts the entries it must read with `ldap.count`, which runs the
//! same search on its own copy of the directory.
//!
//! [`FUNCTIONS`] is the catalog that `modules/ldap/manifest.yaml` lists under
//! `functions`. The catalog test compares the two.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod ber;
pub mod catalog;
pub mod directory;
pub mod filter;
pub mod message;

use std::collections::VecDeque;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS, MAX_MESSAGE};

use crate::args::Args;
use crate::ber::Malformed;
use crate::directory::{first_value, parse_attributes, Directory};
use crate::filter::{escape, Filter};
use crate::message::{Auth, Entry, LdapResult, Message, Op, SearchRequest, MAX_MESSAGE_ID};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "ldap";

/// The kinds of message that `recv` accepts.
const KINDS: [&str; 6] = [
    "bind_request",
    "bind_response",
    "unbind_request",
    "search_request",
    "search_entry",
    "search_done",
];

/// The search that `matches` ran, which `search_next` and `search_end` answer.
#[derive(Clone, Debug)]
struct Pending {
    id: i64,
    entries: VecDeque<Entry>,
    result: LdapResult,
}

/// The template functions of the `ldap` module for one session side.
///
/// The value keeps the head of the message that `size` read, the message that
/// `recv` decoded, and the search that `matches` ran. Use one value per
/// session and side.
#[derive(Clone, Debug, Default)]
pub struct Ldap {
    head: Option<Vec<u8>>,
    current: Option<Message>,
    pending: Option<Pending>,
}

fn invalid(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

/// A malformed value that the scenario gave to a function.
fn arg_err(function: &str, e: Malformed) -> FnError {
    invalid(format!("'{function}': {e}"))
}

/// A malformed message from the peer.
fn wire_err(e: Malformed) -> FnError {
    protocol(e.to_string())
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

fn push_num(out: &mut Vec<u8>, n: i64) {
    push(out, &n.to_string());
}

/// The directory of the module, `modules/ldap/directory.ldif`: 16 people with
/// a password, the groups that they belong to, and a root DSE. A function that
/// gets an empty directory argument answers from it.
pub const DEFAULT_DIRECTORY: &str = include_str!("../../directory.ldif");

/// Parses the directory argument of a function. An empty text, which is what
/// the `directory` variable of every scenario holds by default, selects
/// [`DEFAULT_DIRECTORY`].
fn directory_of(a: &Args<'_>, function: &str) -> Result<Directory, FnError> {
    let text = a.text("directory")?;
    let text = if text.trim().is_empty() {
        DEFAULT_DIRECTORY
    } else {
        text
    };
    Directory::parse(text).map_err(|e| arg_err(function, e))
}

fn scope_of(function: &str, text: &str) -> Result<i64, FnError> {
    match text {
        "base" => Ok(0),
        "one" => Ok(1),
        "sub" => Ok(2),
        other => Err(invalid(format!(
            "'scope' of '{function}' must be base, one, or sub, not '{other}'"
        ))),
    }
}

fn scope_name(scope: i64) -> &'static str {
    ["base", "one", "sub"]
        .get(scope as usize)
        .copied()
        .unwrap_or("sub")
}

fn deref_of(function: &str, text: &str) -> Result<i64, FnError> {
    match text {
        "never" => Ok(0),
        "searching" => Ok(1),
        "finding" => Ok(2),
        "always" => Ok(3),
        other => Err(invalid(format!(
            "'deref' of '{function}' must be never, searching, finding, or always, not '{other}'"
        ))),
    }
}

fn deref_name(deref: i64) -> &'static str {
    ["never", "searching", "finding", "always"]
        .get(deref as usize)
        .copied()
        .unwrap_or("never")
}

fn message_id(function: &str, a: &Args<'_>) -> Result<i64, FnError> {
    let id = a.int("message_id")?;
    if (0..=MAX_MESSAGE_ID).contains(&id) {
        Ok(id)
    } else {
        Err(invalid(format!(
            "'message_id' of '{function}' must lie between 0 and {MAX_MESSAGE_ID}, not {id}"
        )))
    }
}

fn non_negative(function: &str, name: &str, v: i64, max: i64) -> Result<i64, FnError> {
    if (0..=max).contains(&v) {
        Ok(v)
    } else {
        Err(invalid(format!(
            "'{name}' of '{function}' must lie between 0 and {max}, not {v}"
        )))
    }
}

fn encode(function: &str, m: &Message, out: &mut Vec<u8>) -> Result<(), FnError> {
    out.extend_from_slice(&m.encode().map_err(|e| arg_err(function, e))?);
    Ok(())
}

fn result_of(function: &str, a: &Args<'_>) -> Result<LdapResult, FnError> {
    Ok(LdapResult {
        code: non_negative(function, "result_code", a.int("result_code")?, 1000)?,
        matched_dn: a.text("matched_dn")?.to_owned(),
        diagnostic: a.text("diagnostic")?.to_owned(),
    })
}

/// The number of octets that follow the 4 octet head of a message.
fn rest_of_head(head: &[u8]) -> Result<usize, FnError> {
    let [tag, len0, b2, b3] = *head else {
        return Err(invalid(format!(
            "'head' of 'size' must be 4 octets, not {}",
            head.len()
        )));
    };
    if tag != ber::SEQUENCE {
        return Err(protocol(format!(
            "an LDAPMessage starts with 0x30, not 0x{tag:02x}"
        )));
    }
    // The header has 2, 3, or 4 octets, and the rest of the 4 octet head
    // already belongs to the content.
    let (header, content) = match len0 {
        0x00..=0x7f => (2, usize::from(len0)),
        0x81 => (3, usize::from(b2)),
        0x82 => (4, usize::from(u16::from_be_bytes([b2, b3]))),
        other => {
            return Err(protocol(format!(
                "the length form 0x{other:02x} is not supported, messages have at most 65535 content octets"
            )))
        }
    };
    // The shortest message, an unbind, has 5 content octets.
    if content < 5 {
        return Err(protocol(format!(
            "an LDAPMessage with {content} content octets is too short"
        )));
    }
    Ok(content - (4 - header))
}

impl Ldap {
    fn current(&self, what: &str) -> Result<&Message, FnError> {
        self.current
            .as_ref()
            .ok_or_else(|| protocol(format!("{what} needs a message, and no recv call kept one")))
    }

    fn recv(&mut self, message: &[u8], kind: &str, result: i64) -> Result<i64, FnError> {
        if !KINDS.contains(&kind) {
            return Err(invalid(format!(
                "'kind' of 'recv' must be one of {}, not '{kind}'",
                KINDS.join(", ")
            )));
        }
        let head = self
            .head
            .take()
            .ok_or_else(|| protocol("recv was called without a size call before it"))?;
        let mut full = head;
        full.extend_from_slice(message);
        let m = Message::decode(&full).map_err(wire_err)?;
        if m.op.name() != kind {
            return Err(protocol(format!("expected {kind}, got {}", m.op.name())));
        }
        if let Op::BindResponse(r) | Op::SearchDone(r) = &m.op {
            if r.code != result {
                return Err(protocol(format!(
                    "the {kind} has the result code {} ({}), the scenario expects {result}",
                    r.code, r.diagnostic
                )));
            }
        }
        let id = m.id;
        self.current = Some(m);
        Ok(id)
    }

    fn field(&self, name: &str, out: &mut Vec<u8>) -> Result<(), FnError> {
        let m = self.current("field")?;
        let unknown = || protocol(format!("the {} has no field '{name}'", m.op.name()));
        match (name, &m.op) {
            ("message_id", _) => push_num(out, m.id),
            ("operation", op) => push(out, op.name()),
            ("version", Op::BindRequest { version, .. }) => push_num(out, *version),
            ("name", Op::BindRequest { name, .. }) => push(out, name),
            ("auth", Op::BindRequest { auth, .. }) => push(
                out,
                match auth {
                    Auth::Simple(_) => "simple",
                    Auth::Sasl(_) => "sasl",
                },
            ),
            (
                "password",
                Op::BindRequest {
                    auth: Auth::Simple(p),
                    ..
                },
            ) => out.extend_from_slice(p),
            ("result_code", Op::BindResponse(r) | Op::SearchDone(r)) => push_num(out, r.code),
            ("matched_dn", Op::BindResponse(r) | Op::SearchDone(r)) => push(out, &r.matched_dn),
            ("diagnostic", Op::BindResponse(r) | Op::SearchDone(r)) => push(out, &r.diagnostic),
            ("base", Op::SearchRequest(s)) => push(out, &s.base),
            ("scope", Op::SearchRequest(s)) => push(out, scope_name(s.scope)),
            ("deref", Op::SearchRequest(s)) => push(out, deref_name(s.deref)),
            ("size_limit", Op::SearchRequest(s)) => push_num(out, s.size_limit),
            ("time_limit", Op::SearchRequest(s)) => push_num(out, s.time_limit),
            ("types_only", Op::SearchRequest(s)) => {
                push(out, if s.types_only { "true" } else { "false" })
            }
            ("filter", Op::SearchRequest(s)) => push(out, &s.filter.to_string()),
            ("attributes", Op::SearchRequest(s)) => push(out, &s.attributes.join(",")),
            ("dn", Op::SearchEntry(e)) => push(out, &e.dn),
            ("attributes", Op::SearchEntry(e)) => {
                let lines: Vec<String> = e
                    .attrs
                    .iter()
                    .flat_map(|(n, vals)| {
                        vals.iter()
                            .map(move |v| format!("{n}: {}", String::from_utf8_lossy(v)))
                    })
                    .collect();
                push(out, &lines.join("\n"));
            }
            _ => return Err(unknown()),
        }
        Ok(())
    }

    fn matches(&mut self, d: &Directory, out: &mut Vec<u8>) -> Result<(), FnError> {
        let m = self.current("matches")?;
        let Op::SearchRequest(req) = &m.op else {
            return Err(protocol(format!(
                "matches needs a search request, and the kept message is a {}",
                m.op.name()
            )));
        };
        let outcome = d.search(req);
        push_num(out, outcome.entries.len() as i64);
        self.pending = Some(Pending {
            id: m.id,
            entries: outcome.entries.into(),
            result: outcome.result,
        });
        Ok(())
    }
}

impl TemplateFn for Ldap {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not ldap", call.module),
            ));
        }
        use catalog as c;
        match call.function {
            "size" => {
                let a = args::bind(&c::SIZE, &call.args)?;
                let head = a.bytes("head")?;
                let rest = rest_of_head(head)?;
                self.head = Some(head.to_vec());
                push_num(out, rest as i64);
            }
            "recv" => {
                let a = args::bind(&c::RECV, &call.args)?;
                let id = self.recv(a.bytes("message")?, a.text("kind")?, a.int("result")?)?;
                push_num(out, id);
            }
            "field" => {
                let a = args::bind(&c::FIELD, &call.args)?;
                self.field(a.text("name")?, out)?;
            }
            "bind_request" => {
                let a = args::bind(&c::BIND_REQUEST, &call.args)?;
                let m = Message {
                    id: message_id("bind_request", &a)?,
                    op: Op::BindRequest {
                        version: non_negative("bind_request", "version", a.int("version")?, 127)?,
                        name: a.text("name")?.to_owned(),
                        auth: Auth::Simple(a.text("password")?.as_bytes().to_vec()),
                    },
                };
                encode("bind_request", &m, out)?;
            }
            "bind_response" => {
                let a = args::bind(&c::BIND_RESPONSE, &call.args)?;
                let m = Message {
                    id: message_id("bind_response", &a)?,
                    op: Op::BindResponse(result_of("bind_response", &a)?),
                };
                encode("bind_response", &m, out)?;
            }
            "search_request" => {
                let a = args::bind(&c::SEARCH_REQUEST, &call.args)?;
                let f = "search_request";
                let filter = Filter::parse(a.text("filter")?).map_err(|e| arg_err(f, e))?;
                let attributes = a
                    .text("attributes")?
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                let m = Message {
                    id: message_id(f, &a)?,
                    op: Op::SearchRequest(SearchRequest {
                        base: a.text("base")?.to_owned(),
                        scope: scope_of(f, a.text("scope")?)?,
                        deref: deref_of(f, a.text("deref")?)?,
                        size_limit: non_negative(
                            f,
                            "size_limit",
                            a.int("size_limit")?,
                            i64::from(i32::MAX),
                        )?,
                        time_limit: non_negative(
                            f,
                            "time_limit",
                            a.int("time_limit")?,
                            i64::from(i32::MAX),
                        )?,
                        types_only: a.boolean("types_only")?,
                        filter,
                        attributes,
                    }),
                };
                encode(f, &m, out)?;
            }
            "search_entry" => {
                let a = args::bind(&c::SEARCH_ENTRY, &call.args)?;
                let attrs = parse_attributes(a.text("attributes")?)
                    .map_err(|e| arg_err("search_entry", e))?;
                let m = Message {
                    id: message_id("search_entry", &a)?,
                    op: Op::SearchEntry(Entry {
                        dn: a.text("dn")?.to_owned(),
                        attrs,
                    }),
                };
                encode("search_entry", &m, out)?;
            }
            "search_done" => {
                let a = args::bind(&c::SEARCH_DONE, &call.args)?;
                let m = Message {
                    id: message_id("search_done", &a)?,
                    op: Op::SearchDone(result_of("search_done", &a)?),
                };
                encode("search_done", &m, out)?;
            }
            "unbind_request" => {
                let a = args::bind(&c::UNBIND_REQUEST, &call.args)?;
                let m = Message {
                    id: message_id("unbind_request", &a)?,
                    op: Op::UnbindRequest,
                };
                encode("unbind_request", &m, out)?;
            }
            "filter_eq" => {
                let a = args::bind(&c::FILTER_EQ, &call.args)?;
                let text = format!(
                    "({}={})",
                    a.text("attribute")?,
                    escape(a.text("value")?.as_bytes())
                );
                // Parsing proves that the attribute name is one.
                Filter::parse(&text).map_err(|e| arg_err("filter_eq", e))?;
                push(out, &text);
            }
            "account" => {
                let a = args::bind(&c::ACCOUNT, &call.args)?;
                let d = directory_of(&a, "account")?;
                let seed = a.int("seed")?;
                let seed = u64::try_from(seed).ok();
                let entry = d
                    .account(seed, |n| (rng.next_u64() % n as u64) as usize)
                    .ok_or_else(|| {
                        invalid("the directory of 'account' has no entry with a userPassword")
                    })?;
                let field = a.text("field")?;
                let v = first_value(entry, field).ok_or_else(|| {
                    invalid(format!(
                        "the account '{}' has no attribute '{field}'",
                        entry.dn
                    ))
                })?;
                out.extend_from_slice(&v);
            }
            "count" => {
                let a = args::bind(&c::COUNT, &call.args)?;
                let d = directory_of(&a, "count")?;
                let f = "count";
                let req = SearchRequest {
                    base: a.text("base")?.to_owned(),
                    scope: scope_of(f, a.text("scope")?)?,
                    deref: 0,
                    size_limit: non_negative(
                        f,
                        "size_limit",
                        a.int("size_limit")?,
                        i64::from(i32::MAX),
                    )?,
                    time_limit: 0,
                    types_only: false,
                    filter: Filter::parse(a.text("filter")?).map_err(|e| arg_err(f, e))?,
                    attributes: Vec::new(),
                };
                push_num(out, d.search(&req).entries.len() as i64);
            }
            "bind_answer" => {
                let a = args::bind(&c::BIND_ANSWER, &call.args)?;
                let d = directory_of(&a, "bind_answer")?;
                let m = self.current("bind_answer")?;
                let Op::BindRequest {
                    version,
                    name,
                    auth,
                } = &m.op
                else {
                    return Err(protocol(format!(
                        "bind_answer needs a bind request, and the kept message is a {}",
                        m.op.name()
                    )));
                };
                let reply = Message {
                    id: m.id,
                    op: Op::BindResponse(d.bind(*version, name, auth)),
                };
                encode("bind_answer", &reply, out)?;
            }
            "matches" => {
                let a = args::bind(&c::MATCHES, &call.args)?;
                let d = directory_of(&a, "matches")?;
                self.matches(&d, out)?;
            }
            "search_next" => {
                args::bind(&c::SEARCH_NEXT, &call.args)?;
                let p = self
                    .pending
                    .as_mut()
                    .ok_or_else(|| protocol("search_next was called before matches"))?;
                let entry = p
                    .entries
                    .pop_front()
                    .ok_or_else(|| protocol("search_next was called after the last entry"))?;
                let m = Message {
                    id: p.id,
                    op: Op::SearchEntry(entry),
                };
                encode("search_next", &m, out)?;
            }
            "search_end" => {
                args::bind(&c::SEARCH_END, &call.args)?;
                let unsent = self
                    .pending
                    .as_ref()
                    .ok_or_else(|| protocol("search_end was called before matches"))?
                    .entries
                    .len();
                if unsent > 0 {
                    return Err(protocol(format!(
                        "search_end was called with {unsent} entries not sent"
                    )));
                }
                let Some(p) = self.pending.take() else {
                    return Err(protocol("search_end was called before matches"));
                };
                let m = Message {
                    id: p.id,
                    op: Op::SearchDone(p.result),
                };
                encode("search_end", &m, out)?;
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("module 'ldap' has no function '{other}'"),
                ))
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
