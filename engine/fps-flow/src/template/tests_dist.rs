// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests of the `dist` functions and the `profiles` references.

use super::tests::render_with;
use super::*;
use crate::function::NoFunctions;

/// Renders `src` `n` times with one generator that `seed` fixes and returns
/// the integers.
fn draws_with(src: &str, seed: u64, n: usize, vars: &Vars) -> Result<Vec<i64>, TemplateError> {
    let t = Template::compile(src)?;
    let filler = Filler::new(1);
    let overlay = Vars::new();
    let mut rng = Pcg64Mcg::new(u128::from(seed));
    let mut none = NoFunctions;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let mut ctx = RenderCtx {
            vars,
            overlay: &overlay,
            session_id: 1,
            flow: "f",
            last: &[],
            rng: &mut rng,
            filler: &filler,
            functions: &mut none,
        };
        let text = t.render_to_vec(&mut ctx)?;
        let text = String::from_utf8(text).unwrap();
        out.push(text.parse().unwrap());
    }
    Ok(out)
}

fn draws(src: &str, seed: u64, n: usize) -> Vec<i64> {
    draws_with(src, seed, n, &Vars::new()).unwrap()
}

fn mean(xs: &[i64]) -> f64 {
    xs.iter().map(|x| *x as f64).sum::<f64>() / xs.len() as f64
}

fn near(got: f64, want: f64, tolerance: f64) {
    assert!(
        (got - want).abs() <= want.abs() * tolerance,
        "got {got}, want {want} within {}%",
        tolerance * 100.0
    );
}

const N: usize = 200_000;

#[test]
fn exponential_mean_matches() {
    let xs = draws("{{ dist.exponential(500.0) }}", 1, N);
    near(mean(&xs), 500.0, 0.03);
    // The median of an exponential is mean * ln 2.
    let mut sorted = xs;
    sorted.sort_unstable();
    near(sorted[N / 2] as f64, 500.0 * std::f64::consts::LN_2, 0.03);
}

#[test]
fn pareto_has_its_scale_as_minimum_and_its_mean() {
    let xs = draws("{{ dist.pareto(4.0, 100.0) }}", 2, N);
    assert!(xs.iter().all(|x| *x >= 100));
    // The mean of a Pareto is alpha * scale / (alpha - 1).
    near(mean(&xs), 4.0 * 100.0 / 3.0, 0.03);
    // P(X > x) = (scale / x)^alpha, so a fifth of the draws exceed 100 * 5^(1/4).
    let limit = 100.0 * 5f64.powf(0.25);
    let above = xs.iter().filter(|x| **x as f64 > limit).count();
    near(above as f64 / N as f64, 0.2, 0.05);
}

#[test]
fn lognormal_median_and_mean_match() {
    let xs = draws("{{ dist.lognormal(5.0, 0.5) }}", 3, N);
    let mut sorted = xs.clone();
    sorted.sort_unstable();
    near(sorted[N / 2] as f64, 5f64.exp(), 0.03);
    near(mean(&xs), (5.0f64 + 0.125).exp(), 0.03);
}

#[test]
fn uniform_stays_inside_its_bounds_and_covers_them() {
    let xs = draws("{{ dist.uniform(10, 20) }}", 4, N);
    assert!(xs.iter().all(|x| (10..=20).contains(x)));
    near(mean(&xs), 15.0, 0.01);
    for v in 10..=20 {
        assert!(xs.contains(&v), "{v} never drawn");
    }
    assert!(draws("{{ dist.uniform(7, 7) }}", 4, 10)
        .iter()
        .all(|x| *x == 7));
}

#[test]
fn every_function_returns_an_integer_text() {
    for src in [
        "{{ dist.pareto(1.2, 4096) }}",
        "{{ dist.lognormal(3.0, 0.8) }}",
        "{{ dist.exponential(10) }}",
        "{{ dist.uniform(0, 3) }}",
    ] {
        let t = Template::compile(src).unwrap();
        assert!(!t.is_static(), "{src}");
        assert_eq!(draws(src, 5, 50).len(), 50);
    }
}

#[test]
fn arguments_bind_by_keyword_and_int_binds_float() {
    let a = draws("{{ dist.pareto(alpha=1.5, scale=10, cap=99) }}", 6, 100);
    let b = draws("{{ dist.pareto(1.5, 10.0, 99) }}", 6, 100);
    assert_eq!(a, b);
    let c = draws("{{ dist.exponential(cap=1000, mean=50) }}", 6, 100);
    assert_eq!(c, draws("{{ dist.exponential(50.0, 1000) }}", 6, 100));
}

#[test]
fn cap_limits_the_draw() {
    let xs = draws("{{ dist.pareto(0.5, 1000.0, cap=5000) }}", 7, 5000);
    assert!(xs.iter().all(|x| *x <= 5000));
    assert!(xs.contains(&5000), "a heavy tail should reach the cap");
    let xs = draws("{{ dist.exponential(1000.0, 3) }}", 7, 500);
    assert!(xs.iter().all(|x| *x <= 3));
    let xs = draws("{{ dist.lognormal(20.0, 3.0, cap=77) }}", 7, 500);
    assert!(xs.iter().all(|x| *x <= 77));
}

#[test]
fn the_default_cap_is_two_to_the_31_minus_one() {
    assert_eq!(DEFAULT_CAP, 2_147_483_647);
    // The scale is far above the default cap, so every draw lands on it.
    let xs = draws("{{ dist.pareto(0.1, 1000000000000) }}", 8, 200);
    assert!(xs.iter().all(|x| *x == DEFAULT_CAP));
    let xs = draws("{{ dist.exponential(1000000000000000) }}", 8, 200);
    assert!(xs.iter().all(|x| *x <= DEFAULT_CAP));
    assert!(xs.contains(&DEFAULT_CAP));
}

#[test]
fn a_seed_repeats_the_draws() {
    for src in [
        "{{ dist.pareto(1.2, 4096) }}",
        "{{ dist.lognormal(3.0, 0.8) }}",
        "{{ dist.exponential(10) }}",
        "{{ dist.uniform(0, 1000000) }}",
    ] {
        let a = draws(src, 11, 200);
        assert_eq!(a, draws(src, 11, 200), "{src}");
        assert_ne!(a, draws(src, 12, 200), "{src}");
    }
}

#[test]
fn a_call_draws_a_fixed_number_of_words() {
    // The generator state after a call depends only on the function, so a
    // run stays in step whatever the arguments drew.
    let render = |src: &str| {
        let t = Template::compile(src).unwrap();
        let filler = Filler::new(1);
        let vars = Vars::new();
        let overlay = Vars::new();
        let mut rng = Pcg64Mcg::new(5);
        let mut none = NoFunctions;
        let mut ctx = RenderCtx {
            vars: &vars,
            overlay: &overlay,
            session_id: 1,
            flow: "f",
            last: &[],
            rng: &mut rng,
            filler: &filler,
            functions: &mut none,
        };
        t.render_to_vec(&mut ctx).unwrap();
        rng.next_u64()
    };
    // Pareto, exponential, and uniform draw one word, and lognormal two.
    assert_eq!(
        render("{{ dist.pareto(2.0, 5.0) }}"),
        render("{{ dist.exponential(9.0) }}")
    );
    assert_eq!(
        render("{{ dist.exponential(9.0) }}"),
        render("{{ dist.uniform(1, 2) }}")
    );
    assert_ne!(
        render("{{ dist.lognormal(1.0, 1.0) }}"),
        render("{{ dist.uniform(1, 2) }}")
    );
}

#[test]
fn arguments_may_be_variables_and_calls() {
    let mut vars = Vars::new();
    vars.insert("m".into(), Value::Int(40));
    vars.insert("s".into(), Value::Str("2.5".into()));
    vars.insert("c".into(), Value::Int(60));
    let xs = draws_with("{{ dist.exponential(vars.m, vars.c) }}", 1, 500, &vars).unwrap();
    assert!(xs.iter().all(|x| *x <= 60));
    let xs = draws_with("{{ dist.pareto(vars.s, rand_int(10, 10)) }}", 1, 100, &vars).unwrap();
    assert!(xs.iter().all(|x| *x >= 10));
    // A text variable that parses as a number binds a float parameter.
    assert_eq!(
        draws_with("{{ dist.pareto(vars.s, 3) }}", 1, 50, &vars).unwrap(),
        draws("{{ dist.pareto(2.5, 3) }}", 1, 50)
    );
}

#[test]
fn a_draw_can_size_a_fill() {
    let t = Template::compile("{{ fill(dist.uniform(8, 8)) }}").unwrap();
    let filler = Filler::new(1);
    let (vars, overlay) = (Vars::new(), Vars::new());
    let mut rng = Pcg64Mcg::new(1);
    let mut none = NoFunctions;
    let mut ctx = RenderCtx {
        vars: &vars,
        overlay: &overlay,
        session_id: 1,
        flow: "f",
        last: &[],
        rng: &mut rng,
        filler: &filler,
        functions: &mut none,
    };
    assert_eq!(t.render_to_vec(&mut ctx).unwrap().len(), 8);
}

#[test]
fn compile_rejects_bad_calls() {
    type Want = fn(&TemplateError) -> bool;
    let cases: [(&str, Want); 14] = [
        (
            "{{ dist.zipf(1) }}",
            |e| matches!(e, TemplateError::UnknownFunction(n) if n == "dist.zipf"),
        ),
        (
            "{{ dist.pareto(1.2) }}",
            |e| matches!(e, TemplateError::MissingParameter { param, .. } if param == "scale"),
        ),
        (
            "{{ dist.uniform(1) }}",
            |e| matches!(e, TemplateError::MissingParameter { param, .. } if param == "high"),
        ),
        ("{{ dist.uniform(1, 2, 3) }}", |e| {
            matches!(e, TemplateError::Arity { max: 2, got: 3, .. })
        }),
        ("{{ dist.exponential(1, 2, 3) }}", |e| {
            matches!(e, TemplateError::Arity { max: 2, got: 3, .. })
        }),
        ("{{ dist.exponential(1, cap=2, mean=3) }}", |e| {
            matches!(e, TemplateError::DuplicateParameter { .. })
        }),
        ("{{ dist.exponential(1, cap=2, cap=3) }}", |e| {
            matches!(e, TemplateError::DuplicateParameter { .. })
        }),
        ("{{ dist.exponential(rate=1) }}", |e| {
            matches!(e, TemplateError::UnknownParameter { .. })
        }),
        ("{{ dist.exponential('ten') }}", |e| {
            matches!(e, TemplateError::ArgumentType { .. })
        }),
        (
            "{{ dist.exponential(10, 1.5) }}",
            |e| matches!(e, TemplateError::ArgumentType { param, .. } if param == "cap"),
        ),
        ("{{ dist.pareto }}", |e| {
            matches!(e, TemplateError::UnknownReference(_))
        }),
        ("{{ profiles }}", |e| {
            matches!(e, TemplateError::UnknownReference(_))
        }),
        ("{{ profiles.a.b }}", |e| {
            matches!(e, TemplateError::UnknownReference(_))
        }),
        ("{{ profiles.a(1) }}", |e| {
            matches!(e, TemplateError::ReservedModule(_))
        }),
    ];
    for (src, want) in cases {
        let e = Template::compile(src).expect_err(src);
        assert!(want(&e), "{src}: {e:?}");
    }
}

#[test]
fn compile_checks_the_range_of_literal_arguments() {
    for src in [
        "{{ dist.pareto(0, 1) }}",
        "{{ dist.pareto(-1.5, 1) }}",
        "{{ dist.pareto(1, 0) }}",
        "{{ dist.lognormal(1, 0) }}",
        "{{ dist.lognormal(1, -0.5) }}",
        "{{ dist.exponential(0) }}",
        "{{ dist.exponential(-3) }}",
        "{{ dist.exponential(1, cap=0) }}",
        "{{ dist.pareto(1, 1, -5) }}",
        "{{ dist.uniform(-1, 5) }}",
        "{{ dist.uniform(1, -5) }}",
    ] {
        let e = Template::compile(src).expect_err(src);
        assert!(
            matches!(e, TemplateError::DistParameter { .. }),
            "{src}: {e:?}"
        );
    }
    assert_eq!(
        Template::compile("{{ dist.uniform(5, 1) }}").unwrap_err(),
        TemplateError::UniformRange
    );
    // A negative mu is fine, and zero is a valid lower bound.
    Template::compile("{{ dist.lognormal(-2.0, 1.0) }}").unwrap();
    Template::compile("{{ dist.uniform(0, 0) }}").unwrap();
    let e = Template::compile("{{ dist.exponential(0) }}").unwrap_err();
    assert_eq!(
        e.to_string(),
        "parameter 'mean' of 'dist.exponential' must be positive"
    );
}

#[test]
fn render_checks_the_range_of_computed_arguments() {
    let mut vars = Vars::new();
    vars.insert("neg".into(), Value::Int(-4));
    vars.insert("zero".into(), Value::Int(0));
    vars.insert("word".into(), Value::Str("many".into()));
    let err = |src: &str| draws_with(src, 1, 1, &vars).unwrap_err();
    assert!(matches!(
        err("{{ dist.exponential(vars.neg) }}"),
        TemplateError::DistParameter { .. }
    ));
    assert!(matches!(
        err("{{ dist.exponential(5, vars.zero) }}"),
        TemplateError::DistParameter { .. }
    ));
    assert!(matches!(
        err("{{ dist.uniform(vars.neg, 3) }}"),
        TemplateError::DistParameter { .. }
    ));
    assert_eq!(
        err("{{ dist.uniform(3, vars.zero) }}"),
        TemplateError::UniformRange
    );
    assert!(matches!(
        err("{{ dist.exponential(vars.word) }}"),
        TemplateError::NotNumber(_)
    ));
}

fn chrome() -> Profiles {
    let mut p = Profiles::new();
    p.insert(
        "chrome",
        vec![
            ("User-Agent".into(), Value::Str("Mozilla/5.0 (X11)".into())),
            (
                "Accept-Language".into(),
                Value::Str("en-US,en;q=0.9".into()),
            ),
            ("Upgrade-Insecure-Requests".into(), Value::Int(1)),
            ("DNT".into(), Value::Bool(true)),
            ("Q".into(), Value::Float(0.5)),
        ],
    );
    p.insert("bare", vec![("X-One".into(), Value::Int(1))]);
    p
}

#[test]
fn a_profile_renders_entries_in_declaration_order_with_crlf() {
    let t = Template::compile("GET / HTTP/1.1\r\n{{ profiles.chrome }}\r\n").unwrap();
    assert!(!t.is_static());
    let t = t.fold_profiles(&chrome()).unwrap();
    assert_eq!(
        t.as_static().unwrap(),
        b"GET / HTTP/1.1\r\nUser-Agent: Mozilla/5.0 (X11)\r\nAccept-Language: en-US,en;q=0.9\r\n\
Upgrade-Insecure-Requests: 1\r\nDNT: true\r\nQ: 0.5\r\n\r\n"
    );
    let two = Template::compile("{{ profiles.bare }}{{ profiles.bare }}")
        .unwrap()
        .fold_profiles(&chrome())
        .unwrap();
    assert_eq!(two.as_static().unwrap(), b"X-One: 1\r\nX-One: 1\r\n");
}

#[test]
fn an_unfolded_or_unknown_profile_fails_to_render() {
    let t = Template::compile("{{ profiles.chrome }}").unwrap();
    assert_eq!(
        render_with(&t, &Vars::new(), &Vars::new()).unwrap_err(),
        TemplateError::UndefinedProfile("chrome".into())
    );
    let t = Template::compile("{{ profiles.ghost }}")
        .unwrap()
        .fold_profiles(&chrome())
        .unwrap();
    assert_eq!(
        render_with(&t, &Vars::new(), &Vars::new()).unwrap_err(),
        TemplateError::UndefinedProfile("ghost".into())
    );
}

#[test]
fn a_profile_is_a_text_argument_of_a_module_call() {
    let t = Template::compile("{{ m.f('GET', headers=profiles.bare) }}")
        .unwrap()
        .fold_profiles(&chrome())
        .unwrap();
    let out = render_with(&t, &Vars::new(), &Vars::new()).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("headers=Text(\"X-One: 1\\r\\n\")"), "{out}");
}

#[test]
fn dump_names_dist_calls_and_profiles() {
    let t = Template::compile("{{ profiles.p }}{{ dist.pareto(1.5, vars.s, cap=9) }}").unwrap();
    assert_eq!(
        t.dump(),
        concat!(
            "dynamic, 2 segments\n",
            "[0] profiles.p\n",
            "[1] dist.pareto\n",
            "    alpha = float 1.5\n",
            "    scale = vars.s\n",
            "    cap = int 9\n"
        )
    );
}
