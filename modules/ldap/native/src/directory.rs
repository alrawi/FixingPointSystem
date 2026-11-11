// SPDX-License-Identifier: GPL-3.0-only
//! The directory fixture: entries in LDIF (RFC 2849), and the bind and search
//! logic that answers from them.
//!
//! The supported LDIF is plain: entries separated by blank lines, a `dn:` line
//! first, `attribute: value` lines, folded lines that continue with one space,
//! and `#` comments. Values in base64 (`::`) and URLs (`:<`) are not
//! supported, and the parser says so.

use crate::ber::{bad, Malformed};
use crate::message::{code, Attribute, Auth, Entry, LdapResult, SearchRequest};

/// The attribute that holds the secret of a bind. It never leaves in a search
/// result, as a real server hides it with an access rule.
const PASSWORD_ATTR: &str = "userPassword";

/// The most entries that a directory fixture may hold.
pub const MAX_ENTRIES: usize = 5_000;

/// A directory: its entries in the order of the fixture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directory {
    entries: Vec<Entry>,
}

/// The outcome of a search: the entries to send, then the final result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The entries, already cut to the size limit and trimmed to the requested
    /// attributes.
    pub entries: Vec<Entry>,
    /// The result for the `SearchResultDone`.
    pub result: LdapResult,
}

/// Parses attribute lines of LDIF, without a DN line, into attributes. The
/// lines of one name collect into one attribute with several values.
pub fn parse_attributes(text: &str) -> Result<Vec<Attribute>, Malformed> {
    let mut attrs: Vec<Attribute> = Vec::new();
    for line in unfold(text) {
        let (name, value) = split_line(&line)?;
        match attrs
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(&name))
        {
            Some((_, v)) => v.push(value.into_bytes()),
            None => attrs.push((name, vec![value.into_bytes()])),
        }
    }
    Ok(attrs)
}

/// Joins folded lines and drops comments and blank lines.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix(' ') {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        if raw.starts_with('#') || raw.trim().is_empty() {
            continue;
        }
        out.push(raw.trim_end().to_owned());
    }
    out
}

fn split_line(line: &str) -> Result<(String, String), Malformed> {
    let Some((name, rest)) = line.split_once(':') else {
        return Err(bad(format!("the LDIF line '{line}' has no colon")));
    };
    if rest.starts_with(':') || rest.starts_with('<') {
        return Err(bad(format!(
            "the LDIF line '{line}' uses a base64 or URL value, which is not supported"
        )));
    }
    let name = name.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b';'))
    {
        return Err(bad(format!("'{name}' is not an LDIF attribute name")));
    }
    Ok((
        name.to_owned(),
        rest.strip_prefix(' ').unwrap_or(rest).to_owned(),
    ))
}

/// The form of a DN that compares equal for equal DNs: lower case, with no
/// space around the commas, plus signs, and equals signs.
pub fn normalize(dn: &str) -> String {
    let mut out = String::with_capacity(dn.len());
    let mut pending_space = false;
    for c in dn.chars() {
        match c {
            ' ' => pending_space = true,
            ',' | '+' | '=' => {
                pending_space = false;
                out.push(c);
            }
            _ => {
                if pending_space && !out.is_empty() && !out.ends_with([',', '+', '=']) {
                    out.push(' ');
                }
                pending_space = false;
                out.extend(c.to_lowercase());
            }
        }
    }
    out
}

/// The DN without its first component, or the empty string.
fn parent(norm: &str) -> &str {
    let mut escaped = false;
    for (i, c) in norm.char_indices() {
        match c {
            '\\' => escaped = !escaped,
            ',' if !escaped => return &norm[i + 1..],
            _ => escaped = false,
        }
    }
    ""
}

impl Directory {
    /// Parses the LDIF of a fixture.
    pub fn parse(text: &str) -> Result<Directory, Malformed> {
        let mut entries = Vec::new();
        let mut block = String::new();
        for line in text.lines().chain(std::iter::once("")) {
            if line.trim().is_empty() && !line.starts_with(' ') {
                if !unfold(&block).is_empty() {
                    entries.push(parse_entry(&block)?);
                    if entries.len() > MAX_ENTRIES {
                        return Err(bad(format!(
                            "the directory holds more than {MAX_ENTRIES} entries"
                        )));
                    }
                }
                block.clear();
            } else {
                block.push_str(line);
                block.push('\n');
            }
        }
        let mut seen = std::collections::HashSet::new();
        for e in &entries {
            if !seen.insert(normalize(&e.dn)) {
                return Err(bad(format!("the directory has the DN '{}' twice", e.dn)));
            }
        }
        Ok(Directory { entries })
    }

    /// The entries in the order of the fixture.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn find(&self, dn: &str) -> Option<&Entry> {
        let want = normalize(dn);
        self.entries.iter().find(|e| normalize(&e.dn) == want)
    }

    /// Answers a bind request.
    pub fn bind(&self, version: i64, name: &str, auth: &Auth) -> LdapResult {
        let result = |code: i64, diagnostic: &str| LdapResult {
            code,
            matched_dn: String::new(),
            diagnostic: diagnostic.to_owned(),
        };
        if version != 3 {
            return result(
                code::PROTOCOL_ERROR,
                "historical protocol version requested, use LDAPv3 instead",
            );
        }
        let password = match auth {
            Auth::Simple(p) => p,
            Auth::Sasl(m) => {
                return result(
                    code::AUTH_METHOD_NOT_SUPPORTED,
                    &format!("SASL mechanism {m} is not supported"),
                )
            }
        };
        match (name.is_empty(), password.is_empty()) {
            // An anonymous bind.
            (true, true) => result(code::SUCCESS, ""),
            (false, true) => result(
                code::UNWILLING_TO_PERFORM,
                "unauthenticated bind (DN with no password) disallowed",
            ),
            (true, false) => result(
                code::UNWILLING_TO_PERFORM,
                "DN is empty and a password is given",
            ),
            (false, false) => {
                let known = self.find(name).is_some_and(|e| {
                    e.attrs
                        .iter()
                        .filter(|(n, _)| n.eq_ignore_ascii_case(PASSWORD_ATTR))
                        .flat_map(|(_, v)| v)
                        .any(|v| v == password)
                });
                if known {
                    result(code::SUCCESS, "")
                } else {
                    result(code::INVALID_CREDENTIALS, "")
                }
            }
        }
    }

    /// Answers a search request.
    pub fn search(&self, req: &SearchRequest) -> Outcome {
        let base = normalize(&req.base);
        let done = |code: i64, matched_dn: String, diagnostic: &str| LdapResult {
            code,
            matched_dn,
            diagnostic: diagnostic.to_owned(),
        };
        // The base object must exist, except for the root DSE, which the
        // fixture may leave out.
        if !base.is_empty() && self.find(&req.base).is_none() {
            let mut matched = String::new();
            let mut up = parent(&base);
            while !up.is_empty() {
                if let Some(e) = self.entries.iter().find(|e| normalize(&e.dn) == up) {
                    matched = e.dn.clone();
                    break;
                }
                up = parent(up);
            }
            return Outcome {
                entries: Vec::new(),
                result: done(code::NO_SUCH_OBJECT, matched, ""),
            };
        }
        let in_scope = |e: &&Entry| {
            let dn = normalize(&e.dn);
            match req.scope {
                0 => dn == base,
                1 => !dn.is_empty() && parent(&dn) == base,
                _ => {
                    dn == base
                        || (!dn.is_empty()
                            && (base.is_empty() || dn.ends_with(&format!(",{base}"))))
                }
            }
        };
        let mut found: Vec<&Entry> = self
            .entries
            .iter()
            .filter(in_scope)
            .filter(|e| req.filter.matches(&e.attrs))
            .collect();
        let mut result = done(code::SUCCESS, String::new(), "");
        if req.size_limit > 0 && found.len() as i64 > req.size_limit {
            found.truncate(req.size_limit as usize);
            result = done(code::SIZE_LIMIT_EXCEEDED, String::new(), "");
        }
        Outcome {
            entries: found.into_iter().map(|e| trim(e, req)).collect(),
            result,
        }
    }

    /// The seed-th entry that has a `userPassword`, wrapping around, or one
    /// picked with `pick` when `seed` is negative.
    pub fn account(&self, seed: Option<u64>, pick: impl FnOnce(usize) -> usize) -> Option<&Entry> {
        let accounts: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| {
                e.attrs
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case(PASSWORD_ATTR))
            })
            .collect();
        if accounts.is_empty() {
            return None;
        }
        let i = match seed {
            Some(s) => (s % accounts.len() as u64) as usize,
            None => pick(accounts.len()) % accounts.len(),
        };
        accounts.get(i).copied()
    }
}

/// The first value of the attribute `name` of an entry, or its DN for `dn`.
pub fn first_value(e: &Entry, name: &str) -> Option<Vec<u8>> {
    if name.eq_ignore_ascii_case("dn") {
        return Some(e.dn.clone().into_bytes());
    }
    e.attrs
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| v.first().cloned())
}

fn parse_entry(block: &str) -> Result<Entry, Malformed> {
    let mut lines = unfold(block).into_iter();
    let Some(first) = lines.next() else {
        return Err(bad("an LDIF entry is empty"));
    };
    let (name, dn) = split_line(&first)?;
    if !name.eq_ignore_ascii_case("dn") {
        return Err(bad(format!(
            "an LDIF entry starts with '{first}' and not with dn:"
        )));
    }
    let rest: Vec<String> = lines.collect();
    let mut attrs: Vec<Attribute> = Vec::new();
    for line in &rest {
        let (n, v) = split_line(line)?;
        match attrs.iter_mut().find(|(a, _)| a.eq_ignore_ascii_case(&n)) {
            Some((_, values)) => values.push(v.into_bytes()),
            None => attrs.push((n, vec![v.into_bytes()])),
        }
    }
    Ok(Entry { dn, attrs })
}

/// Cuts an entry to the attributes that the request names. No attribute list
/// and `*` select every user attribute, `1.1` selects none, and
/// `types_only` drops the values. The password never leaves.
fn trim(e: &Entry, req: &SearchRequest) -> Entry {
    let all = req.attributes.is_empty() || req.attributes.iter().any(|a| a == "*");
    let wanted = |name: &str| {
        all || req
            .attributes
            .iter()
            .any(|a| a.eq_ignore_ascii_case(name.split(';').next().unwrap_or(name)))
    };
    let attrs = e
        .attrs
        .iter()
        .filter(|(n, _)| !n.eq_ignore_ascii_case(PASSWORD_ATTR) && wanted(n))
        .map(|(n, v)| {
            (
                n.clone(),
                if req.types_only {
                    Vec::new()
                } else {
                    v.clone()
                },
            )
        })
        .collect();
    Entry {
        dn: e.dn.clone(),
        attrs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Filter;

    const LDIF: &str = "\
# The root DSE has an empty DN.
dn:
objectClass: top
namingContexts: dc=example,dc=test
supportedLDAPVersion: 3

dn: dc=example,dc=test
objectClass: top
objectClass: domain
dc: example

dn: ou=People,dc=example,dc=test
objectClass: organizationalUnit
ou: People

dn: uid=jdoe,ou=People,dc=example,dc=test
objectClass: inetOrgPerson
uid: jdoe
cn: John Doe
sn: Doe
mail: jdoe@example.test
description: A very long description that
  continues on a folded line
userPassword: Tst-alpha-1

dn: uid=asmith,ou=People,dc=example,dc=test
objectClass: inetOrgPerson
uid: asmith
cn: Anna Smith
sn: Smith
userPassword: Tst-beta-2

dn: cn=admin,dc=example,dc=test
objectClass: organizationalRole
cn: admin
userPassword: Tst-root-3
";

    fn dir() -> Directory {
        Directory::parse(LDIF).unwrap()
    }

    fn req(base: &str, scope: i64, filter: &str, attrs: &[&str]) -> SearchRequest {
        SearchRequest {
            base: base.into(),
            scope,
            deref: 0,
            size_limit: 0,
            time_limit: 0,
            types_only: false,
            filter: Filter::parse(filter).unwrap(),
            attributes: attrs.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    fn dns(o: &Outcome) -> Vec<&str> {
        o.entries.iter().map(|e| e.dn.as_str()).collect()
    }

    #[test]
    fn the_ldif_parser_reads_entries_folds_and_the_root_dse() {
        let d = dir();
        assert_eq!(d.entries().len(), 6);
        assert_eq!(d.entries()[0].dn, "");
        let jdoe = &d.entries()[3];
        let desc = jdoe.attrs.iter().find(|(n, _)| n == "description").unwrap();
        assert_eq!(
            desc.1[0],
            b"A very long description that continues on a folded line"
        );
        let classes = &d.entries()[1].attrs[0].1;
        assert_eq!(classes.len(), 2);
    }

    #[test]
    fn the_ldif_parser_rejects_what_it_does_not_support() {
        for (text, why) in [
            ("dn: a=b\nfoo:: AAAA\n", "base64"),
            ("dn: a=b\nfoo:< file:///x\n", "url"),
            ("cn: nodn\n", "no dn first"),
            ("dn: a=b\nnocolon\n", "no colon"),
            ("dn: a=b\n\ndn: A = B\n", "duplicate dn after normalizing"),
            ("dn: a=b\nb c: x\n", "space in attribute name"),
        ] {
            assert!(Directory::parse(text).is_err(), "{why}");
        }
        assert_eq!(Directory::parse("").unwrap().entries().len(), 0);
    }

    #[test]
    fn dns_compare_without_case_and_spacing() {
        assert_eq!(
            normalize("UID=jdoe , OU=People,dc = Example , dc=test"),
            "uid=jdoe,ou=people,dc=example,dc=test"
        );
        assert_eq!(normalize("cn=John  Doe"), "cn=john doe");
        assert_eq!(parent("uid=a,ou=b,dc=c"), "ou=b,dc=c");
        assert_eq!(parent("dc=c"), "");
        assert_eq!(parent("cn=a\\,b,dc=c"), "dc=c");
    }

    #[test]
    fn a_subtree_search_finds_the_user_and_hides_the_password() {
        let o = dir().search(&req("dc=example,dc=test", 2, "(uid=JDoe)", &[]));
        assert_eq!(dns(&o), ["uid=jdoe,ou=People,dc=example,dc=test"]);
        assert_eq!(o.result.code, code::SUCCESS);
        let names: Vec<&str> = o.entries[0].attrs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("userPassword")));
        assert!(names.contains(&"mail"));
        // A filter on the password does not find it either through the wire form.
        let o = dir().search(&req(
            "dc=example,dc=test",
            2,
            "(cn=admin)",
            &["userPassword"],
        ));
        assert_eq!(o.entries[0].attrs.len(), 0);
    }

    #[test]
    fn scopes_select_the_base_its_children_and_its_subtree() {
        let d = dir();
        let base = d.search(&req(
            "ou=People,dc=example,dc=test",
            0,
            "(objectClass=*)",
            &[],
        ));
        assert_eq!(dns(&base), ["ou=People,dc=example,dc=test"]);
        let one = d.search(&req(
            "ou=people, dc=example, dc=test",
            1,
            "(objectClass=*)",
            &[],
        ));
        assert_eq!(
            dns(&one),
            [
                "uid=jdoe,ou=People,dc=example,dc=test",
                "uid=asmith,ou=People,dc=example,dc=test"
            ]
        );
        let sub = d.search(&req(
            "ou=People,dc=example,dc=test",
            2,
            "(objectClass=*)",
            &[],
        ));
        assert_eq!(sub.entries.len(), 3);
        let all = d.search(&req("dc=example,dc=test", 2, "(objectClass=*)", &[]));
        assert_eq!(all.entries.len(), 5);
    }

    #[test]
    fn the_root_dse_answers_a_base_search_with_an_empty_dn() {
        let o = dir().search(&req(
            "",
            0,
            "(objectClass=*)",
            &["namingContexts", "supportedLDAPVersion"],
        ));
        assert_eq!(o.entries.len(), 1);
        assert_eq!(o.entries[0].dn, "");
        assert_eq!(o.entries[0].attrs.len(), 2);
        assert_eq!(o.result.code, code::SUCCESS);
    }

    #[test]
    fn attribute_selection_follows_rfc_4511() {
        let d = dir();
        let sel = |attrs: &[&str], types_only: bool| {
            let mut r = req("uid=jdoe,ou=People,dc=example,dc=test", 0, "(uid=*)", attrs);
            r.types_only = types_only;
            d.search(&r).entries.remove(0).attrs
        };
        assert_eq!(sel(&["cn", "MAIL"], false).len(), 2);
        assert_eq!(sel(&["*"], false).len(), sel(&[], false).len());
        assert!(sel(&["1.1"], false).is_empty());
        let t = sel(&[], true);
        assert!(!t.is_empty() && t.iter().all(|(_, v)| v.is_empty()));
    }

    #[test]
    fn a_missing_base_gives_no_such_object_with_the_matched_dn() {
        let o = dir().search(&req(
            "uid=ghost,ou=People,dc=example,dc=test",
            2,
            "(objectClass=*)",
            &[],
        ));
        assert!(o.entries.is_empty());
        assert_eq!(o.result.code, code::NO_SUCH_OBJECT);
        assert_eq!(o.result.matched_dn, "ou=People,dc=example,dc=test");
        let o = dir().search(&req("dc=other,dc=test", 2, "(objectClass=*)", &[]));
        assert_eq!(o.result.matched_dn, "");
    }

    #[test]
    fn the_size_limit_cuts_the_result_and_says_so() {
        let mut r = req("dc=example,dc=test", 2, "(objectClass=*)", &[]);
        r.size_limit = 2;
        let o = dir().search(&r);
        assert_eq!(o.entries.len(), 2);
        assert_eq!(o.result.code, code::SIZE_LIMIT_EXCEEDED);
        r.size_limit = 5;
        assert_eq!(dir().search(&r).result.code, code::SUCCESS);
    }

    #[test]
    fn binds_follow_rfc_4513() {
        let d = dir();
        let pw = |p: &str| Auth::Simple(p.as_bytes().to_vec());
        let dn = "uid=jdoe,ou=People,dc=example,dc=test";
        assert_eq!(d.bind(3, dn, &pw("Tst-alpha-1")).code, code::SUCCESS);
        assert_eq!(
            d.bind(
                3,
                "UID=JDOE, OU=People,dc=example,dc=test",
                &pw("Tst-alpha-1")
            )
            .code,
            code::SUCCESS
        );
        assert_eq!(
            d.bind(3, dn, &pw("Tst-alpha-2")).code,
            code::INVALID_CREDENTIALS
        );
        assert_eq!(
            d.bind(3, "uid=ghost,dc=example,dc=test", &pw("x")).code,
            code::INVALID_CREDENTIALS
        );
        assert_eq!(d.bind(3, "", &pw("")).code, code::SUCCESS);
        assert_eq!(d.bind(3, dn, &pw("")).code, code::UNWILLING_TO_PERFORM);
        assert_eq!(d.bind(3, "", &pw("x")).code, code::UNWILLING_TO_PERFORM);
        assert_eq!(d.bind(2, dn, &pw("Tst-alpha-1")).code, code::PROTOCOL_ERROR);
        assert_eq!(
            d.bind(3, dn, &Auth::Sasl("GSSAPI".into())).code,
            code::AUTH_METHOD_NOT_SUPPORTED
        );
        // An entry without a password cannot be bound to.
        assert_eq!(
            d.bind(3, "ou=People,dc=example,dc=test", &pw("x")).code,
            code::INVALID_CREDENTIALS
        );
    }

    #[test]
    fn accounts_are_the_entries_with_a_password() {
        let d = dir();
        let first = d.account(Some(0), |_| 0).unwrap();
        assert_eq!(first.dn, "uid=jdoe,ou=People,dc=example,dc=test");
        // Three accounts, so seed 4 wraps to the second.
        assert_eq!(
            d.account(Some(4), |_| 0).unwrap().dn,
            "uid=asmith,ou=People,dc=example,dc=test"
        );
        assert_eq!(
            d.account(None, |n| n - 1).unwrap().dn,
            "cn=admin,dc=example,dc=test"
        );
        assert!(Directory::default().account(Some(0), |_| 0).is_none());
        assert_eq!(first_value(first, "dn").unwrap(), first.dn.as_bytes());
        assert_eq!(first_value(first, "UID").unwrap(), b"jdoe");
        assert!(first_value(first, "missing").is_none());
    }
}
