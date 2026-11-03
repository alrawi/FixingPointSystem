// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests of the template compiler and renderer.

use super::*;
use crate::function::{FnError, NoFunctions};

/// Echoes each call as text and draws one number from the session generator.
struct Echo;

impl TemplateFn for Echo {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.function == "fail" {
            return Err(FnError::new(FnErrorKind::Protocol, "short message"));
        }
        let mut args: Vec<String> = call
            .args
            .positional
            .iter()
            .map(|v| format!("{v:?}"))
            .collect();
        args.extend(
            call.args
                .named
                .iter()
                .map(|n| format!("{}={:?}", n.name, n.value)),
        );
        out.extend_from_slice(
            format!(
                "<{}.{}({}) r{}>",
                call.module,
                call.function,
                args.join(", "),
                rng.next_u32() % 100
            )
            .as_bytes(),
        );
        Ok(())
    }
}

fn render_with(t: &Template, vars: &Vars, overlay: &Vars) -> Result<Vec<u8>, TemplateError> {
    let filler = Filler::new(1);
    let mut rng = Pcg64Mcg::new(9);
    let mut echo = Echo;
    let mut ctx = RenderCtx {
        vars,
        overlay,
        session_id: 42,
        flow: "page",
        last: b"\x00\x01ok",
        rng: &mut rng,
        filler: &filler,
        functions: &mut echo,
    };
    t.render_to_vec(&mut ctx)
}

fn render(t: &str, vars: &Vars) -> Result<Vec<u8>, TemplateError> {
    render_with(&Template::compile(t)?, vars, &Vars::new())
}

fn vars() -> Vars {
    let mut v = Vars::new();
    v.insert("host".into(), Value::Str("example.test".into()));
    v.insert("n".into(), Value::Int(5));
    v.insert("ratio".into(), Value::Float(0.5));
    v.insert("hexstr".into(), Value::Str("0a0b".into()));
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
fn session_fields_and_last() {
    assert_eq!(
        render("{{session.id}}-{{ session.flow }}-{{ last }}", &vars()).unwrap(),
        b"42-page-\x00\x01ok"
    );
}

#[test]
fn literals_fold_into_text() {
    let t = Template::compile("a{{ 7 }}{{ -2.5 }}{{ true }}{{ 'x' }}{{ hex('41') }}b").unwrap();
    assert!(t.is_static());
    assert_eq!(t.as_static(), Some(&b"a7-2.5truexAb"[..]));
}

#[test]
fn fill_length_and_printable() {
    let out = render(
        "{{ fill(vars.n) }}|{{ fill(n=3000000) }}|{{ fill(rand_int(2, 2)) }}",
        &vars(),
    )
    .unwrap();
    assert_eq!(out.len(), 5 + 1 + 3_000_000 + 1 + 2);
    assert!(out.iter().all(u8::is_ascii));
}

#[test]
fn hex_decodes_at_compile_time() {
    let t = Template::compile("{{ hex('00ff10') }}").unwrap();
    assert!(t.is_static());
    assert_eq!(
        render("{{ hex(s='00ff10') }}", &vars()).unwrap(),
        vec![0, 255, 16]
    );
    let t = Template::compile("{{ hex(vars.hexstr) }}").unwrap();
    assert!(!t.is_static());
    assert_eq!(
        render_with(&t, &vars(), &Vars::new()).unwrap(),
        vec![10, 11]
    );
}

#[test]
fn rand_int_in_range() {
    for _ in 0..50 {
        let out = render("{{ rand_int(3, b=4) }}", &vars()).unwrap();
        assert!(out == b"3" || out == b"4");
    }
}

#[test]
fn module_calls_pass_evaluated_arguments_in_call_order() {
    let out = render(
        "{{ http1.response(200, body=fill(2), ratio=vars.ratio, ok=true, raw=last) }}",
        &vars(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.starts_with("<http1.response(Int(200), body=Text(\""),
        "{text}"
    );
    assert!(
        text.contains("ratio=Float(0.5), ok=Boolean(true), raw=Bytes([0, 1, 111, 107])) r"),
        "{text}"
    );
}

#[test]
fn nested_module_calls_pass_bytes() {
    let out = render("{{ dns.wrap(dns.query(name=vars.host)) }}", &vars()).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("<dns.wrap(Bytes(["), "{text}");
}

#[test]
fn module_errors_name_the_call() {
    assert_eq!(
        render("{{ dns.fail() }}", &vars()),
        Err(TemplateError::Function {
            module: "dns".into(),
            function: "fail".into(),
            kind: FnErrorKind::Protocol,
            message: "short message".into(),
        })
    );
    let t = Template::compile("{{ dns.query() }}").unwrap();
    let filler = Filler::new(1);
    let mut rng = Pcg64Mcg::new(1);
    let (v, o) = (vars(), Vars::new());
    let mut none = NoFunctions;
    let mut ctx = RenderCtx {
        vars: &v,
        overlay: &o,
        session_id: 0,
        flow: "f",
        last: &[],
        rng: &mut rng,
        filler: &filler,
        functions: &mut none,
    };
    assert!(matches!(
        t.render_to_vec(&mut ctx),
        Err(TemplateError::Function {
            kind: FnErrorKind::Unsupported,
            ..
        })
    ));
}

#[test]
fn compile_errors() {
    let cases: &[(&str, TemplateError)] = &[
        (
            "{{ nope(1) }}",
            TemplateError::UnknownFunction("nope".into()),
        ),
        (
            "{{ fill(1, 2) }}",
            TemplateError::Arity {
                name: "fill".into(),
                max: 1,
                got: 2,
            },
        ),
        ("{{ vars.x ", TemplateError::Unbalanced("{{")),
        ("x }}", TemplateError::Unbalanced("}}")),
        ("{{ }}", TemplateError::EmptyPlaceholder),
        (
            "{{ fill(m=3) }}",
            TemplateError::UnknownParameter {
                function: "fill".into(),
                param: "m".into(),
            },
        ),
        (
            "{{ rand_int(1, a=2) }}",
            TemplateError::DuplicateParameter {
                function: "rand_int".into(),
                param: "a".into(),
            },
        ),
        (
            "{{ m.f(k=1, k=2) }}",
            TemplateError::DuplicateParameter {
                function: "m.f".into(),
                param: "k".into(),
            },
        ),
        (
            "{{ rand_int(1) }}",
            TemplateError::MissingParameter {
                function: "rand_int".into(),
                param: "b".into(),
            },
        ),
        (
            "{{ fill('x') }}",
            TemplateError::ArgumentType {
                function: "fill".into(),
                param: "n".into(),
                expected: ValueType::Int,
                got: ValueType::Text,
            },
        ),
        (
            "{{ fill(last) }}",
            TemplateError::ArgumentType {
                function: "fill".into(),
                param: "n".into(),
                expected: ValueType::Int,
                got: ValueType::Bytes,
            },
        ),
        (
            "{{ hex(1) }}",
            TemplateError::ArgumentType {
                function: "hex".into(),
                param: "s".into(),
                expected: ValueType::Text,
                got: ValueType::Int,
            },
        ),
        (
            "{{ vars.f(1) }}",
            TemplateError::ReservedModule("vars".into()),
        ),
        (
            "{{ session.secret }}",
            TemplateError::UnknownReference("session.secret".into()),
        ),
        (
            "{{ last.size }}",
            TemplateError::UnknownReference("last.size".into()),
        ),
    ];
    for (src, want) in cases {
        assert_eq!(&Template::compile(src).unwrap_err(), want, "{src}");
    }
    for src in [
        "{{ 1 + 2 }}",
        "{{ fill(1,) }}",
        "{{ fill(n=1, 2) }}",
        "{{ hex('ab) }}",
        "{{ a.b.c(1) }}",
        "{{ f(1)(2) }}",
    ] {
        assert!(
            matches!(Template::compile(src), Err(TemplateError::Syntax { .. })),
            "{src}"
        );
    }
    assert!(matches!(
        Template::compile("{{ hex('abc') }}"),
        Err(TemplateError::Hex(_))
    ));
}

#[test]
fn call_depth_limit() {
    let mut inner = "1".to_string();
    for _ in 0..MAX_CALL_DEPTH {
        inner = format!("m.f(x={inner})");
    }
    assert!(Template::compile(&format!("{{{{ {inner} }}}}")).is_ok());
    assert_eq!(
        Template::compile(&format!("{{{{ fill(n={inner}) }}}}")),
        Err(TemplateError::TooDeep { max: 8 })
    );
}

#[test]
fn render_errors() {
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
    assert_eq!(
        render("{{ fill(vars.host) }}", &vars()),
        Err(TemplateError::NotInteger("vars.host".into()))
    );
    assert_eq!(
        render("{{ hex(vars.n) }}", &vars()),
        Err(TemplateError::ArgumentType {
            function: "hex".into(),
            param: "s".into(),
            expected: ValueType::Text,
            got: ValueType::Int,
        })
    );
}

#[test]
fn render_int() {
    let v = vars();
    let o = Vars::new();
    let n = Template::compile("{{ vars.n }}").unwrap();
    assert_eq!(n.render_int_for_test(&v, &o), Ok(5));
    let host = Template::compile("{{ vars.host }}").unwrap();
    assert!(host.render_int_for_test(&v, &o).is_err());
}

#[test]
fn overlay_shadows_scenario_vars() {
    let mut o = Vars::new();
    o.insert("n".into(), Value::Int(7));
    let t = Template::compile("{{ vars.n }}").unwrap();
    assert_eq!(t.render_int_for_test(&vars(), &o), Ok(7));
}

#[test]
fn fold_vars_substitutes_constants_only() {
    let t = Template::compile(
        "Host: {{ vars.host }} {{ hex(vars.hexstr) }} {{ fill(vars.n) }} {{ m.f(vars.n, x=vars.other) }}",
    )
    .unwrap();
    let mut constants = vars();
    constants.remove("hexstr");
    let folded = t.fold_vars(&constants).unwrap();
    assert_eq!(folded.variables(), vec!["hexstr", "other"]);
    let mut all = vars();
    all.insert("other".into(), Value::Int(1));
    assert_eq!(
        render_with(&t, &all, &Vars::new()),
        render_with(&folded, &all, &Vars::new())
    );
    let mut constants = vars();
    constants.insert("hexstr".into(), Value::Str("zz".into()));
    assert!(matches!(
        t.fold_vars(&constants),
        Err(TemplateError::Hex(_))
    ));
}

#[test]
fn reports_calls_and_last() {
    let t = Template::compile("{{ http1.response(200, body=dns.answer_ip(last)) }}").unwrap();
    assert!(t.uses_last());
    assert_eq!(
        t.module_calls(),
        vec![
            CallSite {
                module: "http1",
                function: "response",
                positional: 1,
                keywords: vec!["body"],
            },
            CallSite {
                module: "dns",
                function: "answer_ip",
                positional: 1,
                keywords: vec![],
            },
        ]
    );
    assert!(!Template::compile("{{ vars.x }}").unwrap().uses_last());
}

impl Template {
    fn render_int_for_test(&self, vars: &Vars, overlay: &Vars) -> Result<i64, TemplateError> {
        let filler = Filler::new(1);
        let mut rng = Pcg64Mcg::new(1);
        let mut none = NoFunctions;
        let mut ctx = RenderCtx {
            vars,
            overlay,
            session_id: 0,
            flow: "f",
            last: &[],
            rng: &mut rng,
            filler: &filler,
            functions: &mut none,
        };
        self.render_int(&mut ctx)
    }
}
