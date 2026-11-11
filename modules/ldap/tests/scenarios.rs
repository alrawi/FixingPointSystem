// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests of the ldap scenarios. Every scenario runs with both
//! sides against each other over an in memory network, and the bytes of the
//! connection are decoded with the BER codec of the crate: the messages must
//! follow one another as the scenario says, with matching message ids, the
//! right result codes, and the entries that the directory holds.

mod common;

use std::collections::BTreeSet;

use common::{run, scenarios, scenarios_with, Conn, Run, SEED};
use fps_flow::Value;
use fps_interp::{FlowState, Side};
use fps_module_ldap::ber;
use fps_module_ldap::directory::Directory;
use fps_module_ldap::message::{Auth, Message, Op};
use fps_module_ldap::DEFAULT_DIRECTORY;

/// Splits what one side wrote into its messages, and requires that nothing is
/// left over.
fn messages(wire: &[u8]) -> Vec<Message> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < wire.len() {
        let (tag, head, content) = ber::head(&wire[at..]).expect("a head");
        assert_eq!(tag, 0x30, "an LDAPMessage is a sequence");
        let end = at + head + content;
        assert!(end <= wire.len(), "a message is cut off at {at}");
        out.push(Message::decode(&wire[at..end]).expect("a message that decodes"));
        at = end;
    }
    out
}

fn names(m: &[Message]) -> Vec<&'static str> {
    m.iter().map(|m| m.op.name()).collect()
}

fn session(run: &Run) -> Conn {
    let conns = run.conns_of("session");
    assert_eq!(conns.len(), 1, "one connection per session");
    assert_eq!(conns[0].port, 389);
    conns[0].clone()
}

/// The checks that every scenario passes in every session. Returns the
/// messages of the client and of the server.
fn check(name: &str, run: &Run) -> (Vec<Message>, Vec<Message>) {
    if let Err(e) = &run.result {
        panic!("{name}: {e}");
    }
    for (flow, state) in &run.states {
        assert_eq!(*state, Some(FlowState::Done), "{name} flow {flow}");
    }
    assert!(run.records.iter().all(|r| r.failed.is_none()), "{name}");
    let conn = session(run);
    let (c, s) = (messages(&conn.c2s()), messages(&conn.s2c()));
    // The client numbers its requests 1, 2, 3 and so on, and ends with an unbind.
    let ids: Vec<i64> = c.iter().map(|m| m.id).collect();
    assert_eq!(ids, (1..=c.len() as i64).collect::<Vec<_>>(), "{name}");
    assert_eq!(
        c.last().map(|m| m.op.name()),
        Some("unbind_request"),
        "{name}"
    );
    // Every response carries the id of its request, in ascending order, and
    // every request but the unbind gets an answer that ends the exchange.
    let answered: BTreeSet<i64> = s.iter().map(|m| m.id).collect();
    assert_eq!(
        answered,
        (1..c.len() as i64).collect::<BTreeSet<_>>(),
        "{name}"
    );
    assert!(s.windows(2).all(|w| w[0].id <= w[1].id), "{name}");
    for pair in s.windows(2) {
        // A bind response or a search done ends an exchange.
        if matches!(pair[0].op, Op::BindResponse(_) | Op::SearchDone(_)) {
            assert!(pair[1].id > pair[0].id, "{name}: two ends for one request");
        }
    }
    (c, s)
}

fn directory() -> Directory {
    Directory::parse(DEFAULT_DIRECTORY).unwrap()
}

fn result_codes(s: &[Message]) -> Vec<i64> {
    s.iter()
        .filter_map(|m| match &m.op {
            Op::BindResponse(r) | Op::SearchDone(r) => Some(r.code),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn every_scenario_completes_in_every_session() {
    let all = scenarios();
    assert_eq!(all.len(), 4);
    for (name, s) in &all {
        for session_id in [1, 7, 301] {
            let r = run(s, SEED, session_id).await;
            check(name, &r);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn bind_search_binds_as_an_account_and_finds_its_entry() {
    let all = scenarios();
    let (name, s) = all.iter().find(|(n, _)| n == "bind_search").unwrap();
    for session_id in 1..=8 {
        let r = run(s, SEED, session_id).await;
        let (c, srv) = check(name, &r);
        assert_eq!(
            names(&c),
            [
                "bind_request",
                "bind_request",
                "search_request",
                "unbind_request"
            ]
        );
        assert_eq!(
            names(&srv),
            [
                "bind_response",
                "bind_response",
                "search_entry",
                "search_done"
            ]
        );
        // The first bind is anonymous.
        let Op::BindRequest {
            name: anon,
            auth: Auth::Simple(anon_pw),
            ..
        } = &c[0].op
        else {
            panic!("an anonymous bind")
        };
        assert!(anon.is_empty() && anon_pw.is_empty());
        let Op::BindRequest {
            version,
            name: dn,
            auth,
        } = &c[1].op
        else {
            unreachable!()
        };
        assert_eq!(*version, 3);
        let Auth::Simple(password) = auth else {
            panic!("a simple bind")
        };
        // The account exists and the password is the one of the directory.
        let d = directory();
        let entry = d
            .entries()
            .iter()
            .find(|e| e.dn == *dn)
            .unwrap_or_else(|| panic!("{dn} is not in the directory"));
        let stored = &entry
            .attrs
            .iter()
            .find(|(n, _)| n == "userPassword")
            .unwrap()
            .1[0];
        assert_eq!(password, stored);
        assert!(password.starts_with(b"Tst-"));
        assert_eq!(result_codes(&srv), [0, 0, 0]);
        // The search asks for the uid of that account and gets the entry back.
        let Op::SearchRequest(q) = &c[2].op else {
            unreachable!()
        };
        assert_eq!(
            q.filter.to_string(),
            format!("(uid={})", dn.split([',', '=']).nth(1).unwrap())
        );
        assert_eq!(q.scope, 2);
        assert_eq!(q.attributes.len(), 5);
        let Op::SearchEntry(e) = &srv[2].op else {
            unreachable!()
        };
        assert_eq!(e.dn, *dn);
        let got: Vec<&str> = e.attrs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            got,
            ["cn", "mail", "telephoneNumber", "title", "departmentNumber"]
        );
    }
}

#[tokio::test(start_paused = true)]
async fn sessions_bind_as_different_accounts() {
    let all = scenarios();
    let (name, s) = all.iter().find(|(n, _)| n == "bind_search").unwrap();
    let mut dns = BTreeSet::new();
    for session_id in 1..=20 {
        let r = run(s, SEED, session_id).await;
        let (c, _) = check(name, &r);
        let Op::BindRequest { name: dn, .. } = &c[1].op else {
            unreachable!()
        };
        dns.insert(dn.clone());
    }
    assert!(dns.len() >= 8, "only {} accounts in 20 sessions", dns.len());
}

#[tokio::test(start_paused = true)]
async fn failed_bind_is_rejected_and_then_accepted() {
    let all = scenarios();
    let (name, s) = all.iter().find(|(n, _)| n == "failed_bind").unwrap();
    for session_id in 1..=8 {
        let r = run(s, SEED, session_id).await;
        let (c, srv) = check(name, &r);
        assert_eq!(
            names(&c),
            [
                "bind_request",
                "bind_request",
                "bind_request",
                "search_request",
                "unbind_request"
            ]
        );
        assert_eq!(result_codes(&srv)[..3], [0, 49, 0]);
        let (
            Op::BindRequest {
                name: a,
                auth: Auth::Simple(p1),
                ..
            },
            Op::BindRequest {
                name: b,
                auth: Auth::Simple(p2),
                ..
            },
        ) = (&c[1].op, &c[2].op)
        else {
            unreachable!()
        };
        assert_eq!(a, b, "the same account binds twice");
        assert_eq!(p1, b"Tst-not-the-password-00");
        assert_ne!(p1, p2);
        // The groups of the account: its department, and a member filter that
        // names the account.
        let Op::SearchRequest(q) = &c[3].op else {
            unreachable!()
        };
        assert_eq!(q.base, "ou=Groups,dc=example,dc=test");
        assert_eq!(q.filter.to_string(), format!("(member={a})"));
        let entries = srv
            .iter()
            .filter(|m| matches!(m.op, Op::SearchEntry(_)))
            .count();
        assert_eq!(entries, 1, "one department group per account");
    }
}

#[tokio::test(start_paused = true)]
async fn directory_browse_lists_the_people_without_logging_in() {
    let all = scenarios();
    let (name, s) = all.iter().find(|(n, _)| n == "directory_browse").unwrap();
    let r = run(s, SEED, 1).await;
    let (c, srv) = check(name, &r);
    assert_eq!(
        names(&c),
        [
            "bind_request",
            "search_request",
            "search_request",
            "unbind_request"
        ]
    );
    // An anonymous bind: empty name, empty password.
    assert_eq!(
        c[0].op,
        Op::BindRequest {
            version: 3,
            name: String::new(),
            auth: Auth::Simple(Vec::new())
        }
    );
    // 1 bind response, 16 entries, a done, 1 entry, a done.
    let kinds = names(&srv);
    assert_eq!(kinds.len(), 1 + 16 + 1 + 1 + 1);
    assert_eq!(kinds.iter().filter(|k| **k == "search_entry").count(), 17);
    assert_eq!(result_codes(&srv), [0, 0, 0]);
    // Each entry is a message of its own, and none shows a password.
    let wire = session(&r).s2c();
    assert!(!wire.windows(4).any(|w| w == b"Tst-"));
    for m in &srv {
        if let Op::SearchEntry(e) = &m.op {
            assert!(!e
                .attrs
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("userPassword")));
        }
    }
    // The detail search opens one person with more attributes than the list.
    let Op::SearchEntry(list_entry) = &srv[1].op else {
        unreachable!()
    };
    let Op::SearchEntry(detail) = &srv[18].op else {
        unreachable!()
    };
    assert_eq!(list_entry.attrs.len(), 3);
    assert_eq!(detail.attrs.len(), 6);
}

#[tokio::test(start_paused = true)]
async fn a_size_limit_cuts_the_listing_and_ends_with_size_limit_exceeded() {
    let all = scenarios_with(&[("size_limit", Value::Int(5)), ("done_code", Value::Int(4))]);
    let (name, s) = all.iter().find(|(n, _)| n == "directory_browse").unwrap();
    let r = run(s, SEED, 1).await;
    let (_, srv) = check(name, &r);
    let entries = srv
        .iter()
        .filter(|m| matches!(m.op, Op::SearchEntry(_)))
        .count();
    // 5 listed, 1 detail.
    assert_eq!(entries, 6);
    assert_eq!(result_codes(&srv), [0, 4, 0]);
    // Without the matching expectation the client fails the session.
    let wrong = scenarios_with(&[("size_limit", Value::Int(5))]);
    let (_, s) = wrong.iter().find(|(n, _)| n == "directory_browse").unwrap();
    let r = run(s, SEED, 1).await;
    assert!(
        r.result.is_err() || r.records.iter().any(|x| x.failed.is_some()),
        "a client that expects success must reject sizeLimitExceeded"
    );
}

#[tokio::test(start_paused = true)]
async fn rootdse_probe_reads_the_root_dse_and_the_naming_context() {
    let all = scenarios();
    let (name, s) = all.iter().find(|(n, _)| n == "rootdse_probe").unwrap();
    let r = run(s, SEED, 1).await;
    let (c, srv) = check(name, &r);
    assert_eq!(
        names(&c),
        [
            "bind_request",
            "search_request",
            "search_request",
            "unbind_request"
        ]
    );
    let Op::SearchRequest(q) = &c[1].op else {
        unreachable!()
    };
    assert_eq!((q.base.as_str(), q.scope), ("", 0));
    assert_eq!(q.filter.to_string(), "(objectClass=*)");
    let Op::SearchEntry(root) = &srv[1].op else {
        unreachable!()
    };
    assert_eq!(root.dn, "");
    let got: Vec<&str> = root.attrs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        got,
        [
            "namingContexts",
            "supportedLDAPVersion",
            "supportedSASLMechanisms",
            "subschemaSubentry",
            "vendorName"
        ]
    );
    let Op::SearchEntry(ctx) = &srv[3].op else {
        unreachable!()
    };
    assert_eq!(ctx.dn, "dc=example,dc=test");
    assert_eq!(ctx.attrs[0].0, "description");
    // The probe is short.
    let conn = session(&r);
    assert!(conn.c2s().len() < 400 && conn.s2c().len() < 900);
}

const SMALL: &str = "\
dn: dc=lab,dc=test
objectClass: domain
dc: lab

dn: ou=People,dc=lab,dc=test
objectClass: organizationalUnit
ou: People

dn: uid=solo,ou=People,dc=lab,dc=test
objectClass: inetOrgPerson
uid: solo
cn: Solo Tester
description: LONG
userPassword: Tst-solo-only-1
";

#[tokio::test(start_paused = true)]
async fn a_custom_directory_with_a_long_entry_answers_in_long_length_form() {
    let long = "x".repeat(900);
    let ldif = SMALL.replace("LONG", &long);
    let all = scenarios_with(&[
        ("directory", Value::Str(ldif)),
        ("search_base", Value::Str("dc=lab,dc=test".into())),
        ("user_attributes", Value::Str(String::new())),
    ]);
    let (name, s) = all.iter().find(|(n, _)| n == "bind_search").unwrap();
    let r = run(s, SEED, 1).await;
    let (c, srv) = check(name, &r);
    let Op::BindRequest {
        name: dn,
        auth: Auth::Simple(p),
        ..
    } = &c[1].op
    else {
        unreachable!()
    };
    assert_eq!(dn, "uid=solo,ou=People,dc=lab,dc=test");
    assert_eq!(p, b"Tst-solo-only-1");
    // No attribute list asks for every attribute, so the long value comes back.
    let Op::SearchEntry(e) = &srv[2].op else {
        unreachable!()
    };
    let desc = e.attrs.iter().find(|(n, _)| n == "description").unwrap();
    assert_eq!(desc.1[0].len(), 900);
    let wire = session(&r).s2c();
    let entry_at = wire
        .windows(3)
        .position(|w| w == [0x30, 0x82, 0x03])
        .expect("an entry in the long length form");
    assert!(entry_at > 0, "after the bind response");
}

#[tokio::test(start_paused = true)]
async fn a_directory_that_cannot_be_read_fails_the_session() {
    for bad in [
        "garbage without a colon",
        "dn: a=b\nfoo:: AAAA\n",
        "dn: a=b\ncn: x\n",
    ] {
        let all = scenarios_with(&[("directory", Value::Str(bad.into()))]);
        let (_, s) = all.iter().find(|(n, _)| n == "bind_search").unwrap();
        let r = run(s, SEED, 1).await;
        let failed = r.result.is_err()
            || r.records
                .iter()
                .any(|x| x.failed.is_some() && x.side == Side::Client);
        assert!(failed, "{bad:?} must fail the client");
    }
}

#[test]
fn the_parameter_schema_matches_the_variables_of_the_scenarios() {
    use serde_yaml::Value as Yaml;
    let text = std::fs::read_to_string(common::module_dir().join("manifest.yaml")).unwrap();
    let doc: Yaml = serde_yaml::from_str(&text).unwrap();
    let props = doc["parameters"]["properties"].as_mapping().unwrap();
    let mut used = BTreeSet::new();
    for (name, s) in scenarios() {
        for (var, value) in &s.vars {
            used.insert(var.clone());
            let p = props
                .get(var.as_str())
                .unwrap_or_else(|| panic!("{name}: {var} is not a parameter"));
            let default = &p["default"];
            let same = match value {
                Value::Int(i) => default.as_i64() == Some(*i),
                Value::Str(t) => default.as_str() == Some(t.as_str()),
                other => panic!("{other:?}"),
            };
            assert!(
                same,
                "{name}: the default of {var} differs: {default:?} and {value:?}"
            );
        }
    }
    let declared: BTreeSet<String> = props
        .keys()
        .map(|k| k.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        declared, used,
        "every parameter is a variable of a scenario"
    );
}
