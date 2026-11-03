// SPDX-License-Identifier: GPL-3.0-only
//! Golden test of the template grammar of contract v0.2.
//!
//! Each case names the grammar rule it covers. The golden file records the
//! compiled program dump, or the compile error, and the bytes that one session
//! renders with seed 42.

mod common;

use std::fmt::Write;

use fps_flow::Template;

/// Cases in the order of the grammar in `contracts/README.md`.
const CASES: &[(&str, &str)] = &[
    // template := { text | "{{" expression "}}" }
    ("text only", "GET / HTTP/1.1\r\n"),
    ("empty template", ""),
    (
        "text around placeholders",
        "a {{ vars.host }} b {{ session.id }} c",
    ),
    ("placeholder without spaces", "{{vars.size}}"),
    ("unbalanced open", "x {{ vars.host"),
    ("unbalanced close", "x }} y"),
    ("empty placeholder", "{{   }}"),
    // literal := integer | float | string | "true" | "false"
    ("integer literal", "{{ 42 }}"),
    ("negative integer literal", "{{ -7 }}"),
    ("integer out of range", "{{ 99999999999999999999 }}"),
    ("float literal", "{{ 2.50 }}"),
    ("negative float literal", "{{ -0.5 }}"),
    ("string literal", "{{ 'a {{ b' }}"),
    ("boolean literals", "{{ true }}/{{ false }}"),
    ("unterminated string", "{{ 'abc }}"),
    ("a float needs digits after the dot", "{{ 1. }}"),
    // reference := "last" | "vars" "." ident | "session" "." ident
    ("vars reference", "{{ vars.host }}"),
    (
        "vars of every type",
        "{{ vars.size }} {{ vars.ratio }} {{ vars.tls }}",
    ),
    ("undefined variable renders an error", "{{ vars.missing }}"),
    ("session.id", "{{ session.id }}"),
    ("session.flow", "{{ session.flow }}"),
    ("last", "[{{ last }}]"),
    ("unknown session field", "{{ session.secret }}"),
    ("last has no fields", "{{ last.size }}"),
    ("unknown root", "{{ other.x }}"),
    ("vars needs one name", "{{ vars.a.b }}"),
    // call := name "(" [ arguments ] ")" with core functions
    ("fill positional", "{{ fill(5) }}"),
    ("fill keyword", "{{ fill(n=5) }}"),
    ("fill from a variable", "{{ fill(vars.size) }}"),
    ("fill from a nested call", "{{ fill(rand_int(1, 3)) }}"),
    ("hex folds at compile time", "{{ hex('48 49') }}"),
    ("hex keyword", "{{ hex(s='0d0a') }}"),
    ("hex from a variable", "{{ hex(vars.digits) }}"),
    ("hex odd digits", "{{ hex('abc') }}"),
    ("hex bad digit", "{{ hex('zz') }}"),
    ("rand_int positional", "{{ rand_int(1, 6) }}"),
    ("rand_int mixed", "{{ rand_int(1, b=6) }}"),
    ("rand_int keywords", "{{ rand_int(b=6, a=1) }}"),
    (
        "rand_int empty range renders an error",
        "{{ rand_int(6, 1) }}",
    ),
    ("unknown core function", "{{ nope(1) }}"),
    ("too many positional", "{{ fill(1, 2) }}"),
    ("missing parameter", "{{ rand_int(1) }}"),
    ("unknown keyword", "{{ fill(m=3) }}"),
    ("keyword repeats a positional", "{{ rand_int(1, a=2) }}"),
    ("int parameter takes no text", "{{ fill('x') }}"),
    ("int parameter takes no float", "{{ fill(1.5) }}"),
    ("int parameter takes no bytes", "{{ fill(last) }}"),
    ("text parameter takes no int", "{{ hex(1) }}"),
    // name := ident | module "." ident
    ("module call without arguments", "{{ ssh.kexinit() }}"),
    ("module call positional", "{{ http1.request('GET', '/') }}"),
    (
        "module call keywords",
        "{{ dns.query(name=vars.host, qtype='A') }}",
    ),
    (
        "module call with every value type",
        "{{ m.f(1, 2.5, 'x', true, vars.ratio, session.id, session.flow, last) }}",
    ),
    (
        "nested core call in a module call",
        "{{ http1.response(200, body=fill(vars.size)) }}",
    ),
    (
        "nested module call",
        "{{ dns.response(dns.query(name='a'), address='192.0.2.1') }}",
    ),
    (
        "module call order draws the generator in call order",
        "{{ m.a() }}{{ m.b(rand_int(0, 9)) }}{{ m.c() }}",
    ),
    ("module error", "{{ m.fail() }}"),
    ("too many name parts", "{{ a.b.c(1) }}"),
    // arguments := positional { "," positional } { "," keyword } | keyword { "," keyword }
    ("positional after keyword", "{{ m.f(k=1, 2) }}"),
    ("trailing comma", "{{ fill(1,) }}"),
    ("repeated keyword", "{{ m.f(k=1, k=2) }}"),
    ("missing close paren", "{{ fill(1 }}"),
    ("operator", "{{ 1 + 2 }}"),
    // reserved module names and nesting depth
    ("reserved module vars", "{{ vars.f() }}"),
    ("reserved module session", "{{ session.f() }}"),
    ("reserved module last", "{{ last.f() }}"),
    ("reserved module true", "{{ true.f() }}"),
    ("reserved module false", "{{ false.f() }}"),
    (
        "eight levels",
        "{{ m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=1)))))))) }}",
    ),
    (
        "nine levels",
        "{{ m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=m.f(x=1))))))))) }}",
    ),
];

#[test]
fn every_grammar_rule_matches_its_golden_dump() {
    let vars = common::vars();
    let mut out = String::new();
    for (rule, src) in CASES {
        let _ = writeln!(out, "== {rule}");
        let _ = writeln!(out, "source \"{}\"", src.as_bytes().escape_ascii());
        match Template::compile(src) {
            Err(e) => {
                let _ = writeln!(out, "error: {e}");
            }
            Ok(t) => {
                out.push_str(&t.dump());
                match common::render(&t, &vars, 42, 7, b"\x01\x02") {
                    Ok(bytes) => {
                        let _ = writeln!(out, "render \"{}\"", bytes.escape_ascii());
                    }
                    Err(e) => {
                        let _ = writeln!(out, "render error: {e}");
                    }
                }
            }
        }
    }
    fps_golden::assert_golden!("grammar.seed42.txt", out);
}
