// SPDX-License-Identifier: GPL-3.0-only
//! Search filters: the string form of RFC 4515, the BER form of RFC 4511, and
//! the evaluation against an entry.

use std::fmt::Write as _;

use crate::ber::{self, bad, Malformed, Tlv};

/// A search filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    /// `(&(a)(b))`.
    And(Vec<Filter>),
    /// `(|(a)(b))`.
    Or(Vec<Filter>),
    /// `(!(a))`.
    Not(Box<Filter>),
    /// `(attr=value)`.
    Equal(String, Vec<u8>),
    /// `(attr=initial*any*final)`.
    Substrings {
        /// The attribute.
        attr: String,
        /// The part before the first star.
        initial: Option<Vec<u8>>,
        /// The parts between stars.
        any: Vec<Vec<u8>>,
        /// The part after the last star.
        last: Option<Vec<u8>>,
    },
    /// `(attr>=value)`.
    GreaterOrEqual(String, Vec<u8>),
    /// `(attr<=value)`.
    LessOrEqual(String, Vec<u8>),
    /// `(attr=*)`.
    Present(String),
    /// `(attr~=value)`.
    Approx(String, Vec<u8>),
}

// Context tags of the CHOICE of RFC 4511 section 4.5.1.
const AND: u8 = 0xa0;
const OR: u8 = 0xa1;
const NOT: u8 = 0xa2;
const EQUAL: u8 = 0xa3;
const SUBSTRINGS: u8 = 0xa4;
const GE: u8 = 0xa5;
const LE: u8 = 0xa6;
const PRESENT: u8 = 0x87;
const APPROX: u8 = 0xa8;
const EXTENSIBLE: u8 = 0xa9;
const SUB_INITIAL: u8 = 0x80;
const SUB_ANY: u8 = 0x81;
const SUB_FINAL: u8 = 0x82;

/// The deepest nesting that the parser and the reader accept.
const MAX_DEPTH: usize = 16;

impl Filter {
    /// Parses the string form of RFC 4515, such as `(&(objectClass=person)(uid=a*))`.
    pub fn parse(text: &str) -> Result<Filter, Malformed> {
        let mut p = Parser {
            s: text.as_bytes(),
            at: 0,
        };
        let f = p.filter(0)?;
        if p.at != p.s.len() {
            return Err(bad(format!(
                "the filter has text after its last parenthesis, at offset {}",
                p.at
            )));
        }
        Ok(f)
    }

    /// Encodes the filter as BER.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Filter::And(v) => {
                ber::constructed(AND, &v.iter().map(Filter::encode).collect::<Vec<_>>())
            }
            Filter::Or(v) => {
                ber::constructed(OR, &v.iter().map(Filter::encode).collect::<Vec<_>>())
            }
            Filter::Not(f) => ber::tlv(NOT, &f.encode()),
            Filter::Equal(a, v) => ava(EQUAL, a, v),
            Filter::GreaterOrEqual(a, v) => ava(GE, a, v),
            Filter::LessOrEqual(a, v) => ava(LE, a, v),
            Filter::Approx(a, v) => ava(APPROX, a, v),
            Filter::Present(a) => ber::octets(PRESENT, a.as_bytes()),
            Filter::Substrings {
                attr,
                initial,
                any,
                last,
            } => {
                let mut parts = Vec::new();
                if let Some(i) = initial {
                    parts.push(ber::octets(SUB_INITIAL, i));
                }
                for a in any {
                    parts.push(ber::octets(SUB_ANY, a));
                }
                if let Some(l) = last {
                    parts.push(ber::octets(SUB_FINAL, l));
                }
                ber::constructed(
                    SUBSTRINGS,
                    &[
                        ber::octets(ber::OCTET_STRING, attr.as_bytes()),
                        ber::constructed(ber::SEQUENCE, &parts),
                    ],
                )
            }
        }
    }

    /// Decodes a filter from its BER value.
    pub fn decode(t: Tlv<'_>) -> Result<Filter, Malformed> {
        decode(t, 0)
    }

    /// Evaluates the filter on an entry whose attributes are `attrs`.
    /// Attribute names compare without regard to case, and so do the values.
    pub fn matches(&self, attrs: &[(String, Vec<Vec<u8>>)]) -> bool {
        let values = |name: &str| -> Vec<&Vec<u8>> {
            let name = base_name(name);
            attrs
                .iter()
                .filter(|(a, _)| a.eq_ignore_ascii_case(name))
                .flat_map(|(_, v)| v.iter())
                .collect()
        };
        match self {
            Filter::And(v) => v.iter().all(|f| f.matches(attrs)),
            Filter::Or(v) => v.iter().any(|f| f.matches(attrs)),
            Filter::Not(f) => !f.matches(attrs),
            Filter::Present(a) => !values(a).is_empty(),
            Filter::Equal(a, want) | Filter::Approx(a, want) => {
                let want = fold(want);
                values(a).iter().any(|v| fold(v) == want)
            }
            Filter::GreaterOrEqual(a, want) => values(a)
                .iter()
                .any(|v| compare(v, want).is_some_and(|o| o.is_ge())),
            Filter::LessOrEqual(a, want) => values(a)
                .iter()
                .any(|v| compare(v, want).is_some_and(|o| o.is_le())),
            Filter::Substrings {
                attr,
                initial,
                any,
                last,
            } => values(attr).iter().any(|v| {
                let v = fold(v);
                let mut rest = v.as_str();
                if let Some(i) = initial {
                    let i = fold(i);
                    match rest.strip_prefix(i.as_str()) {
                        Some(r) => rest = r,
                        None => return false,
                    }
                }
                for a in any {
                    let a = fold(a);
                    match rest.find(a.as_str()) {
                        Some(p) => rest = &rest[p + a.len()..],
                        None => return false,
                    }
                }
                last.as_ref()
                    .is_none_or(|l| rest.ends_with(fold(l).as_str()))
            }),
        }
    }
}

impl std::fmt::Display for Filter {
    /// Renders the string form of RFC 4515 with the special octets escaped.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Filter::And(v) => {
                f.write_str("(&")?;
                v.iter().try_for_each(|x| write!(f, "{x}"))?;
                f.write_str(")")
            }
            Filter::Or(v) => {
                f.write_str("(|")?;
                v.iter().try_for_each(|x| write!(f, "{x}"))?;
                f.write_str(")")
            }
            Filter::Not(x) => write!(f, "(!{x})"),
            Filter::Equal(a, v) => write!(f, "({a}={})", escape(v)),
            Filter::GreaterOrEqual(a, v) => write!(f, "({a}>={})", escape(v)),
            Filter::LessOrEqual(a, v) => write!(f, "({a}<={})", escape(v)),
            Filter::Approx(a, v) => write!(f, "({a}~={})", escape(v)),
            Filter::Present(a) => write!(f, "({a}=*)"),
            Filter::Substrings {
                attr,
                initial,
                any,
                last,
            } => {
                write!(f, "({attr}=")?;
                if let Some(i) = initial {
                    f.write_str(&escape(i))?;
                }
                f.write_str("*")?;
                for a in any {
                    write!(f, "{}*", escape(a))?;
                }
                if let Some(l) = last {
                    f.write_str(&escape(l))?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Escapes a value for a filter string: `*`, `(`, `)`, `\`, and NUL as a
/// backslash and two hexadecimal digits, and every other octet that is not
/// printable ASCII as well.
pub fn escape(v: &[u8]) -> String {
    let mut s = String::with_capacity(v.len());
    for &b in v {
        if matches!(b, b'*' | b'(' | b')' | b'\\' | 0) || !(0x20..0x7f).contains(&b) {
            let _ = write!(s, "\\{b:02x}");
        } else {
            s.push(char::from(b));
        }
    }
    s
}

fn ava(tag: u8, attr: &str, value: &[u8]) -> Vec<u8> {
    ber::constructed(
        tag,
        &[
            ber::octets(ber::OCTET_STRING, attr.as_bytes()),
            ber::octets(ber::OCTET_STRING, value),
        ],
    )
}

fn decode(t: Tlv<'_>, depth: usize) -> Result<Filter, Malformed> {
    if depth > MAX_DEPTH {
        return Err(bad("the filter nests deeper than 16 levels"));
    }
    let list = |t: Tlv<'_>| -> Result<Vec<Filter>, Malformed> {
        let mut r = t.reader();
        let mut v = Vec::new();
        while !r.is_empty() {
            v.push(decode(r.next_any()?, depth + 1)?);
        }
        if v.is_empty() {
            return Err(bad("an and or or filter has no member"));
        }
        Ok(v)
    };
    let pair = |t: Tlv<'_>| -> Result<(String, Vec<u8>), Malformed> {
        let mut r = t.reader();
        let a = r
            .next(ber::OCTET_STRING, "the attribute of a filter")?
            .text()?;
        let v = r.next(ber::OCTET_STRING, "the value of a filter")?.content;
        r.finish("an attribute value assertion")?;
        Ok((a.to_owned(), v.to_vec()))
    };
    Ok(match t.tag {
        AND => Filter::And(list(t)?),
        OR => Filter::Or(list(t)?),
        NOT => {
            let mut r = t.reader();
            let inner = decode(r.next_any()?, depth + 1)?;
            r.finish("a not filter")?;
            Filter::Not(Box::new(inner))
        }
        EQUAL => {
            let (a, v) = pair(t)?;
            Filter::Equal(a, v)
        }
        GE => {
            let (a, v) = pair(t)?;
            Filter::GreaterOrEqual(a, v)
        }
        LE => {
            let (a, v) = pair(t)?;
            Filter::LessOrEqual(a, v)
        }
        APPROX => {
            let (a, v) = pair(t)?;
            Filter::Approx(a, v)
        }
        PRESENT => Filter::Present(t.text()?.to_owned()),
        SUBSTRINGS => {
            let mut r = t.reader();
            let attr = r
                .next(ber::OCTET_STRING, "the attribute of a filter")?
                .text()?;
            let mut parts = r
                .next(ber::SEQUENCE, "the substrings of a filter")?
                .reader();
            r.finish("a substring filter")?;
            let (mut initial, mut any, mut last) = (None, Vec::new(), None);
            while !parts.is_empty() {
                let p = parts.next_any()?;
                match p.tag {
                    SUB_INITIAL if initial.is_none() && any.is_empty() && last.is_none() => {
                        initial = Some(p.content.to_vec());
                    }
                    SUB_ANY if last.is_none() => any.push(p.content.to_vec()),
                    SUB_FINAL if last.is_none() => last = Some(p.content.to_vec()),
                    other => {
                        return Err(bad(format!(
                            "a substring part with the tag 0x{other:02x} is out of order"
                        )))
                    }
                }
            }
            if initial.is_none() && any.is_empty() && last.is_none() {
                return Err(bad("a substring filter has no part"));
            }
            Filter::Substrings {
                attr: attr.to_owned(),
                initial,
                any,
                last,
            }
        }
        EXTENSIBLE => return Err(bad("extensible match filters are not supported")),
        other => return Err(bad(format!("a filter has the unknown tag 0x{other:02x}"))),
    })
}

/// The attribute name without options, so `cn;lang-en` finds `cn`.
fn base_name(name: &str) -> &str {
    name.split(';').next().unwrap_or(name)
}

/// The caseIgnoreMatch form of a value: lower case, trimmed, runs of spaces
/// collapsed to one.
fn fold(v: &[u8]) -> String {
    let s = String::from_utf8_lossy(v).to_lowercase();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Orders two values as integers when both are, and as folded text otherwise.
fn compare(a: &[u8], b: &[u8]) -> Option<std::cmp::Ordering> {
    let (sa, sb) = (fold(a), fold(b));
    match (sa.parse::<i64>(), sb.parse::<i64>()) {
        (Ok(x), Ok(y)) => Some(x.cmp(&y)),
        _ => Some(sa.cmp(&sb)),
    }
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn eat(&mut self, c: u8) -> Result<(), Malformed> {
        if self.s.get(self.at) == Some(&c) {
            self.at += 1;
            Ok(())
        } else {
            Err(bad(format!(
                "the filter needs '{}' at offset {}",
                char::from(c),
                self.at
            )))
        }
    }

    fn filter(&mut self, depth: usize) -> Result<Filter, Malformed> {
        if depth > MAX_DEPTH {
            return Err(bad("the filter nests deeper than 16 levels"));
        }
        self.eat(b'(')?;
        let f = match self.s.get(self.at) {
            Some(b'&') => {
                self.at += 1;
                Filter::And(self.list(depth)?)
            }
            Some(b'|') => {
                self.at += 1;
                Filter::Or(self.list(depth)?)
            }
            Some(b'!') => {
                self.at += 1;
                Filter::Not(Box::new(self.filter(depth + 1)?))
            }
            _ => self.item()?,
        };
        self.eat(b')')?;
        Ok(f)
    }

    fn list(&mut self, depth: usize) -> Result<Vec<Filter>, Malformed> {
        let mut v = Vec::new();
        while self.s.get(self.at) == Some(&b'(') {
            v.push(self.filter(depth + 1)?);
        }
        if v.is_empty() {
            return Err(bad(format!(
                "the and or or filter at offset {} has no member",
                self.at
            )));
        }
        Ok(v)
    }

    fn item(&mut self) -> Result<Filter, Malformed> {
        let start = self.at;
        while let Some(&c) = self.s.get(self.at) {
            if matches!(c, b'=' | b'<' | b'>' | b'~' | b'(' | b')') {
                break;
            }
            self.at += 1;
        }
        let attr = std::str::from_utf8(&self.s[start..self.at])
            .map_err(|_| bad("an attribute name is not UTF-8"))?
            .to_owned();
        if attr.is_empty()
            || !attr
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b';' | b'_'))
        {
            return Err(bad(format!(
                "the attribute name '{attr}' at offset {start} is empty or has a character that is not allowed"
            )));
        }
        let op = match self.s.get(self.at) {
            Some(b'=') => {
                self.at += 1;
                b'='
            }
            Some(&c @ (b'<' | b'>' | b'~')) => {
                self.at += 1;
                self.eat(b'=')?;
                c
            }
            _ => {
                return Err(bad(format!(
                    "the filter needs a comparison after '{attr}' at offset {}",
                    self.at
                )))
            }
        };
        // The value runs to the closing parenthesis. A star splits it.
        let start = self.at;
        while let Some(&c) = self.s.get(self.at) {
            if c == b')' {
                break;
            }
            if c == b'(' {
                return Err(bad(format!(
                    "the value at offset {} has an unescaped '('",
                    self.at
                )));
            }
            self.at += 1;
        }
        let raw = &self.s[start..self.at];
        let pieces: Vec<&[u8]> = raw.split(|b| *b == b'*').collect();
        match op {
            b'=' if pieces.len() == 1 => Ok(Filter::Equal(attr, unescape(raw)?)),
            b'=' if raw == b"*" => Ok(Filter::Present(attr)),
            b'=' => {
                let n = pieces.len();
                let initial = (!pieces[0].is_empty())
                    .then(|| unescape(pieces[0]))
                    .transpose()?;
                let last = (!pieces[n - 1].is_empty())
                    .then(|| unescape(pieces[n - 1]))
                    .transpose()?;
                let mut any = Vec::new();
                for p in &pieces[1..n - 1] {
                    if p.is_empty() {
                        return Err(bad("a substring filter has two stars in a row"));
                    }
                    any.push(unescape(p)?);
                }
                Ok(Filter::Substrings {
                    attr,
                    initial,
                    any,
                    last,
                })
            }
            _ if pieces.len() != 1 => Err(bad("only an equality filter may hold a star")),
            b'>' => Ok(Filter::GreaterOrEqual(attr, unescape(raw)?)),
            b'<' => Ok(Filter::LessOrEqual(attr, unescape(raw)?)),
            _ => Ok(Filter::Approx(attr, unescape(raw)?)),
        }
    }
}

/// Decodes the `\xx` escapes of a filter value.
fn unescape(raw: &[u8]) -> Result<Vec<u8>, Malformed> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' {
            let hex = raw
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            let Some(b) = hex else {
                return Err(bad(
                    "a backslash in a filter value needs two hexadecimal digits",
                ));
            };
            out.push(b);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ber::Reader;

    fn entry(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<Vec<u8>>)> {
        pairs
            .iter()
            .map(|(a, v)| {
                (
                    (*a).to_owned(),
                    v.iter().map(|x| x.as_bytes().to_vec()).collect(),
                )
            })
            .collect()
    }

    fn round(text: &str) -> Filter {
        let f = Filter::parse(text).unwrap();
        let enc = f.encode();
        let back = Filter::decode(Reader::new(&enc).next_any().unwrap()).unwrap();
        assert_eq!(back, f, "{text}");
        assert_eq!(back.to_string(), text, "{text}");
        f
    }

    #[test]
    fn the_rfc_4515_examples_round_trip_through_ber() {
        for text in [
            "(cn=Babs Jensen)",
            "(!(cn=Tim Howes))",
            "(&(objectClass=Person)(|(sn=Jensen)(cn=Babs J*)))",
            "(o=univ*of*mich*)",
            "(seeAlso=)",
            "(cn=*)",
            "(uidNumber>=1000)",
            "(uidNumber<=2000)",
            "(cn~=jensen)",
            "(cn=*Jensen)",
            "(cn=a*b*c*d)",
            "(cn=Lu\\28i\\29\\5c\\2a)",
        ] {
            round(text);
        }
    }

    #[test]
    fn the_ber_of_an_equality_filter_is_the_known_one() {
        // (uid=jdoe): A3 0B 04 03 'uid' 04 04 'jdoe'.
        let f = Filter::parse("(uid=jdoe)").unwrap();
        assert_eq!(
            f.encode(),
            [0xa3, 0x0b, 0x04, 0x03, b'u', b'i', b'd', 0x04, 0x04, b'j', b'd', b'o', b'e']
        );
        // (objectClass=*): 87 0B 'objectClass'.
        assert_eq!(
            Filter::parse("(objectClass=*)").unwrap().encode(),
            [&[0x87, 0x0b][..], b"objectClass"].concat()
        );
    }

    #[test]
    fn the_parser_rejects_malformed_filters() {
        for bad in [
            "",
            "cn=a",
            "(cn=a",
            "(cn=a))",
            "(&)",
            "(|)",
            "(cn)",
            "(=a)",
            "(c n=a)",
            "(cn=a(b)",
            "(cn=\\4)",
            "(cn=\\zz)",
            "(cn>=a*)",
            "(cn=a**b)",
            "(!(cn=a)(cn=b))",
            "(cn=a)(cn=b)",
            "(&(cn=a)",
            "(cn=a)x",
        ] {
            assert!(Filter::parse(bad).is_err(), "{bad:?} parsed");
        }
        let deep = format!("{}(cn=a){}", "(!".repeat(40), ")".repeat(40));
        assert!(Filter::parse(&deep).unwrap_err().0.contains("deeper"));
    }

    #[test]
    fn the_reader_rejects_malformed_filters() {
        for bytes in [
            &[0xa0, 0x00][..],                               // and without a member
            &[0xa4, 0x05, 0x04, 0x01, b'a', 0x30, 0x00][..], // substrings without a part
            &[0xa9, 0x00][..],                               // extensible match
            &[0x90, 0x00][..],                               // unknown tag
            &[0xa3, 0x03, 0x04, 0x01, b'a'][..],             // value missing
        ] {
            let t = Reader::new(bytes).next_any().unwrap();
            assert!(Filter::decode(t).is_err(), "{bytes:02x?}");
        }
    }

    #[test]
    fn evaluation_follows_the_matching_rules() {
        let e = entry(&[
            ("objectClass", &["top", "inetOrgPerson"]),
            ("uid", &["jdoe"]),
            ("cn", &["John  DOE"]),
            ("uidNumber", &["1042"]),
            ("mail", &["jdoe@example.test", "john@example.test"]),
        ]);
        let yes = [
            "(uid=JDOE)",
            "(cn=john doe)",
            "(objectclass=inetorgperson)",
            "(mail=john@example.test)",
            "(uidNumber>=1000)",
            "(uidNumber<=1042)",
            "(uidNumber>=999)",
            "(cn=jo*)",
            "(cn=*doe)",
            "(cn=*hn d*)",
            "(mail=*@example.test)",
            "(&(uid=jdoe)(objectClass=top))",
            "(|(uid=nobody)(uid=jdoe))",
            "(!(uid=nobody))",
            "(!(missing=x))",
            "(uid=*)",
            "(cn;lang-en=*)",
            "(cn~=JOHN DOE)",
        ];
        for t in yes {
            assert!(Filter::parse(t).unwrap().matches(&e), "{t} must match");
        }
        let no = [
            "(uid=jdoe2)",
            "(uidNumber>=1043)",
            "(uidNumber<=1041)",
            "(missing=*)",
            "(cn=x*)",
            "(cn=*x)",
            "(cn=j*h*n*x)",
            "(&(uid=jdoe)(uid=nobody))",
            "(!(uid=jdoe))",
        ];
        for t in no {
            assert!(!Filter::parse(t).unwrap().matches(&e), "{t} must not match");
        }
    }

    #[test]
    fn substring_parts_match_in_order_and_do_not_overlap() {
        let e = entry(&[("cn", &["abcabc"])]);
        assert!(Filter::parse("(cn=ab*ca*bc)").unwrap().matches(&e));
        assert!(!Filter::parse("(cn=abc*bca*abc)").unwrap().matches(&e));
        assert!(!Filter::parse("(cn=abcabc*c)").unwrap().matches(&e));
    }

    #[test]
    fn escape_covers_the_special_octets() {
        assert_eq!(escape(b"a*b(c)d\\e"), "a\\2ab\\28c\\29d\\5ce");
        assert_eq!(escape(&[0, 0xc3, 0xa9]), "\\00\\c3\\a9");
        assert_eq!(unescape(b"\\c3\\a9").unwrap(), "é".as_bytes());
    }
}
