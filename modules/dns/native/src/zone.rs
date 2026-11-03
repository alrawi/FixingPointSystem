// SPDX-License-Identifier: GPL-3.0-only
//! Zones in the master file format of RFC 1035 section 5 and the answers
//! that an authoritative server gives from them.
//!
//! The parser reads the subset that synthetic zones need. It knows the
//! directives `$ORIGIN` and `$TTL`, the owner `@`, relative and absolute
//! names, blank owners that repeat the previous owner, optional TTL and
//! class fields in either order, comments, quoted strings, and parentheses
//! that continue a record over several lines. Records carry the types A,
//! AAAA, CNAME, MX, TXT, NS, and SOA in class IN.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::wire::{Name, RData, Rcode, Record, RecordType, CLASS_IN};

/// The zone that the module ships, `modules/dns/zones/example.test.zone`.
pub const BUNDLED_ZONE: &str = include_str!("../../zones/example.test.zone");

/// Longest CNAME chain that a lookup follows, as resolvers bound it.
pub const MAX_CHAIN: usize = 8;

/// A zone file error with its line number.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("zone line {line}: {message}")]
pub struct ZoneError {
    /// One based line number of the record that failed.
    pub line: usize,
    /// Lowercase sentence without trailing punctuation.
    pub message: String,
}

/// The records of one zone.
#[derive(Clone, Debug)]
pub struct Zone {
    origin: Name,
    records: Vec<Record>,
    /// Lowercase owner key to record indexes in file order.
    index: HashMap<String, Vec<usize>>,
}

/// The sections of an answer that [`Zone::lookup`] builds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lookup {
    /// The response code.
    pub rcode: Option<Rcode>,
    /// True when the zone is authoritative for the name.
    pub authoritative: bool,
    /// Answer section.
    pub answers: Vec<Record>,
    /// Authority section.
    pub authority: Vec<Record>,
    /// Additional section.
    pub additional: Vec<Record>,
}

/// One logical record of the master file: its tokens and its first line.
struct Entry {
    line: usize,
    /// True when the first line starts with white space, so the owner is
    /// the previous one.
    blank_owner: bool,
    tokens: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(Vec<u8>),
}

impl Token {
    fn word(&self) -> Option<&str> {
        match self {
            Token::Word(w) => Some(w),
            Token::Quoted(_) => None,
        }
    }
}

fn err(line: usize, message: impl Into<String>) -> ZoneError {
    ZoneError {
        line,
        message: message.into(),
    }
}

/// Splits the file into logical records, joining parenthesized lines and
/// dropping comments.
fn entries(text: &str) -> Result<Vec<Entry>, ZoneError> {
    let mut out: Vec<Entry> = Vec::new();
    let mut pending: Option<Entry> = None;
    let mut depth = 0usize;
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let mut tokens = Vec::new();
        let mut chars = line.chars().peekable();
        let mut word = String::new();
        let flush = |word: &mut String, tokens: &mut Vec<Token>| {
            if !word.is_empty() {
                tokens.push(Token::Word(std::mem::take(word)));
            }
        };
        while let Some(c) = chars.next() {
            match c {
                ';' => break,
                '"' => {
                    flush(&mut word, &mut tokens);
                    let mut s = Vec::new();
                    let mut closed = false;
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => {
                                closed = true;
                                break;
                            }
                            '\\' => {
                                let e = chars.next().ok_or_else(|| err(n, "escape at line end"))?;
                                let mut b = [0u8; 4];
                                s.extend_from_slice(e.encode_utf8(&mut b).as_bytes());
                            }
                            c => {
                                let mut b = [0u8; 4];
                                s.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                            }
                        }
                    }
                    if !closed {
                        return Err(err(n, "unterminated quoted string"));
                    }
                    tokens.push(Token::Quoted(s));
                }
                '(' => {
                    flush(&mut word, &mut tokens);
                    depth += 1;
                }
                ')' => {
                    flush(&mut word, &mut tokens);
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| err(n, "unbalanced ')'"))?;
                }
                c if c.is_whitespace() => flush(&mut word, &mut tokens),
                c => word.push(c),
            }
        }
        flush(&mut word, &mut tokens);
        match pending.as_mut() {
            Some(open) => open.tokens.extend(tokens),
            None if tokens.is_empty() => {}
            None => {
                pending = Some(Entry {
                    line: n,
                    blank_owner: line.starts_with([' ', '\t']),
                    tokens,
                })
            }
        }
        if depth == 0 {
            out.extend(pending.take());
        }
    }
    if let Some(open) = pending {
        return Err(err(open.line, "unbalanced '(' at end of file"));
    }
    Ok(out)
}

fn is_ttl(w: &str) -> bool {
    !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit())
}

fn parse_u32(line: usize, w: Option<&Token>, what: &str) -> Result<u32, ZoneError> {
    w.and_then(Token::word)
        .and_then(|w| w.parse().ok())
        .ok_or_else(|| err(line, format!("{what} must be an unsigned 32 bit integer")))
}

impl Zone {
    /// Parses a master file. Relative names before any `$ORIGIN` are
    /// relative to `origin`.
    pub fn parse(text: &str, origin: &Name) -> Result<Self, ZoneError> {
        let mut origin = origin.clone();
        let mut zone_origin: Option<Name> = None;
        let mut default_ttl: Option<u32> = None;
        let mut last_owner: Option<Name> = None;
        let mut last_ttl: Option<u32> = None;
        let mut records = Vec::new();
        for e in entries(text)? {
            let line = e.line;
            let name_of = |w: &str, origin: &Name| -> Result<Name, ZoneError> {
                if w == "@" {
                    return Ok(origin.clone());
                }
                let n = Name::parse(w).map_err(|x| err(line, x.to_string()))?;
                if w.ends_with('.') {
                    Ok(n)
                } else {
                    n.join(origin)
                        .ok_or_else(|| err(line, format!("name '{w}' is too long")))
                }
            };
            let mut t = e.tokens.iter().peekable();
            let first = t.peek().and_then(|x| x.word()).unwrap_or_default();
            if first.eq_ignore_ascii_case("$ORIGIN") {
                t.next();
                let w = t
                    .next()
                    .and_then(Token::word)
                    .ok_or_else(|| err(line, "$ORIGIN needs a name"))?;
                if !w.ends_with('.') {
                    return Err(err(line, "$ORIGIN needs an absolute name"));
                }
                origin = name_of(w, &origin)?;
                continue;
            }
            if first.eq_ignore_ascii_case("$TTL") {
                t.next();
                default_ttl = Some(parse_u32(line, t.next(), "$TTL")?);
                continue;
            }
            if first.starts_with('$') {
                return Err(err(line, format!("directive {first} is not supported")));
            }
            let owner = if e.blank_owner {
                last_owner
                    .clone()
                    .ok_or_else(|| err(line, "the first record needs an owner"))?
            } else {
                let w = t
                    .next()
                    .and_then(Token::word)
                    .ok_or_else(|| err(line, "the owner must not be quoted"))?;
                name_of(w, &origin)?
            };
            let mut ttl = None;
            let mut rtype = None;
            while let Some(w) = t.next().and_then(Token::word) {
                if is_ttl(w) && ttl.is_none() {
                    ttl = Some(parse_u32(line, Some(&Token::Word(w.into())), "the TTL")?);
                } else if w.eq_ignore_ascii_case("IN") {
                } else if ["CH", "HS", "CS"].iter().any(|c| w.eq_ignore_ascii_case(c)) {
                    return Err(err(line, format!("class {w} is not supported")));
                } else {
                    rtype = Some(
                        RecordType::parse(w)
                            .ok_or_else(|| err(line, format!("type {w} is not supported")))?,
                    );
                    break;
                }
            }
            let rtype = rtype.ok_or_else(|| err(line, "the record has no type"))?;
            let rest: Vec<&Token> = t.collect();
            let word = |i: usize, what: &str| -> Result<&str, ZoneError> {
                rest.get(i)
                    .and_then(|x| x.word())
                    .ok_or_else(|| err(line, format!("{rtype} needs {what}")))
            };
            let arity = |n: usize| -> Result<(), ZoneError> {
                if rest.len() == n {
                    Ok(())
                } else {
                    Err(err(
                        line,
                        format!("{rtype} takes {n} data fields, got {}", rest.len()),
                    ))
                }
            };
            let data = match rtype {
                RecordType::A => {
                    arity(1)?;
                    let a: Ipv4Addr = word(0, "an IPv4 address")?
                        .parse()
                        .map_err(|_| err(line, "A needs an IPv4 address"))?;
                    RData::A(a)
                }
                RecordType::Aaaa => {
                    arity(1)?;
                    let a: Ipv6Addr = word(0, "an IPv6 address")?
                        .parse()
                        .map_err(|_| err(line, "AAAA needs an IPv6 address"))?;
                    RData::Aaaa(a)
                }
                RecordType::Cname => {
                    arity(1)?;
                    RData::Cname(name_of(word(0, "a target name")?, &origin)?)
                }
                RecordType::Ns => {
                    arity(1)?;
                    RData::Ns(name_of(word(0, "a server name")?, &origin)?)
                }
                RecordType::Mx => {
                    arity(2)?;
                    let preference = word(0, "a preference")?
                        .parse()
                        .map_err(|_| err(line, "MX needs a 16 bit preference"))?;
                    let exchange = name_of(word(1, "an exchange name")?, &origin)?;
                    RData::Mx {
                        preference,
                        exchange,
                    }
                }
                RecordType::Txt => {
                    if rest.is_empty() {
                        return Err(err(line, "TXT needs at least one string"));
                    }
                    let mut strings = Vec::new();
                    for x in &rest {
                        let s = match x {
                            Token::Quoted(q) => q.clone(),
                            Token::Word(w) => w.as_bytes().to_vec(),
                        };
                        if s.len() > 255 {
                            return Err(err(line, "a TXT string is longer than 255 bytes"));
                        }
                        strings.push(s);
                    }
                    RData::Txt(strings)
                }
                RecordType::Soa => {
                    arity(7)?;
                    let n = |i| parse_u32(line, rest.get(i).copied(), "an SOA timer");
                    RData::Soa {
                        mname: name_of(word(0, "a primary server")?, &origin)?,
                        rname: name_of(word(1, "a mailbox")?, &origin)?,
                        serial: n(2)?,
                        refresh: n(3)?,
                        retry: n(4)?,
                        expire: n(5)?,
                        minimum: n(6)?,
                    }
                }
            };
            if rtype == RecordType::Soa {
                if zone_origin.is_some() {
                    return Err(err(line, "the zone has a second SOA record"));
                }
                zone_origin = Some(owner.clone());
            }
            let ttl = ttl
                .or(default_ttl)
                .or(last_ttl)
                .ok_or_else(|| err(line, "the record has no TTL and no $TTL is set"))?;
            last_owner = Some(owner.clone());
            last_ttl = Some(ttl);
            records.push(Record {
                name: owner,
                class: CLASS_IN,
                ttl,
                data,
            });
        }
        let origin = zone_origin.unwrap_or(origin);
        let mut index: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in records.iter().enumerate() {
            index.entry(r.name.key()).or_default().push(i);
        }
        let zone = Zone {
            origin,
            records,
            index,
        };
        zone.check_cnames()?;
        Ok(zone)
    }

    /// Parses the zone that the module ships.
    pub fn bundled() -> Result<Self, ZoneError> {
        Zone::parse(BUNDLED_ZONE, &Name::root())
    }

    /// RFC 1034 section 3.6.2 forbids other data next to a CNAME.
    fn check_cnames(&self) -> Result<(), ZoneError> {
        for idx in self.index.values() {
            let cnames = idx
                .iter()
                .filter(|&&i| matches!(self.records[i].data, RData::Cname(_)))
                .count();
            if cnames > 0 && idx.len() > 1 {
                let name = &self.records[idx[0]].name;
                return Err(err(0, format!("{name} has a CNAME and other data")));
            }
        }
        Ok(())
    }

    /// Returns the origin, the owner of the SOA record when the zone has one.
    pub fn origin(&self) -> &Name {
        &self.origin
    }

    /// Returns every record in file order.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    fn at(&self, name: &Name) -> impl Iterator<Item = &Record> {
        self.index
            .get(&name.key())
            .into_iter()
            .flatten()
            .filter_map(|&i| self.records.get(i))
    }

    fn soa(&self) -> Vec<Record> {
        self.at(&self.origin)
            .filter(|r| matches!(r.data, RData::Soa { .. }))
            .cloned()
            .collect()
    }

    /// Answers a question of type code `qtype` for `qname` as an
    /// authoritative server of the zone.
    ///
    /// The answer follows CNAME chains inside the zone up to [`MAX_CHAIN`]
    /// links. A missing name gives NXDOMAIN and a missing type gives an
    /// empty NOERROR answer, both with the SOA record in the authority
    /// section for negative caching (RFC 2308). A name outside the zone is
    /// refused. MX and NS answers carry the addresses of their targets in
    /// the additional section when the zone holds them.
    pub fn lookup(&self, qname: &Name, qtype: u16) -> Lookup {
        if !qname.is_within(&self.origin) {
            return Lookup {
                rcode: Some(Rcode::Refused),
                ..Lookup::default()
            };
        }
        let mut out = Lookup {
            rcode: Some(Rcode::NoError),
            authoritative: true,
            ..Lookup::default()
        };
        let mut name = qname.clone();
        for _ in 0..=MAX_CHAIN {
            let here: Vec<&Record> = self.at(&name).collect();
            if here.is_empty() {
                if out.answers.is_empty() || name.is_within(&self.origin) {
                    out.rcode = Some(Rcode::NxDomain);
                    out.authority = self.soa();
                }
                return out;
            }
            let matching: Vec<Record> = here
                .iter()
                .filter(|r| r.data.type_code() == qtype)
                .map(|r| with_owner(r, &name))
                .collect();
            if !matching.is_empty() {
                out.answers.extend(matching);
                out.additional = self.glue(&out.answers);
                return out;
            }
            let cname = here.iter().find_map(|r| match &r.data {
                RData::Cname(t) => Some((with_owner(r, &name), t.clone())),
                _ => None,
            });
            match cname {
                Some((rec, target)) if qtype != RecordType::Cname.code() => {
                    out.answers.push(rec);
                    if !target.is_within(&self.origin) {
                        // The resolver continues the chain elsewhere.
                        return out;
                    }
                    name = target;
                }
                _ => {
                    out.authority = self.soa();
                    return out;
                }
            }
        }
        // The chain is longer than resolvers follow.
        out.rcode = Some(Rcode::ServFail);
        out.answers.clear();
        out
    }

    /// Returns the A and AAAA records of the MX and NS targets in `answers`.
    fn glue(&self, answers: &[Record]) -> Vec<Record> {
        let mut out = Vec::new();
        for r in answers {
            let target = match &r.data {
                RData::Mx { exchange, .. } => exchange,
                RData::Ns(n) => n,
                _ => continue,
            };
            for g in self.at(target) {
                if matches!(g.data, RData::A(_) | RData::Aaaa(_)) && !out.contains(g) {
                    out.push(g.clone());
                }
            }
        }
        out
    }
}

/// Returns a copy of `r` whose owner keeps the case of `name`, so the
/// answer compresses against the question.
fn with_owner(r: &Record, name: &Name) -> Record {
    Record {
        name: name.clone(),
        ..r.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Name {
        Name::parse(s).unwrap()
    }

    fn zone(text: &str) -> Result<Zone, ZoneError> {
        Zone::parse(text, &n("example.test"))
    }

    const SMALL: &str = r#"
$TTL 300
@        IN SOA ns1 hostmaster (
                2026110301 ; serial
                7200 3600 1209600 60 )
         IN NS  ns1
ns1      IN A   192.0.2.53
www  600 IN A   192.0.2.1
         IN AAAA 2001:db8::1
alias    CNAME  www
outside  CNAME  www.other.test.
loop1    CNAME  loop2
loop2    CNAME  loop1
@        MX 10  mail
mail     A      192.0.2.25
@        TXT    "v=spf1 -all" "second; string"
"#;

    #[test]
    fn parses_directives_owners_ttls_and_parentheses() {
        let z = zone(SMALL).unwrap();
        assert_eq!(z.origin(), &n("example.test"));
        let r = z.records();
        assert_eq!(r.len(), 12);
        assert_eq!(r[0].ttl, 300);
        assert!(matches!(
            r[0].data,
            RData::Soa {
                serial: 2026110301,
                minimum: 60,
                ..
            }
        ));
        assert_eq!(r[1].name, n("example.test"));
        assert_eq!(r[3].ttl, 600);
        assert_eq!(r[4].name, n("www.example.test"));
        assert_eq!(r[4].ttl, 300);
        assert_eq!(
            r[11].data,
            RData::Txt(vec![b"v=spf1 -all".to_vec(), b"second; string".to_vec()])
        );
    }

    #[test]
    fn answers_a_aaaa_mx_and_txt_with_glue() {
        let z = zone(SMALL).unwrap();
        let a = z.lookup(&n("WWW.example.test"), 1);
        assert_eq!(a.rcode, Some(Rcode::NoError));
        assert!(a.authoritative);
        assert_eq!(a.answers.len(), 1);
        assert_eq!(a.answers[0].name.labels()[0], b"WWW");
        assert_eq!(z.lookup(&n("www.example.test"), 28).answers.len(), 1);
        let mx = z.lookup(&n("example.test"), 15);
        assert_eq!(mx.answers.len(), 1);
        assert_eq!(mx.additional.len(), 1);
        assert_eq!(mx.additional[0].name, n("mail.example.test"));
        assert_eq!(z.lookup(&n("example.test"), 16).answers.len(), 1);
    }

    #[test]
    fn follows_cname_chains_and_stops_at_loops_and_the_zone_edge() {
        let z = zone(SMALL).unwrap();
        let a = z.lookup(&n("alias.example.test"), 1);
        assert_eq!(a.rcode, Some(Rcode::NoError));
        assert_eq!(a.answers.len(), 2);
        assert!(matches!(a.answers[0].data, RData::Cname(_)));
        assert!(matches!(a.answers[1].data, RData::A(_)));
        let c = z.lookup(&n("alias.example.test"), 5);
        assert_eq!(c.answers.len(), 1);
        let out = z.lookup(&n("outside.example.test"), 1);
        assert_eq!((out.rcode, out.answers.len()), (Some(Rcode::NoError), 1));
        let l = z.lookup(&n("loop1.example.test"), 1);
        assert_eq!(l.rcode, Some(Rcode::ServFail));
    }

    #[test]
    fn negative_answers_carry_the_soa() {
        let z = zone(SMALL).unwrap();
        let nx = z.lookup(&n("missing.example.test"), 1);
        assert_eq!(nx.rcode, Some(Rcode::NxDomain));
        assert!(nx.answers.is_empty());
        assert!(matches!(nx.authority[0].data, RData::Soa { .. }));
        let nodata = z.lookup(&n("mail.example.test"), 28);
        assert_eq!(nodata.rcode, Some(Rcode::NoError));
        assert!(nodata.answers.is_empty());
        assert_eq!(nodata.authority.len(), 1);
        let refused = z.lookup(&n("www.other.test"), 1);
        assert_eq!(refused.rcode, Some(Rcode::Refused));
        assert!(!refused.authoritative);
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        for (text, line, part) in [
            ("www A 192.0.2.1\n", 1, "no TTL"),
            ("$TTL 1\nwww A 300.0.0.1\n", 2, "IPv4"),
            ("$TTL 1\nwww SRV 1 2 3 x\n", 2, "SRV"),
            ("$TTL 1\nwww CH A 192.0.2.1\n", 2, "class CH"),
            ("$TTL 1\n  A 192.0.2.1\n", 2, "owner"),
            ("$TTL 1\nwww TXT \"open\n", 2, "unterminated"),
            ("$TTL 1\nwww A ( 192.0.2.1\n", 2, "unbalanced"),
            ("$TTL 1\nwww MX 10\n", 2, "2 data fields"),
            ("$INCLUDE other\n", 1, "not supported"),
            ("$ORIGIN relative\n", 1, "absolute"),
            ("$TTL 1\na CNAME b\na A 192.0.2.1\n", 0, "CNAME and other"),
        ] {
            let e = zone(text).unwrap_err();
            assert_eq!(e.line, line, "{text:?}: {e}");
            assert!(e.message.contains(part), "{text:?}: {e}");
        }
    }

    #[test]
    fn the_bundled_zone_parses() {
        let z = Zone::bundled().unwrap();
        assert_eq!(z.origin(), &n("example.test"));
        for t in [
            RecordType::A,
            RecordType::Aaaa,
            RecordType::Cname,
            RecordType::Mx,
            RecordType::Txt,
        ] {
            assert!(
                z.records().iter().any(|r| r.data.type_code() == t.code()),
                "{t}"
            );
        }
    }
}
