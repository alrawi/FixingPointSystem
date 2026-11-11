// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests of the template functions, called the way the interpreter calls
//! them.

use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs, FnCall, FnErrorKind, TemplateFn};
use rand_pcg::Pcg64Mcg;

use super::*;

const LDIF: &str = "\
dn:
objectClass: top
namingContexts: dc=example,dc=test

dn: dc=example,dc=test
objectClass: domain
dc: example

dn: ou=People,dc=example,dc=test
objectClass: organizationalUnit
ou: People

dn: uid=jdoe,ou=People,dc=example,dc=test
objectClass: inetOrgPerson
uid: jdoe
cn: John Doe
mail: jdoe@example.test
userPassword: Tst-alpha-1

dn: uid=asmith,ou=People,dc=example,dc=test
objectClass: inetOrgPerson
uid: asmith
cn: Anna Smith
userPassword: Tst-beta-2
";

const JDOE: &str = "uid=jdoe,ou=People,dc=example,dc=test";

fn rng() -> Pcg64Mcg {
    use rand::SeedableRng;
    Pcg64Mcg::seed_from_u64(7)
}

fn call(l: &mut Ldap, name: &str, pos: &[ArgValue<'_>]) -> Result<Vec<u8>, fps_flow::FnError> {
    call_named(l, name, pos, &[])
}

fn call_named(
    l: &mut Ldap,
    name: &str,
    pos: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Result<Vec<u8>, fps_flow::FnError> {
    let mut out = Vec::new();
    l.render(
        &FnCall {
            module: "ldap",
            function: name,
            args: CallArgs {
                positional: pos,
                named,
            },
        },
        &mut rng(),
        &mut out,
    )?;
    Ok(out)
}

fn text(v: &[u8]) -> String {
    String::from_utf8(v.to_vec()).unwrap()
}

/// Hands `wire` to `l` the way two `expect` actions and a `set` do.
fn receive(l: &mut Ldap, wire: &[u8], kind: &str) -> Result<i64, fps_flow::FnError> {
    receive_result(l, wire, kind, 0)
}

fn receive_result(
    l: &mut Ldap,
    wire: &[u8],
    kind: &str,
    result: i64,
) -> Result<i64, fps_flow::FnError> {
    let rest = text(&call(l, "size", &[ArgValue::Bytes(&wire[..4])])?)
        .parse::<usize>()
        .unwrap();
    assert_eq!(rest, wire.len() - 4, "size of the message");
    let id = call(
        l,
        "recv",
        &[
            ArgValue::Bytes(&wire[4..]),
            ArgValue::Text(kind),
            ArgValue::Int(result),
        ],
    )?;
    Ok(text(&id).parse().unwrap())
}

fn field(l: &mut Ldap, name: &str) -> String {
    text(&call(l, "field", &[ArgValue::Text(name)]).unwrap())
}

#[test]
fn the_head_gives_the_rest_in_all_three_length_forms() {
    let mut l = Ldap::default();
    for (head, want) in [
        ([0x30, 0x05, 0x02, 0x01], 3),
        ([0x30, 0x7f, 0x02, 0x01], 125),
        ([0x30, 0x81, 0x80, 0x02], 127),
        ([0x30, 0x81, 0xff, 0x02], 254),
        ([0x30, 0x82, 0x01, 0x00], 256),
        ([0x30, 0x82, 0xff, 0xff], 65_535),
    ] {
        let got = call(&mut l, "size", &[ArgValue::Bytes(&head)]).unwrap();
        assert_eq!(text(&got), want.to_string(), "{head:02x?}");
    }
    for (head, kind) in [
        (&[0x30, 0x05, 0x02][..], FnErrorKind::InvalidArgument),
        (&[0x31, 0x05, 0x02, 0x01][..], FnErrorKind::Protocol),
        (&[0x30, 0x80, 0x02, 0x01][..], FnErrorKind::Protocol),
        (&[0x30, 0x83, 0x01, 0x00][..], FnErrorKind::Protocol),
        (&[0x30, 0x04, 0x02, 0x01][..], FnErrorKind::Protocol),
        (&[0x30, 0x81, 0x04, 0x02][..], FnErrorKind::Protocol),
    ] {
        let e = call(&mut l, "size", &[ArgValue::Bytes(head)]).unwrap_err();
        assert_eq!(e.kind, kind, "{head:02x?}: {e}");
    }
}

#[test]
fn a_bind_request_is_read_back_field_by_field() {
    let mut c = Ldap::default();
    let wire = call(
        &mut c,
        "bind_request",
        &[
            ArgValue::Int(1),
            ArgValue::Text(JDOE),
            ArgValue::Text("Tst-alpha-1"),
        ],
    )
    .unwrap();
    assert_eq!(wire[0], 0x30);
    let mut s = Ldap::default();
    assert_eq!(receive(&mut s, &wire, "bind_request").unwrap(), 1);
    assert_eq!(field(&mut s, "name"), JDOE);
    assert_eq!(field(&mut s, "password"), "Tst-alpha-1");
    assert_eq!(field(&mut s, "version"), "3");
    assert_eq!(field(&mut s, "auth"), "simple");
    assert_eq!(field(&mut s, "operation"), "bind_request");
    assert_eq!(field(&mut s, "message_id"), "1");
    let e = call(&mut s, "field", &[ArgValue::Text("filter")]).unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol);
    assert!(e.message.contains("bind_request"), "{e}");
}

#[test]
fn the_server_answers_binds_from_the_directory() {
    let d = ArgValue::Text(LDIF);
    for (name, pw, want, text_part) in [
        (JDOE, "Tst-alpha-1", 0, ""),
        (JDOE, "wrong", 49, ""),
        ("", "", 0, ""),
        (JDOE, "", 53, "unauthenticated"),
    ] {
        let mut c = Ldap::default();
        let req = call(
            &mut c,
            "bind_request",
            &[ArgValue::Int(4), ArgValue::Text(name), ArgValue::Text(pw)],
        )
        .unwrap();
        let mut s = Ldap::default();
        receive(&mut s, &req, "bind_request").unwrap();
        let resp = call(&mut s, "bind_answer", &[d]).unwrap();
        assert_eq!(
            receive_result(&mut c, &resp, "bind_response", want).unwrap(),
            4
        );
        assert_eq!(field(&mut c, "result_code"), want.to_string());
        assert!(field(&mut c, "diagnostic").contains(text_part));
    }
}

#[test]
fn a_response_with_another_result_code_fails_the_receiver() {
    let mut c = Ldap::default();
    let resp = call(&mut c, "bind_response", &[ArgValue::Int(1)]).unwrap();
    let e = receive_result(&mut c, &resp, "bind_response", 49).unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol);
    assert!(e.message.contains("result code 0"), "{e}");
    // The right kind with the right code passes, a wrong kind does not.
    receive_result(&mut c, &resp, "bind_response", 0).unwrap();
    let e = receive(&mut c, &resp, "search_done").unwrap_err();
    assert!(
        e.message
            .contains("expected search_done, got bind_response"),
        "{e}"
    );
}

#[test]
fn recv_needs_a_size_call_and_a_known_kind_and_a_sound_message() {
    let mut l = Ldap::default();
    let e = call(
        &mut l,
        "recv",
        &[ArgValue::Bytes(&[1, 2, 3]), ArgValue::Text("bind_request")],
    )
    .unwrap_err();
    assert!(e.message.contains("without a size call"), "{e}");
    let e = call(
        &mut l,
        "recv",
        &[ArgValue::Bytes(&[1]), ArgValue::Text("search")],
    )
    .unwrap_err();
    assert_eq!(e.kind, FnErrorKind::InvalidArgument);
    // A head that promises 6 octets and a message that carries other bytes.
    call(
        &mut l,
        "size",
        &[ArgValue::Bytes(&[0x30, 0x07, 0x02, 0x01])],
    )
    .unwrap();
    let e = call(
        &mut l,
        "recv",
        &[
            ArgValue::Bytes(&[0x03, 0x42, 0x01, 0x00, 0x00]),
            ArgValue::Text("unbind_request"),
        ],
    )
    .unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol, "{e}");
    // The head is used up, so a second recv fails.
    let e = call(
        &mut l,
        "recv",
        &[
            ArgValue::Bytes(&[0x03, 0x42, 0x00]),
            ArgValue::Text("unbind_request"),
        ],
    )
    .unwrap_err();
    assert!(e.message.contains("without a size call"), "{e}");
    // Fields and answers need a kept message.
    for f in ["field", "bind_answer", "matches"] {
        let pos: Vec<ArgValue<'_>> = if f == "field" {
            vec![ArgValue::Text("operation")]
        } else {
            vec![ArgValue::Text(LDIF)]
        };
        let e = call(&mut l, f, &pos).unwrap_err();
        assert_eq!(e.kind, FnErrorKind::Protocol, "{f}");
    }
}

fn search_wire(filter: &str, base: &str, scope: &str, size_limit: i64) -> Vec<u8> {
    let mut c = Ldap::default();
    call_named(
        &mut c,
        "search_request",
        &[ArgValue::Int(2), ArgValue::Text(base)],
        &[
            NamedArg {
                name: "scope",
                value: ArgValue::Text(scope),
            },
            NamedArg {
                name: "filter",
                value: ArgValue::Text(filter),
            },
            NamedArg {
                name: "attributes",
                value: ArgValue::Text("cn, mail ,"),
            },
            NamedArg {
                name: "size_limit",
                value: ArgValue::Int(size_limit),
            },
        ],
    )
    .unwrap()
}

#[test]
fn the_search_request_reads_back_and_the_server_streams_the_entries() {
    let wire = search_wire(
        "(&(objectClass=inetOrgPerson)(uid=*))",
        "dc=example,dc=test",
        "sub",
        0,
    );
    let mut s = Ldap::default();
    assert_eq!(receive(&mut s, &wire, "search_request").unwrap(), 2);
    assert_eq!(field(&mut s, "base"), "dc=example,dc=test");
    assert_eq!(field(&mut s, "scope"), "sub");
    assert_eq!(field(&mut s, "deref"), "never");
    assert_eq!(
        field(&mut s, "filter"),
        "(&(objectClass=inetOrgPerson)(uid=*))"
    );
    assert_eq!(field(&mut s, "attributes"), "cn,mail");
    assert_eq!(field(&mut s, "types_only"), "false");
    assert_eq!(field(&mut s, "size_limit"), "0");
    let n: usize = text(&call(&mut s, "matches", &[ArgValue::Text(LDIF)]).unwrap())
        .parse()
        .unwrap();
    assert_eq!(n, 2);
    // The client counts the same.
    let count = call(
        &mut s,
        "count",
        &[ArgValue::Text(LDIF), ArgValue::Text("dc=example,dc=test")],
    );
    assert_eq!(text(&count.unwrap()), "4");
    let mut got = Vec::new();
    let mut client = Ldap::default();
    for _ in 0..n {
        let entry = call(&mut s, "search_next", &[]).unwrap();
        assert_eq!(receive(&mut client, &entry, "search_entry").unwrap(), 2);
        got.push(field(&mut client, "dn"));
        // The attribute selection reached the entry, and the password did not.
        let attrs = field(&mut client, "attributes");
        assert!(attrs.starts_with("cn: "), "{attrs}");
        assert!(!attrs.contains("Tst-"), "{attrs}");
    }
    assert_eq!(got, [JDOE, "uid=asmith,ou=People,dc=example,dc=test"]);
    let e = call(&mut s, "search_next", &[]).unwrap_err();
    assert!(e.message.contains("after the last entry"), "{e}");
    let done = call(&mut s, "search_end", &[]).unwrap();
    assert_eq!(receive(&mut client, &done, "search_done").unwrap(), 2);
    let e = call(&mut s, "search_end", &[]).unwrap_err();
    assert!(e.message.contains("before matches"), "{e}");
}

#[test]
fn count_equals_matches_for_every_scope_and_limit() {
    for (filter, base, scope, limit) in [
        ("(uid=jdoe)", "dc=example,dc=test", "sub", 0),
        ("(objectClass=*)", "ou=People,dc=example,dc=test", "one", 0),
        ("(objectClass=*)", "ou=People,dc=example,dc=test", "base", 0),
        ("(objectClass=*)", "dc=example,dc=test", "sub", 2),
        ("(uid=nobody)", "dc=example,dc=test", "sub", 0),
        ("(uid=*)", "dc=missing,dc=test", "sub", 0),
        ("(namingContexts=*)", "", "base", 0),
    ] {
        let wire = search_wire(filter, base, scope, limit);
        let mut s = Ldap::default();
        receive(&mut s, &wire, "search_request").unwrap();
        let m = call(&mut s, "matches", &[ArgValue::Text(LDIF)]).unwrap();
        let c = call_named(
            &mut s,
            "count",
            &[ArgValue::Text(LDIF), ArgValue::Text(base)],
            &[
                NamedArg {
                    name: "scope",
                    value: ArgValue::Text(scope),
                },
                NamedArg {
                    name: "filter",
                    value: ArgValue::Text(filter),
                },
                NamedArg {
                    name: "size_limit",
                    value: ArgValue::Int(limit),
                },
            ],
        )
        .unwrap();
        assert_eq!(text(&m), text(&c), "{filter} {base} {scope} {limit}");
    }
}

#[test]
fn search_end_reports_size_limit_and_missing_base() {
    for (filter, base, limit, code) in [
        ("(objectClass=*)", "dc=example,dc=test", 2, "4"),
        ("(objectClass=*)", "dc=missing,dc=test", 0, "32"),
        ("(uid=nobody)", "dc=example,dc=test", 0, "0"),
    ] {
        let wire = search_wire(filter, base, "sub", limit);
        let mut s = Ldap::default();
        receive(&mut s, &wire, "search_request").unwrap();
        let n: usize = text(&call(&mut s, "matches", &[ArgValue::Text(LDIF)]).unwrap())
            .parse()
            .unwrap();
        // Ending early is a protocol error of the scenario.
        if n > 0 {
            let e = call(&mut s, "search_end", &[]).unwrap_err();
            assert!(e.message.contains("not sent"), "{e}");
            // The failed call kept the search, so the scenario can go on.
        }
        for _ in 0..n {
            call(&mut s, "search_next", &[]).unwrap();
        }
        let done = call(&mut s, "search_end", &[]).unwrap();
        let mut c = Ldap::default();
        let want: i64 = code.parse().unwrap();
        receive_result(&mut c, &done, "search_done", want).unwrap();
    }
}

#[test]
fn search_done_with_a_matched_dn_for_a_missing_base() {
    let wire = search_wire(
        "(uid=*)",
        "uid=ghost,ou=People,dc=example,dc=test",
        "sub",
        0,
    );
    let mut s = Ldap::default();
    receive(&mut s, &wire, "search_request").unwrap();
    call(&mut s, "matches", &[ArgValue::Text(LDIF)]).unwrap();
    let done = call(&mut s, "search_end", &[]).unwrap();
    let mut c = Ldap::default();
    receive_result(&mut c, &done, "search_done", 32).unwrap();
    assert_eq!(field(&mut c, "matched_dn"), "ou=People,dc=example,dc=test");
}

#[test]
fn the_writers_check_their_arguments() {
    let mut l = Ldap::default();
    let bad: Vec<(&str, Vec<ArgValue<'_>>)> = vec![
        (
            "bind_request",
            vec![ArgValue::Int(-1), ArgValue::Text("a"), ArgValue::Text("b")],
        ),
        (
            "bind_request",
            vec![
                ArgValue::Int(2_147_483_648),
                ArgValue::Text("a"),
                ArgValue::Text("b"),
            ],
        ),
        ("unbind_request", vec![ArgValue::Int(-5)]),
        (
            "search_request",
            vec![
                ArgValue::Int(1),
                ArgValue::Text("dc=x"),
                ArgValue::Text("deep"),
            ],
        ),
        (
            "search_request",
            vec![
                ArgValue::Int(1),
                ArgValue::Text("dc=x"),
                ArgValue::Text("sub"),
                ArgValue::Text("(cn=a"),
            ],
        ),
        ("bind_response", vec![ArgValue::Int(1), ArgValue::Int(-1)]),
        ("search_done", vec![ArgValue::Int(1), ArgValue::Int(5000)]),
        (
            "search_entry",
            vec![
                ArgValue::Int(1),
                ArgValue::Text("a=b"),
                ArgValue::Text("no colon"),
            ],
        ),
        (
            "filter_eq",
            vec![ArgValue::Text("bad name"), ArgValue::Text("x")],
        ),
        (
            "account",
            vec![ArgValue::Text("dn: a=b\ncn: x\n"), ArgValue::Text("dn")],
        ),
        (
            "account",
            vec![
                ArgValue::Text(LDIF),
                ArgValue::Text("missing"),
                ArgValue::Int(0),
            ],
        ),
        (
            "count",
            vec![ArgValue::Text("garbage"), ArgValue::Text("dc=x")],
        ),
    ];
    for (f, pos) in bad {
        let e = call(&mut l, f, &pos).unwrap_err();
        assert_eq!(e.kind, FnErrorKind::InvalidArgument, "{f}: {e}");
    }
    let e = call(&mut l, "nope", &[]).unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Unsupported);
    let mut out = Vec::new();
    let e = l
        .render(
            &FnCall {
                module: "other",
                function: "size",
                args: CallArgs {
                    positional: &[],
                    named: &[],
                },
            },
            &mut rng(),
            &mut out,
        )
        .unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Unsupported);
}

#[test]
fn filter_eq_escapes_the_value_and_account_reads_the_directory() {
    let mut l = Ldap::default();
    let f = call(
        &mut l,
        "filter_eq",
        &[ArgValue::Text("cn"), ArgValue::Text("a*(b)\\")],
    )
    .unwrap();
    assert_eq!(text(&f), "(cn=a\\2a\\28b\\29\\5c)");
    let d = ArgValue::Text(LDIF);
    let at = |l: &mut Ldap, field: &str, seed: i64| {
        text(
            &call(
                l,
                "account",
                &[d, ArgValue::Text(field), ArgValue::Int(seed)],
            )
            .unwrap(),
        )
    };
    assert_eq!(at(&mut l, "dn", 0), JDOE);
    assert_eq!(at(&mut l, "uid", 1), "asmith");
    assert_eq!(at(&mut l, "userPassword", 2), "Tst-alpha-1");
    // A negative seed draws from the session generator, which is seeded.
    let a = at(&mut l, "uid", -1);
    assert!(a == "jdoe" || a == "asmith");
    assert_eq!(a, at(&mut l, "uid", -1));
}

#[test]
fn the_unbind_request_and_a_sasl_bind_travel_and_are_refused() {
    let mut c = Ldap::default();
    let wire = call(&mut c, "unbind_request", &[ArgValue::Int(9)]).unwrap();
    assert_eq!(wire, [0x30, 0x05, 0x02, 0x01, 0x09, 0x42, 0x00]);
    let mut s = Ldap::default();
    assert_eq!(receive(&mut s, &wire, "unbind_request").unwrap(), 9);
    // A SASL bind has no writer, so the test builds its bytes.
    let sasl = Message {
        id: 1,
        op: Op::BindRequest {
            version: 3,
            name: String::new(),
            auth: Auth::Sasl("GSSAPI".into()),
        },
    }
    .encode()
    .unwrap();
    receive(&mut s, &sasl, "bind_request").unwrap();
    assert_eq!(field(&mut s, "auth"), "sasl");
    assert!(call(&mut s, "field", &[ArgValue::Text("password")]).is_err());
    let resp = call(&mut s, "bind_answer", &[ArgValue::Text(LDIF)]).unwrap();
    receive_result(&mut c, &resp, "bind_response", 7).unwrap();
}

#[test]
fn the_search_entry_writer_builds_multi_valued_attributes() {
    let mut l = Ldap::default();
    let wire = call(
        &mut l,
        "search_entry",
        &[
            ArgValue::Int(3),
            ArgValue::Text(JDOE),
            ArgValue::Text("cn: John Doe\nmail: a@x.test\nmail: b@x.test"),
        ],
    )
    .unwrap();
    let mut c = Ldap::default();
    receive(&mut c, &wire, "search_entry").unwrap();
    assert_eq!(
        field(&mut c, "attributes"),
        "cn: John Doe\nmail: a@x.test\nmail: b@x.test"
    );
}

#[test]
fn an_empty_directory_argument_selects_the_directory_of_the_module() {
    let d = Directory::parse(DEFAULT_DIRECTORY).unwrap();
    assert_eq!(d.entries().len(), 25);
    let mut l = Ldap::default();
    let empty = ArgValue::Text("");
    let dn = call(
        &mut l,
        "account",
        &[empty, ArgValue::Text("dn"), ArgValue::Int(0)],
    )
    .unwrap();
    assert_eq!(text(&dn), "uid=amarlowe,ou=People,dc=example,dc=test");
    // Sixteen accounts, so the seeds wrap around after 16.
    let again = call(
        &mut l,
        "account",
        &[empty, ArgValue::Text("dn"), ArgValue::Int(16)],
    )
    .unwrap();
    assert_eq!(dn, again);
    let n = call(
        &mut l,
        "count",
        &[empty, ArgValue::Text("ou=People,dc=example,dc=test")],
    );
    assert_eq!(text(&n.unwrap()), "17");
    let n = call(
        &mut l,
        "count",
        &[
            empty,
            ArgValue::Text(""),
            ArgValue::Text("base"),
            ArgValue::Text("(objectClass=*)"),
        ],
    );
    assert_eq!(text(&n.unwrap()), "1");
}
